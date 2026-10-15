//! Photometric refinement of a mesh against the photos, without depth maps.
//!
//! The mesh (the silhouette hull, or any surface of the pipeline) is the surface model; the photos only
//! say where it should move. Anchor vertices search along their normal for the offset at which the photos
//! agree: a small patch on the mesh's tangent plane is projected into the most frontal view that sees the
//! vertex and into views 8 to 35 degrees away that see it too, and an offset scores the mean normalised
//! cross-correlation of those patches with the reference patch. Visibility comes from the mesh itself.
//! An offset is accepted when its score is high, clearly above the scores of the other offsets (a
//! distinct peak: plain surfaces agree everywhere and are left alone) and better than staying. Accepted
//! offsets are spread over the mesh as a smooth field, the mesh moves along its normals, and the search
//! repeats with a smaller range.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};

use crate::inputs::{load_views, parallel_map, ViewRow};

/// Offsets tried per anchor and the side of the square patch.
const OFFSETS: usize = 17;
const GRID: usize = 5;

pub struct Options {
    /// Search radius of each round, in mesh edge lengths (the voxel size of a marching-cubes mesh).
    pub rounds: Vec<f64>,
    /// Vertices that search; the others follow through the smooth field.
    pub anchors: usize,
    /// Views per anchor, the reference included.
    pub views: usize,
    /// Least mean correlation of an accepted offset.
    pub minimum_score: f32,
    /// Least excess of the best score over the median score of the offsets.
    pub peak: f32,
    /// Least gain of the best score over staying.
    pub gain: f32,
    /// Averaging steps that spread the accepted offsets over the mesh.
    pub diffusion: usize,
    /// Move inward only (the silhouette hull is an outer bound).
    pub inward_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            rounds: vec![6.0, 3.0, 1.5],
            anchors: 120_000,
            views: 6,
            minimum_score: 0.55,
            peak: 0.15,
            gain: 0.08,
            diffusion: 200,
            inward_only: true,
        }
    }
}

/// One photo as the search reads it: grey, cropped to the mesh's projection, reduced.
struct Photo {
    width: usize,
    height: usize,
    data: Vec<u8>,
    /// Full-resolution pixel of the crop's corner and the reduction factor.
    origin: [f64; 2],
    reduction: f64,
    k: [f64; 4],
    rotation: [[f64; 3]; 3],
    translation: [f64; 3],
    centre: [f64; 3],
}

impl Photo {
    /// Position in the reduced crop (pixel centres at integers) and camera depth.
    #[inline]
    fn project(&self, p: [f64; 3]) -> ([f64; 2], f64) {
        let r = &self.rotation;
        let c = [0, 1, 2].map(|i| r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2] + self.translation[i]);
        let z = c[2];
        let u = (self.k[0] * c[0] / z + self.k[2] - self.origin[0]) / self.reduction - 0.5;
        let v = (self.k[1] * c[1] / z + self.k[3] - self.origin[1]) / self.reduction - 0.5;
        ([u, v], z)
    }

    #[inline]
    fn sample(&self, at: [f64; 2]) -> Option<f32> {
        let (x, y) = (at[0].floor(), at[1].floor());
        if !(x >= 0.0 && y >= 0.0 && x + 1.0 < self.width as f64 && y + 1.0 < self.height as f64) {
            return None;
        }
        let (ix, iy) = (x as usize, y as usize);
        let (u, v) = ((at[0] - x) as f32, (at[1] - y) as f32);
        let row = iy * self.width + ix;
        let value = |offset: usize| f32::from(self.data[row + offset]);
        Some(
            value(0) * (1.0 - u) * (1.0 - v) + value(1) * u * (1.0 - v) + value(self.width) * (1.0 - u) * v + value(self.width + 1) * u * v,
        )
    }
}

fn load_photo(row: &ViewRow, vertices: &[[f64; 3]]) -> Result<Photo> {
    let rotation = row.rotation;
    let translation = row.translation;
    let centre = [0, 1, 2].map(|j| -(rotation[0][j] * translation[0] + rotation[1][j] * translation[1] + rotation[2][j] * translation[2]));
    let image = crate::storage::open_image(&row.image)?.to_luma8();
    let (full_width, full_height) = (image.width() as usize, image.height() as usize);
    // The window the mesh projects into, with a margin.
    let mut window = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in vertices.iter().step_by(17) {
        let c = [0, 1, 2].map(|i| rotation[i][0] * p[0] + rotation[i][1] * p[1] + rotation[i][2] * p[2] + translation[i]);
        if c[2] > 1e-9 {
            let (x, y) = (row.k[0] * c[0] / c[2] + row.k[2], row.k[1] * c[1] / c[2] + row.k[3]);
            window = [window[0].min(x), window[1].min(y), window[2].max(x), window[3].max(y)];
        }
    }
    ensure!(window[0].is_finite(), "the mesh lies behind view {}", row.name);
    let margin = 24.0;
    let x0 = (window[0] - margin).floor().clamp(0.0, full_width as f64) as usize;
    let y0 = (window[1] - margin).floor().clamp(0.0, full_height as f64) as usize;
    let x1 = (window[2] + margin).ceil().clamp(0.0, full_width as f64) as usize;
    let y1 = (window[3] + margin).ceil().clamp(0.0, full_height as f64) as usize;
    ensure!(x1 > x0 + 8 && y1 > y0 + 8, "the mesh does not project into view {}", row.name);
    // Reduced so that the object is about a thousand pixels across at most: finer photos add time, not signal,
    // at the patch sizes used here.
    let reduction = (((x1 - x0).max(y1 - y0)) as f64 / 1100.0).ceil().max(1.0) as usize;
    let (width, height) = ((x1 - x0) / reduction, (y1 - y0) / reduction);
    let raw = image.as_raw();
    let mut data = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            for dy in 0..reduction {
                let line = (y0 + y * reduction + dy) * full_width + x0 + x * reduction;
                sum += raw[line..line + reduction].iter().map(|&v| u32::from(v)).sum::<u32>();
            }
            data[y * width + x] = (sum / (reduction * reduction) as u32) as u8;
        }
    }
    Ok(Photo { width, height, data, origin: [x0 as f64, y0 as f64], reduction: reduction as f64, k: row.k, rotation, translation, centre })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt().max(1e-300);
    a.map(|v| v / n)
}

/// Area-weighted vertex normals, smoothed three times over the neighbours.
fn normals(vertices: &[[f64; 3]], faces: &[[u32; 3]], start: &[usize], neighbour: &[u32]) -> Vec<[f64; 3]> {
    let mut out = vec![[0.0; 3]; vertices.len()];
    for f in faces {
        let (a, b, c) = (vertices[f[0] as usize], vertices[f[1] as usize], vertices[f[2] as usize]);
        let n = cross(sub(b, a), sub(c, a));
        for &v in f {
            for axis in 0..3 {
                out[v as usize][axis] += n[axis];
            }
        }
    }
    out.iter_mut().for_each(|n| *n = unit(*n));
    for _ in 0..3 {
        let previous = out.clone();
        for v in 0..out.len() {
            let mut sum = previous[v];
            for &n in &neighbour[start[v]..start[v + 1]] {
                for axis in 0..3 {
                    sum[axis] += previous[n as usize][axis] / (start[v + 1] - start[v]).max(1) as f64;
                }
            }
            out[v] = unit(sum);
        }
    }
    out
}

/// Per view: whether each anchor faces the view and is the nearest surface there (a point z-buffer of all
/// vertices on cells of four reduced pixels), and the cosine between its normal and the direction to the view.
fn visibility(photo: &Photo, vertices: &[[f64; 3]], anchors: &[u32], anchor_normals: &[[f64; 3]]) -> Vec<f32> {
    const CELL: f64 = 4.0;
    let (gw, gh) = ((photo.width as f64 / CELL) as usize + 2, (photo.height as f64 / CELL) as usize + 2);
    let mut nearest = vec![f32::INFINITY; gw * gh];
    let cell = |at: [f64; 2]| -> Option<usize> {
        let (x, y) = ((at[0] / CELL).floor(), (at[1] / CELL).floor());
        (x >= 1.0 && y >= 1.0 && x < (gw - 1) as f64 && y < (gh - 1) as f64).then(|| y as usize * gw + x as usize)
    };
    for &p in vertices {
        let (at, z) = photo.project(p);
        if z > 0.0 {
            if let Some(c) = cell(at) {
                nearest[c] = nearest[c].min(z as f32);
            }
        }
    }
    // The nearest surface of the neighbourhood: a vertex behind a closer cell next to it is hidden too.
    let spread: Vec<f32> = (0..gw * gh)
        .map(|c| {
            let (x, y) = (c % gw, c / gw);
            let mut least = f32::INFINITY;
            for dy in y.saturating_sub(1)..(y + 2).min(gh) {
                for dx in x.saturating_sub(1)..(x + 2).min(gw) {
                    least = least.min(nearest[dy * gw + dx]);
                }
            }
            least
        })
        .collect();
    anchors
        .iter()
        .zip(anchor_normals)
        .map(|(&a, &normal)| {
            let p = vertices[a as usize];
            let (at, z) = photo.project(p);
            let cosine = dot(unit(sub(photo.centre, p)), normal) as f32;
            match cell(at) {
                Some(c) if z > 0.0 && (z as f32) <= spread[c] * 1.006 && cosine > 0.3 => cosine,
                _ => -1.0,
            }
        })
        .collect()
}

/// What one anchor found: the offset to move by (scene units) and the weight of that wish (0: no say).
#[derive(Clone, Copy, Default)]
struct Wish {
    offset: f64,
    weight: f64,
    accepted: bool,
    usable: bool,
}

#[allow(clippy::too_many_arguments)]
fn search(photos: &[Photo], position: [f64; 3], normal: [f64; 3], cosines: &[f32], radius: f64, options: &Options) -> Wish {
    // The most frontal view that sees the anchor, and the views 8 to 35 degrees from it that see it too.
    let Some(reference) = (0..photos.len()).filter(|&v| cosines[v] > 0.0).max_by(|&a, &b| cosines[a].total_cmp(&cosines[b])) else {
        return Wish::default();
    };
    let towards = unit(sub(photos[reference].centre, position));
    let (near, far) = (8f64.to_radians().cos(), 35f64.to_radians().cos());
    let mut others: Vec<usize> = (0..photos.len())
        .filter(|&v| v != reference && cosines[v] > 0.0)
        .filter(|&v| {
            let apart = dot(unit(sub(photos[v].centre, position)), towards);
            apart < near && apart > far
        })
        .collect();
    others.sort_by(|&a, &b| cosines[b].total_cmp(&cosines[a]).then(a.cmp(&b)));
    others.truncate(options.views.saturating_sub(1));
    if others.len() < 2 {
        return Wish::default();
    }
    let seed = if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let tu = unit(cross(normal, seed));
    let tv = cross(normal, tu);
    let depth = dot(sub(position, photos[reference].centre), photos[reference].rotation[2]);
    let spacing = 1.2 * photos[reference].reduction * depth / photos[reference].k[0];
    let low = -1.0;
    let high = if options.inward_only { 0.25 } else { 1.0 };
    let offsets: [f64; OFFSETS] = std::array::from_fn(|t| (low + (high - low) * t as f64 / (OFFSETS - 1) as f64) * radius);

    // A patch as a zero-mean unit vector; none when it leaves the photo or is flat (deviation below one grey level).
    let patch = |photo: &Photo, offset: f64| -> Option<[f32; GRID * GRID]> {
        let mut values = [0f32; GRID * GRID];
        let base = [0, 1, 2].map(|a| position[a] + offset * normal[a]);
        for (n, value) in values.iter_mut().enumerate() {
            let (gu, gv) = ((n % GRID) as f64 - (GRID / 2) as f64, (n / GRID) as f64 - (GRID / 2) as f64);
            let p = [0, 1, 2].map(|a| base[a] + spacing * (gu * tu[a] + gv * tv[a]));
            *value = photo.sample(photo.project(p).0)?;
        }
        let mean = values.iter().sum::<f32>() / (GRID * GRID) as f32;
        values.iter_mut().for_each(|v| *v -= mean);
        let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm < GRID as f32 {
            return None;
        }
        values.iter_mut().for_each(|v| *v /= norm);
        Some(values)
    };
    let mut score = [f32::NAN; OFFSETS];
    for (t, &offset) in offsets.iter().enumerate() {
        let Some(first) = patch(&photos[reference], offset) else { continue };
        let (mut total, mut count) = (0f32, 0usize);
        for &v in &others {
            if let Some(second) = patch(&photos[v], offset) {
                total += first.iter().zip(&second).map(|(a, b)| a * b).sum::<f32>();
                count += 1;
            }
        }
        if count >= 2 {
            score[t] = total / count as f32;
        }
    }
    let Some(best) = (0..OFFSETS).filter(|&t| score[t].is_finite()).max_by(|&a, &b| score[a].total_cmp(&score[b]).then(b.cmp(&a))) else {
        return Wish { usable: true, ..Wish::default() };
    };
    let top = score[best];
    let stay = (0..OFFSETS).min_by(|&a, &b| offsets[a].abs().total_cmp(&offsets[b].abs())).unwrap();
    let mut finite: Vec<f32> = score.iter().copied().filter(|s| s.is_finite()).collect();
    finite.sort_by(f32::total_cmp);
    let median =
        if finite.len() % 2 == 1 { finite[finite.len() / 2] } else { 0.5 * (finite[finite.len() / 2 - 1] + finite[finite.len() / 2]) };
    let inner = best > 0 && best + 1 < OFFSETS;
    let step = offsets[1] - offsets[0];
    let mut offset = offsets[best];
    if inner && score[best - 1].is_finite() && score[best + 1].is_finite() {
        let curvature = score[best - 1] - 2.0 * top + score[best + 1];
        if curvature < -1e-6 {
            offset += f64::from((0.5 * (score[best - 1] - score[best + 1]) / curvature).clamp(-0.5, 0.5)) * step;
        }
    }
    let good = top >= options.minimum_score;
    let better = !score[stay].is_finite() || top - score[stay] >= options.gain || best == stay;
    if good && inner && top - median >= options.peak && better {
        return Wish { offset, weight: f64::from((top - options.minimum_score).clamp(0.02, 1.0)), accepted: true, usable: true };
    }
    if good && best.abs_diff(stay) <= 1 {
        // The photos agree where the surface already is: a say for staying.
        return Wish { offset: 0.0, weight: 0.3, accepted: false, usable: true };
    }
    Wish { usable: true, ..Wish::default() }
}

/// Refines `vertices` in place against the photos of `rows`. Returns counts per round.
pub fn refine(
    vertices: &mut [[f64; 3]],
    faces: &[[u32; 3]],
    rows: &[ViewRow],
    options: &Options,
    check: &dyn Fn() -> Result<()>,
) -> Result<Value> {
    ensure!(!vertices.is_empty() && !faces.is_empty(), "the mesh is empty");
    ensure!(rows.len() >= 3, "refinement needs at least three views");
    let started = std::time::Instant::now();
    let (start, neighbour) = crate::mesh::surface::adjacency(vertices.len(), faces);
    let mut lengths: Vec<f64> = faces
        .iter()
        .step_by((faces.len() / 20_000).max(1))
        .map(|f| dot(sub(vertices[f[0] as usize], vertices[f[1] as usize]), sub(vertices[f[0] as usize], vertices[f[1] as usize])).sqrt())
        .collect();
    let edge = crate::inputs::median_f64(&mut lengths);
    ensure!(edge.is_finite() && edge > 0.0, "the mesh has no extent");
    let photos: Vec<Photo> = parallel_map(rows.len(), |n| load_photo(&rows[n], vertices)).into_iter().collect::<Result<_>>()?;
    check()?;
    let loaded = started.elapsed().as_secs_f64();
    // Anchors: a fixed pseudo-random subset, the same for every run.
    let mut order: Vec<u32> = (0..vertices.len() as u32).collect();
    order.sort_by_key(|&i| (i.wrapping_mul(2_654_435_761), i));
    order.truncate(options.anchors.min(vertices.len()));
    order.sort_unstable();
    let anchors = order;
    let mut rounds = Vec::new();
    for &radius_edges in &options.rounds {
        check()?;
        let round_started = std::time::Instant::now();
        let normal = normals(vertices, faces, &start, &neighbour);
        let anchor_normals: Vec<[f64; 3]> = anchors.iter().map(|&a| normal[a as usize]).collect();
        // [view][anchor]
        let seen: Vec<Vec<f32>> = parallel_map(photos.len(), |v| visibility(&photos[v], vertices, &anchors, &anchor_normals));
        check()?;
        let radius = radius_edges * edge;
        const CHUNK: usize = 2048;
        let wishes: Vec<Wish> = parallel_map(anchors.len().div_ceil(CHUNK), |chunk| {
            (chunk * CHUNK..((chunk + 1) * CHUNK).min(anchors.len()))
                .map(|i| {
                    let cosines: Vec<f32> = seen.iter().map(|view| view[i]).collect();
                    search(&photos, vertices[anchors[i] as usize], anchor_normals[i], &cosines, radius, options)
                })
                .collect::<Vec<Wish>>()
        })
        .into_iter()
        .flatten()
        .collect();
        check()?;
        // The wishes as a smooth field: weighted averaging over the mesh.
        let mut moved = vec![0.0f64; vertices.len()];
        let mut weight = vec![0.0f64; vertices.len()];
        for (&a, wish) in anchors.iter().zip(&wishes) {
            moved[a as usize] = wish.offset * wish.weight;
            weight[a as usize] = wish.weight;
        }
        let average = |values: &mut Vec<f64>| {
            let previous = values.clone();
            for v in 0..values.len() {
                let row = &neighbour[start[v]..start[v + 1]];
                let mean = row.iter().map(|&n| previous[n as usize]).sum::<f64>() / row.len().max(1) as f64;
                values[v] = 0.5 * previous[v] + 0.5 * mean;
            }
        };
        for _ in 0..options.diffusion {
            average(&mut moved);
            average(&mut weight);
        }
        let mut shifted = 0usize;
        for v in 0..vertices.len() {
            let mut shift = if weight[v] > 1e-4 { moved[v] / weight[v] } else { 0.0 };
            if options.inward_only {
                shift = shift.min(0.0);
            }
            shifted += usize::from(shift.abs() > 0.1 * edge);
            for axis in 0..3 {
                vertices[v][axis] += shift * normal[v][axis];
            }
        }
        crate::mesh::surface::taubin(vertices, faces, 2, crate::mesh::TAUBIN_LAMBDA, crate::mesh::TAUBIN_MU);
        let count = |test: &dyn Fn(&Wish) -> bool| wishes.iter().filter(|w| test(w)).count() as f64 / wishes.len().max(1) as f64;
        rounds.push(json!({
            "radius_edges": radius_edges, "anchors_usable": count(&|w| w.usable), "accepted": count(&|w| w.accepted),
            "confirmed_in_place": count(&|w| !w.accepted && w.weight > 0.0), "vertices_moved": shifted as f64 / vertices.len() as f64,
            "seconds": round_started.elapsed().as_secs_f64(),
        }));
    }
    Ok(json!({
        "vertices": vertices.len(), "anchors": anchors.len(), "views": photos.len(), "edge": edge,
        "photo_reduction": photos.iter().map(|p| p.reduction).fold(0.0, f64::max), "load_seconds": loaded, "rounds": rounds,
        "seconds": started.elapsed().as_secs_f64(),
    }))
}

/// Vertices that share coordinates become one: an STL holds every triangle's corners separately.
pub fn weld(triangles: &[crate::stl::Triangle]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut index = std::collections::HashMap::with_capacity(triangles.len());
    let mut vertices = Vec::new();
    let mut faces = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        let face = triangle.map(|corner| {
            *index.entry(corner.map(f32::to_bits)).or_insert_with(|| {
                vertices.push(corner.map(f64::from));
                (vertices.len() - 1) as u32
            })
        });
        if face[0] != face[1] && face[1] != face[2] && face[0] != face[2] {
            faces.push(face);
        }
    }
    (vertices, faces)
}

pub fn main(arguments: &[String]) -> Result<()> {
    const USAGE: &str =
        "usage: crispmesh-dense refine --inputs DIR --mesh STL --output STL [--peak X] [--rounds A,B,C] [--anchors N] [--outward]

Refines a mesh of the scene in DIR (cameras.json, images) against its photos and writes the result.";
    let mut values = std::collections::BTreeMap::new();
    let mut options = Options::default();
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        if flag == "--outward" {
            options.inward_only = false;
            continue;
        }
        if !["--inputs", "--mesh", "--output", "--peak", "--rounds", "--anchors"].contains(&flag.as_str()) {
            bail!("unknown argument {flag}\n{USAGE}");
        }
        values.insert(flag.clone(), rest.next().ok_or_else(|| anyhow!("{flag} needs a value"))?.clone());
    }
    let need = |name: &str| values.get(name).map(PathBuf::from).ok_or_else(|| anyhow!("{name} is required\n{USAGE}"));
    let (inputs, mesh, output) = (need("--inputs")?, need("--mesh")?, need("--output")?);
    if let Some(peak) = values.get("--peak") {
        options.peak = peak.parse().map_err(|_| anyhow!("--peak needs a number"))?;
    }
    if let Some(rounds) = values.get("--rounds") {
        options.rounds =
            rounds.split(',').map(|r| r.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| anyhow!("--rounds needs numbers"))?;
    }
    if let Some(anchors) = values.get("--anchors") {
        options.anchors = anchors.parse().map_err(|_| anyhow!("--anchors needs a number"))?;
    }
    let report = run(&inputs, &mesh, &output, &options, &|| Ok(()))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Refines `directory/mesh.stl` in place: the mesh before refinement is kept as `mesh-unrefined.stl`, and
/// `refine.json` records what happened.
pub fn run_in_place(inputs: &Path, directory: &Path, options: &Options, check: &dyn Fn() -> Result<()>) -> Result<Value> {
    let (mesh, refined, kept) = (directory.join("mesh.stl"), directory.join("mesh-refined.stl"), directory.join("mesh-unrefined.stl"));
    let report = run(inputs, &mesh, &refined, options, check)?;
    crate::storage::rename(&mesh, &kept)?;
    crate::storage::rename(&refined, &mesh)?;
    crate::storage::write(directory.join("refine.json"), serde_json::to_string_pretty(&report)? + "\n")?;
    Ok(report)
}

/// Reads `mesh`, refines it against the photos of the scene in `inputs` and writes `output` (which must not exist).
pub fn run(inputs: &Path, mesh: &Path, output: &Path, options: &Options, check: &dyn Fn() -> Result<()>) -> Result<Value> {
    let rows = load_views(inputs)?;
    let (triangles, _) = crate::stl::read_binary(mesh)?;
    let (mut vertices, faces) = weld(&triangles);
    drop(triangles);
    let mut report = refine(&mut vertices, &faces, &rows, options, check).with_context(|| format!("refining {}", mesh.display()))?;
    crate::stl::write_binary(output, "crispmesh refined mesh", &vertices, &faces)?;
    let info = crate::mesh::surface::topology(&vertices, &faces);
    report["triangles"] = json!(info.triangles);
    report["closed"] = json!(info.boundary_edges == 0 && info.nonmanifold_edges == 0);
    report["genus"] = json!(info.genus);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An icosphere of the given radius, subdivided `levels` times.
    fn sphere(radius: f64, levels: usize) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
        let t = (1.0 + 5f64.sqrt()) / 2.0;
        let mut vertices: Vec<[f64; 3]> = [
            [-1.0, t, 0.0],
            [1.0, t, 0.0],
            [-1.0, -t, 0.0],
            [1.0, -t, 0.0],
            [0.0, -1.0, t],
            [0.0, 1.0, t],
            [0.0, -1.0, -t],
            [0.0, 1.0, -t],
            [t, 0.0, -1.0],
            [t, 0.0, 1.0],
            [-t, 0.0, -1.0],
            [-t, 0.0, 1.0],
        ]
        .iter()
        .map(|&v| unit(v))
        .collect();
        let mut faces: Vec<[u32; 3]> = vec![
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        for _ in 0..levels {
            let mut middle = std::collections::HashMap::new();
            let mut next = Vec::with_capacity(faces.len() * 4);
            for f in &faces {
                let mut m = [0u32; 3];
                for e in 0..3 {
                    let (a, b) = (f[e], f[(e + 1) % 3]);
                    m[e] = *middle.entry((a.min(b), a.max(b))).or_insert_with(|| {
                        let (p, q) = (vertices[a as usize], vertices[b as usize]);
                        vertices.push(unit([p[0] + q[0], p[1] + q[1], p[2] + q[2]]));
                        (vertices.len() - 1) as u32
                    });
                }
                next.extend([[f[0], m[0], m[2]], [f[1], m[1], m[0]], [f[2], m[2], m[1]], m]);
            }
            faces = next;
        }
        vertices.iter_mut().for_each(|v| *v = v.map(|c| c * radius));
        (vertices, faces)
    }

    /// A sphere mesh four per cent too large is pulled towards the textured unit sphere of the photos
    /// where they see it, and not pushed outward.
    #[test]
    fn an_inflated_sphere_moves_towards_the_photographed_one() {
        let root = crate::stereo::synthetic::temporary("refine", 36, 256).unwrap();
        let rows = load_views(&root.join("inputs")).unwrap();
        let (mut vertices, faces) = sphere(1.04, 5);
        let options = Options { anchors: 20_000, diffusion: 30, ..Options::default() };
        let report = refine(&mut vertices, &faces, &rows, &options, &|| Ok(())).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let radius = |v: &[f64; 3]| dot(*v, *v).sqrt();
        // The ring of cameras sees the band around the equator of its axis best; judge the vertices that moved.
        let moved: Vec<f64> = vertices.iter().map(radius).filter(|r| *r < 1.035).collect();
        assert!(moved.len() * 4 > vertices.len(), "{} of {} moved, {report}", moved.len(), vertices.len());
        let mean = moved.iter().sum::<f64>() / moved.len() as f64;
        assert!((mean - 1.0).abs() < 0.012, "mean radius of the moved vertices {mean}, {report}");
        // Inward only; the smoothing between rounds may lift vertices of this coarse mesh a little.
        assert!(vertices.iter().all(|v| radius(v) < 1.06));
    }

    #[test]
    fn welding_joins_shared_corners() {
        let triangles = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]];
        let (vertices, faces) = weld(&triangles);
        assert_eq!((vertices.len(), faces.len()), (4, 2));
    }
}
