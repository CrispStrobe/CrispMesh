//! Photo-projected UV atlas and self-contained GLB. Uses recovered cameras only.
//! First implementation: discrete view selection, no seam blending or inferred PBR.
use crate::{
    inputs::{load_views, ViewRow},
    render::{Rgb, ViewCamera},
    stl::Triangle,
};
use anyhow::{bail, ensure, Result};
use serde_json::{json, Value};
use std::{collections::HashMap, path::Path};

fn camera(r: &ViewRow) -> ViewCamera {
    ViewCamera { rotation: r.rotation, translation: r.translation, k: r.k }
}
fn project(c: &ViewCamera, p: [f32; 3]) -> [f64; 3] {
    let q = c.to_camera(p.map(f64::from));
    [q[0] / q[2] * c.k[0] + c.k[2] - 0.5, q[1] / q[2] * c.k[1] + c.k[3] - 0.5, q[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn subtract(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] - b[i])
}
fn length(a: [f32; 3]) -> f32 {
    a.iter().map(|v| v * v).sum::<f32>().sqrt()
}
fn face_normal(t: &Triangle) -> [f32; 3] {
    cross(subtract(t[1], t[0]), subtract(t[2], t[0]))
}

/// Perspective-correct mesh depth, including back faces as occluders.
fn zbuffer(projected: &[[[f64; 3]; 3]], width: usize, height: usize) -> Vec<f64> {
    let mut buffer = vec![f64::INFINITY; width * height];
    for q in projected {
        if q.iter().any(|p| p[2] <= 0.0 || !p.iter().all(|v| v.is_finite())) {
            continue;
        }
        let [a, b, c] = *q;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-12 {
            continue;
        }
        let lo = [a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize, a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize];
        let hi = [a[0].max(b[0]).max(c[0]).ceil().min(width as f64 - 1.0), a[1].max(b[1]).max(c[1]).ceil().min(height as f64 - 1.0)];
        if hi[0] < 0.0 || hi[1] < 0.0 {
            continue;
        }
        for y in lo[1]..=hi[1] as usize {
            for x in lo[0]..=hi[0] as usize {
                let (xv, yv) = (x as f64, y as f64);
                let u = ((b[0] - xv) * (c[1] - yv) - (b[1] - yv) * (c[0] - xv)) / area;
                let v = ((c[0] - xv) * (a[1] - yv) - (c[1] - yv) * (a[0] - xv)) / area;
                let w = 1.0 - u - v;
                if u >= -1e-6 && v >= -1e-6 && w >= -1e-6 {
                    let z = 1.0 / (u / a[2] + v / b[2] + w / c[2]);
                    let n = y * width + x;
                    buffer[n] = buffer[n].min(z);
                }
            }
        }
    }
    buffer
}

/// A smoothed normal field is used ONLY to choose texture photos. Positions,
/// faces and exported surface normals retain the original mesh.
fn selection_normals(triangles: &[Triangle], normals: &[[f32; 3]], control: &crate::control::Control) -> Result<Vec<[f32; 3]>> {
    let mut ids = HashMap::new();
    let mut next = 0usize;
    let faces: Vec<[usize; 3]> = triangles
        .iter()
        .map(|t| {
            t.map(|p| {
                *ids.entry(p.map(f32::to_bits)).or_insert_with(|| {
                    let n = next;
                    next += 1;
                    n
                })
            })
        })
        .collect();
    let unit = |n: [f32; 3]| n.map(|v| v / length(n).max(1e-20));
    let mut field: Vec<_> = normals.iter().copied().map(unit).collect();
    for _ in 0..16 {
        control.check()?;
        let mut vertices = vec![[0.0; 3]; next];
        for (face, n) in faces.iter().zip(&field) {
            for &v in face {
                for a in 0..3 {
                    vertices[v][a] += n[a];
                }
            }
        }
        for n in &mut vertices {
            *n = unit(*n)
        }
        for (face, n) in faces.iter().zip(&mut field) {
            *n = unit([0, 1, 2].map(|a| face.iter().map(|&v| vertices[v][a]).sum()));
        }
    }
    Ok(field)
}

/// Shared-edge adjacency; non-manifold edges and sharp creases do not couple
/// source-image selection. This changes UV choices only, never the mesh.
fn adjacency(triangles: &[Triangle], normals: &[[f32; 3]]) -> Vec<[usize; 3]> {
    let mut vertices = HashMap::new();
    let mut next = 0u32;
    let mut edges = Vec::with_capacity(triangles.len() * 3);
    for (face, t) in triangles.iter().enumerate() {
        let ids = t.map(|p| {
            *vertices.entry(p.map(f32::to_bits)).or_insert_with(|| {
                let v = next;
                next += 1;
                v
            })
        });
        for e in 0..3 {
            let (a, b) = (ids[e].min(ids[(e + 1) % 3]), ids[e].max(ids[(e + 1) % 3]));
            edges.push((((a as u64) << 32) | b as u64, face, e));
        }
    }
    drop(vertices);
    edges.sort_unstable_by_key(|e| e.0);
    let mut out = vec![[usize::MAX; 3]; triangles.len()];
    let mut start = 0;
    while start < edges.len() {
        let mut end = start + 1;
        while end < edges.len() && edges[end].0 == edges[start].0 {
            end += 1;
        }
        if end - start == 2 {
            let (_, a, ae) = edges[start];
            let (_, b, be) = edges[start + 1];
            let dot: f32 = normals[a].iter().zip(normals[b]).map(|(a, b)| a * b).sum();
            if dot > 0.8 * length(normals[a]) * length(normals[b]) {
                out[a][ae] = b;
                out[b][be] = a;
            }
        }
        start = end;
    }
    out
}

fn seam_edges(chosen: &[usize], adjacency: &[[usize; 3]], count: usize) -> usize {
    adjacency
        .iter()
        .enumerate()
        .map(|(i, ns)| {
            ns.iter().filter(|&&j| j != usize::MAX && j > i && chosen[i] < count && chosen[j] < count && chosen[i] != chosen[j]).count()
        })
        .sum()
}

/// Bounded local label optimization among three independently visible views.
/// Monotonic coordinate updates lower a fixed data + shared-edge seam energy.
fn coherent_choices(chosen: &mut [usize], candidates: &[[(u32, f32); 3]], adjacent: &[[usize; 3]], count: usize) {
    for _ in 0..5 {
        let mut changed = 0;
        for i in 0..chosen.len() {
            if chosen[i] >= count {
                continue;
            }
            let best = candidates[i][0].1.max(1e-20);
            let energy = |slot: usize, score: f32, labels: &[usize]| {
                let mut e = 1.0 - score / best;
                for &j in &adjacent[i] {
                    if j != usize::MAX && labels[j] < count && labels[j] != slot {
                        e += 0.12;
                    }
                }
                e
            };
            let current = candidates[i].iter().find(|c| c.0 as usize == chosen[i]).map(|c| c.1).unwrap_or(0.0);
            let mut cost = energy(chosen[i], current, chosen);
            let mut label = chosen[i];
            for &(slot, score) in &candidates[i] {
                if slot as usize >= count || score <= 0.0 {
                    continue;
                }
                let e = energy(slot as usize, score, chosen);
                if e + 1e-6 < cost {
                    cost = e;
                    label = slot as usize;
                }
            }
            if label != chosen[i] {
                chosen[i] = label;
                changed += 1;
            }
        }
        if changed == 0 {
            break;
        }
    }
}

/// Scalar exposure correction from robust log-luminance ratios on the SAME
/// visible surface samples. Bounded gains retain global brightness and hue;
/// this does not estimate intrinsic albedo or remove spatially varying light.
fn exposure_gains(samples: &[Vec<Option<f32>>]) -> Vec<f32> {
    let n = samples.len();
    let mut equations = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let mut ratios: Vec<f32> = samples[i]
                .iter()
                .zip(&samples[j])
                .filter_map(|(a, b)| match (a, b) {
                    (Some(a), Some(b)) if *a > 0.05 && *b > 0.05 && *a < 0.95 && *b < 0.95 => Some((b / a).ln()),
                    _ => None,
                })
                .collect();
            if ratios.len() < 24 {
                continue;
            }
            ratios.sort_by(f32::total_cmp);
            let ratio = ratios[ratios.len() / 2];
            let spread = ratios[ratios.len() * 3 / 4] - ratios[ratios.len() / 4];
            if spread < 0.25 {
                equations.push((i, j, ratio, ratios.len().min(256) as f32));
            }
        }
    }
    let mut gains = vec![0.0f32; n];
    for _ in 0..40 {
        for i in 0..n {
            let (mut total, mut weight) = (0.0, 0.0);
            for &(a, b, r, w) in &equations {
                if a == i {
                    total += w * (gains[b] + r);
                    weight += w;
                }
                if b == i {
                    total += w * (gains[a] - r);
                    weight += w;
                }
            }
            if weight > 0.0 {
                gains[i] = total / weight;
            }
        }
        // Fix each disconnected overlap component's gauge independently.
        let mut visited = vec![false; n];
        for first in 0..n {
            if visited[first] {
                continue;
            }
            let mut component = vec![first];
            visited[first] = true;
            let mut at = 0;
            while at < component.len() {
                let i = component[at];
                at += 1;
                for &(a, b, _, _) in &equations {
                    let j = if a == i {
                        b
                    } else if b == i {
                        a
                    } else {
                        continue;
                    };
                    if !visited[j] {
                        visited[j] = true;
                        component.push(j);
                    }
                }
            }
            let mean = component.iter().map(|&i| gains[i]).sum::<f32>() / component.len() as f32;
            for i in component {
                gains[i] -= mean;
            }
        }
    }
    gains.into_iter().map(|v| v.exp().clamp(0.8, 1.25)).collect()
}

struct Tile {
    row: ViewRow,
    bounds: [usize; 4],
    size: [usize; 2],
}
fn uv(tile: &Tile, p: [f32; 3], slot: usize, side: usize, columns: usize, rows: usize) -> [f32; 2] {
    let q = project(&camera(&tile.row), p);
    let [x, y, w, h] = tile.bounds;
    let scale = (side - 8) as f64 / (w.max(h)) as f64;
    let (tw, th) = ((w as f64 * scale).round() as usize, (h as f64 * scale).round() as usize);
    let left = (side - tw) / 2;
    let top = (side - th) / 2;
    [
        ((slot % columns * side + left) as f64 + (q[0] + 0.5 - x as f64) * tw as f64 / w as f64) / (columns * side) as f64,
        ((slot / columns * side + top) as f64 + (q[1] + 0.5 - y as f64) * th as f64 / h as f64) / (rows * side) as f64,
    ]
    .map(|v| v as f32)
}

/// Textures an existing pipeline STL, preserving coordinates and triangle order.
/// `views` is an evenly spaced subset in input order; all photos remain local.
pub fn run(inputs: &Path, mesh: &Path, output: &Path, views: usize, tile_size: usize) -> Result<Value> {
    run_with(inputs, mesh, output, views, tile_size, &crate::control::Control::none())
}

pub fn run_with(
    inputs: &Path,
    mesh: &Path,
    output: &Path,
    views: usize,
    tile_size: usize,
    control: &crate::control::Control,
) -> Result<Value> {
    run_with_options(inputs, mesh, output, views, tile_size, control, false, false)
}

#[allow(clippy::too_many_arguments)]
pub fn run_with_options(
    inputs: &Path,
    mesh: &Path,
    output: &Path,
    views: usize,
    tile_size: usize,
    control: &crate::control::Control,
    coherent: bool,
    balance: bool,
) -> Result<Value> {
    control.check()?;
    ensure!((4..=64).contains(&views), "views must be in 4..64");
    ensure!((128..=2048).contains(&tile_size), "tile size must be in 128..2048");
    ensure!(!crate::storage::exists(output), "output already exists");
    let mut attribution_path = inputs.join("ATTRIBUTION.txt");
    if !crate::storage::exists(&attribution_path) && output.file_name().is_some_and(|name| name == "texture") {
        attribution_path = output.parent().unwrap_or(inputs).join("ATTRIBUTION.txt");
    }
    let attribution = if crate::storage::exists(&attribution_path) {
        let bytes = crate::storage::read(&attribution_path)?;
        ensure!(bytes.len() <= 65536, "attribution exceeds 64 KiB");
        Some(String::from_utf8(bytes)?)
    } else {
        None
    };
    let start = web_time::Instant::now();
    let source = load_views(inputs)?;
    ensure!(!source.is_empty(), "no recovered views");
    let count = views.min(source.len());
    let columns = (count as f64 + 1.0).sqrt().ceil() as usize;
    let rows = (count + 1).div_ceil(columns);
    ensure!(columns * tile_size <= 8192 && rows * tile_size <= 8192, "atlas exceeds 8192 pixels; reduce views or tile size");
    let (triangles, _) = crate::stl::read_binary(mesh)?;
    ensure!(!triangles.is_empty() && triangles.iter().flatten().flatten().all(|p| p.is_finite()), "mesh must contain finite triangles");
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for p in triangles.iter().flatten() {
        for a in 0..3 {
            low[a] = low[a].min(p[a]);
            high[a] = high[a].max(p[a]);
        }
    }
    let diagonal = length(subtract(high, low)) as f64;
    ensure!(diagonal > 0.0, "mesh has zero extent");
    let mut atlas = Rgb::filled(columns * tile_size, rows * tile_size, [150, 150, 150]);
    let centres: Vec<[f32; 3]> = triangles.iter().map(|t| [0, 1, 2].map(|a| (t[0][a] + t[1][a] + t[2][a]) / 3.0)).collect();
    let normals: Vec<_> = triangles.iter().map(face_normal).collect();
    let selection = selection_normals(&triangles, &normals, control)?;
    let mut best = vec![0.0f64; triangles.len()];
    let mut chosen = vec![count; triangles.len()];
    let mut tiles = Vec::new();
    let mut candidates = if coherent { vec![[(u32::MAX, 0.0f32); 3]; triangles.len()] } else { Vec::new() };
    let sample_step = triangles.len().div_ceil(4000).max(1);
    let mut samples: Vec<Vec<Option<f32>>> = Vec::new();
    for slot in 0..count {
        control.check()?;
        let row = source[slot * source.len() / count].clone();
        let c = camera(&row);
        let photo = Rgb::open(Path::new(row.texture_image.as_deref().unwrap_or(&row.image)))?;
        let mask = crate::scene::read_gray(Path::new(&row.mask))?;
        ensure!((photo.width, photo.height) == (mask.width, mask.height), "image/mask dimensions differ in {}", row.name);
        let (mut x0, mut y0, mut x1, mut y1) = (photo.width, photo.height, 0, 0);
        for y in 0..photo.height {
            for x in 0..photo.width {
                if mask.at(x, y) > 127 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        ensure!(x0 <= x1 && y0 <= y1, "empty mask in {}", row.name);
        let bounds = [x0, y0, x1 - x0 + 1, y1 - y0 + 1];
        let crop = image::RgbImage::from_raw(photo.width as u32, photo.height as u32, photo.data.clone()).unwrap();
        let crop = image::imageops::crop_imm(&crop, x0 as u32, y0 as u32, bounds[2] as u32, bounds[3] as u32).to_image();
        let scale = (tile_size - 8) as f64 / bounds[2].max(bounds[3]) as f64;
        let (tw, th) = ((bounds[2] as f64 * scale).round() as usize, (bounds[3] as f64 * scale).round() as usize);
        let resized = image::imageops::resize(&crop, tw as u32, th as u32, image::imageops::FilterType::Lanczos3);
        let dx = slot % columns * tile_size + (tile_size - tw) / 2;
        let dy = slot / columns * tile_size + (tile_size - th) / 2;
        for y in 0..th {
            for x in 0..tw {
                atlas.set(dx + x, dy + y, resized.get_pixel(x as u32, y as u32).0);
            }
        }
        let vis_scale = 2048.0 / photo.width.max(photo.height) as f64;
        let (vw, vh) = ((photo.width as f64 * vis_scale).ceil() as usize, (photo.height as f64 * vis_scale).ceil() as usize);
        let projected: Vec<_> = triangles
            .iter()
            .map(|t| {
                t.map(|p| {
                    let q = project(&c, p);
                    [(q[0] + 0.5) * vis_scale - 0.5, (q[1] + 0.5) * vis_scale - 0.5, q[2]]
                })
            })
            .collect();
        let depth = zbuffer(&projected, vw, vh);
        let mut view_samples = if balance { vec![None; triangles.len().div_ceil(sample_step)] } else { Vec::new() };
        for (n, t) in triangles.iter().enumerate() {
            let q = project(&c, centres[n]);
            if q[2] <= 0.0 {
                continue;
            }
            let norm = selection[n];
            let norm_len = length(norm) as f64;
            if norm_len == 0.0 {
                continue;
            }
            let cam_norm = [0, 1, 2].map(|a| c.rotation[a].iter().zip(norm).map(|(a, b)| a * b as f64).sum::<f64>());
            let ray = c.to_camera(centres[n].map(f64::from));
            let distance = ray.iter().map(|v| v * v).sum::<f64>().sqrt();
            let cosine = -cam_norm.iter().zip(ray).map(|(a, b)| a * b).sum::<f64>() / (norm_len * distance);
            let score = cosine * cosine * c.k[0] * c.k[1] / (q[2] * q[2]);
            if cosine < 0.15 || (!coherent && !balance && score <= best[n]) {
                continue;
            }
            let valid = t.iter().copied().chain(std::iter::once(centres[n])).all(|p| {
                let q = project(&c, p);
                let (x, y) = (q[0].round(), q[1].round());
                if q[2] <= 0.0
                    || x < 0.0
                    || y < 0.0
                    || x >= photo.width as f64
                    || y >= photo.height as f64
                    || mask.at(x as usize, y as usize) <= 127
                {
                    return false;
                }
                let (vx, vy) = (((q[0] + 0.5) * vis_scale).floor() as usize, ((q[1] + 0.5) * vis_scale).floor() as usize);
                if vx >= vw || vy >= vh {
                    return false;
                }
                // At a silhouette vertex the rounded pixel may lie just outside the
                // rasterised surface. Use its immediate footprint, conservatively
                // choosing the nearest occluder rather than accepting background.
                let mut nearest = depth[vy * vw + vx];
                if !nearest.is_finite() {
                    for yy in vy.saturating_sub(1)..=(vy + 1).min(vh - 1) {
                        for xx in vx.saturating_sub(1)..=(vx + 1).min(vw - 1) {
                            nearest = nearest.min(depth[yy * vw + xx]);
                        }
                    }
                }
                nearest.is_finite() && (q[2] - nearest).abs() < diagonal * 0.002
            });
            if valid {
                if coherent {
                    let mut candidate = (slot as u32, score as f32);
                    for existing in &mut candidates[n] {
                        if candidate.1 > existing.1 {
                            std::mem::swap(existing, &mut candidate);
                        }
                    }
                }
                if balance && n % sample_step == 0 {
                    let (x, y) = (q[0].round() as usize, q[1].round() as usize);
                    let rgb = &photo.data[3 * (y * photo.width + x)..3 * (y * photo.width + x) + 3];
                    view_samples[n / sample_step] =
                        Some((0.2126 * rgb[0] as f32 + 0.7152 * rgb[1] as f32 + 0.0722 * rgb[2] as f32) / 255.0);
                }
                if score > best[n] {
                    best[n] = score;
                    chosen[n] = slot;
                }
            }
        }
        control.log(format!("texture: view {} of {count}", slot + 1));
        tiles.push(Tile { row, bounds, size: [photo.width, photo.height] });
        if balance {
            samples.push(view_samples);
        }
    }
    let mut seam_report = Value::Null;
    if coherent {
        control.check()?;
        let adjacent = adjacency(&triangles, &normals);
        let before = seam_edges(&chosen, &adjacent, count);
        coherent_choices(&mut chosen, &candidates, &adjacent, count);
        let after = seam_edges(&chosen, &adjacent, count);
        seam_report = json!({"shared_edges_before":before,"shared_edges_after":after,"limits":"Only visible top-three candidates; sharp creases uncoupled; not a perceptual seam score"});
    }
    drop(candidates);
    let gains = if balance { exposure_gains(&samples) } else { vec![1.0; count] };
    drop(samples);
    if balance {
        for (slot, &gain) in gains.iter().enumerate() {
            control.check()?;
            let (left, top) = (slot % columns * tile_size, slot / columns * tile_size);
            for y in top..top + tile_size {
                for x in left..left + tile_size {
                    let at = 3 * (y * atlas.width + x);
                    for a in 0..3 {
                        atlas.data[at + a] = (atlas.data[at + a] as f32 * gain).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    let total_area: f64 = normals.iter().map(|v| length(*v) as f64).sum();
    let untextured_area: f64 = normals.iter().zip(&chosen).filter(|(_, s)| **s == count).map(|(v, _)| length(*v) as f64).sum();
    let mut sums: HashMap<[u32; 3], [f32; 3]> = HashMap::new();
    for (t, n) in triangles.iter().zip(&normals) {
        for p in t {
            let entry = sums.entry(p.map(f32::to_bits)).or_insert([0.0; 3]);
            for a in 0..3 {
                entry[a] += n[a];
            }
        }
    }
    let (mut positions, mut smooth, mut uvs, mut indices) = (Vec::<[f32; 3]>::new(), Vec::new(), Vec::new(), Vec::new());
    let mut indexed = HashMap::new();
    for (t, &slot) in triangles.iter().zip(&chosen) {
        for &p in t {
            let key = (p.map(f32::to_bits), slot);
            let index = *indexed.entry(key).or_insert_with(|| {
                let index = positions.len() as u32;
                positions.push(p);
                let n = sums[&p.map(f32::to_bits)];
                smooth.push(n.map(|v| v / length(n).max(1e-20)));
                uvs.push(if slot < count {
                    uv(&tiles[slot], p, slot, tile_size, columns, rows)
                } else {
                    [
                        (slot % columns) as f32 / columns as f32 + 0.5 / columns as f32,
                        (slot / columns) as f32 / rows as f32 + 0.5 / rows as f32,
                    ]
                });
                index
            });
            indices.push(index);
        }
    }
    control.check()?;
    crate::storage::create_dir_all(output)?;
    atlas.save(&output.join("atlas.png"))?;
    let png = crate::storage::read(output.join("atlas.png"))?;
    let bytes = glb(&positions, &smooth, &uvs, &indices, &png, attribution.as_deref())?;
    if let Some(text) = &attribution {
        crate::storage::write_new(output.join("ATTRIBUTION.txt"), text.as_bytes().to_vec())?;
    }
    let glb_bytes = bytes.len();
    crate::storage::write_new(output.join("mesh.glb"), bytes)?;
    let report = json!({"triangles":triangles.len(),"vertices_with_uv_seams":positions.len(),"selected_views":tiles.iter().map(|t|json!({"name":t.row.name,"image_size":t.size,"crop":t.bounds})).collect::<Vec<_>>(),"atlas_size":[atlas.width,atlas.height],"untextured_triangles":chosen.iter().filter(|s|**s==count).count(),"untextured_area_fraction":untextured_area/total_area.max(1e-20),"glb_bytes":glb_bytes,"seconds":start.elapsed().as_secs_f64(),"reference_used":false,"geometry_changed":false,"coherent_selection":coherent,"seam_edges":seam_report,"exposure_balance":balance,"exposure_gains":gains,"attribution_embedded":attribution.is_some(),"original_rgb_views":tiles.iter().filter(|tile|tile.row.texture_image.is_some()).count(),"color":"Captured input photo appearance including lighting; original RGB when texture_image is supplied, otherwise the legacy matching image; not intrinsic albedo or generated PBR","limits":["Per-face projection; optional coherent view selection and bounded global exposure correction do not remove spatial lighting differences or blend chart boundaries","Visibility tested at corners and centre against 2048-pixel mesh depth buffers","Unobserved faces are neutral grey; no invented texture","Mesh units/orientation are preserved; physical scale is not established"]});
    crate::storage::write_new(output.join("result.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}

fn glb(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uv: &[[f32; 2]],
    indices: &[u32],
    png: &[u8],
    attribution: Option<&str>,
) -> Result<Vec<u8>> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    for bytes in
        [bytemuck::cast_slice(positions), bytemuck::cast_slice(normals), bytemuck::cast_slice(uv), bytemuck::cast_slice(indices), png]
    {
        while bin.len() % 4 != 0 {
            bin.push(0)
        }
        views.push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":bytes.len()}));
        bin.extend_from_slice(bytes);
    }
    for (i, v) in views.iter_mut().take(4).enumerate() {
        v["target"] = json!(if i == 3 { 34963 } else { 34962 });
    }
    let min: [f32; 3] = [0, 1, 2].map(|a| positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min));
    let max: [f32; 3] = [0, 1, 2].map(|a| positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max));
    let mut doc = json!({"asset":{"version":"2.0","generator":"CrispMesh photo texture"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3,"material":0}]}],"buffers":[{"byteLength":bin.len()}],"bufferViews":views,"accessors":[{"bufferView":0,"componentType":5126,"count":positions.len(),"type":"VEC3","min":min,"max":max},{"bufferView":1,"componentType":5126,"count":normals.len(),"type":"VEC3"},{"bufferView":2,"componentType":5126,"count":uv.len(),"type":"VEC2"},{"bufferView":3,"componentType":5125,"count":indices.len(),"type":"SCALAR"}],"images":[{"bufferView":4,"mimeType":"image/png"}],"samplers":[{"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071}],"textures":[{"source":0,"sampler":0}],"materials":[{"name":"Captured photo appearance","pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":1}}]});
    if let Some(text) = attribution {
        doc["asset"]["copyright"] = json!(text);
    }
    let mut text = serde_json::to_vec(&doc)?;
    while text.len() % 4 != 0 {
        text.push(b' ')
    }
    while bin.len() % 4 != 0 {
        bin.push(0)
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    ensure!(total <= u32::MAX as usize, "GLB exceeds 4 GiB");
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend(text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend(bin);
    Ok(out)
}

pub fn main(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("crispmesh-dense texture --inputs DIR --mesh mesh.stl --output NEW_DIR [--views 12] [--tile-size 1024] [--coherent] [--color-balance]\nWrites atlas.png, self-contained mesh.glb and coverage report. Uses recovered cameras and photos; geometry is preserved.");
        return Ok(());
    }
    let (mut inputs, mut mesh, mut output) = (None, None, None);
    let (mut views, mut tile) = (12, 1024);
    let (mut coherent, mut balance) = (false, false);
    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        i += 1;
        if key == "--coherent" {
            coherent = true;
            continue;
        }
        if key == "--color-balance" {
            balance = true;
            continue;
        }
        let value = args.get(i).ok_or_else(|| anyhow::anyhow!("missing value for {key}"))?;
        i += 1;
        match key.as_str() {
            "--inputs" => inputs = Some(value),
            "--mesh" => mesh = Some(value),
            "--output" => output = Some(value),
            "--views" => views = value.parse()?,
            "--tile-size" => tile = value.parse()?,
            _ => bail!("unknown texture option {key}"),
        }
    }
    let need = |v: Option<&String>, name: &str| v.cloned().ok_or_else(|| anyhow::anyhow!("missing {name}"));
    let result = run_with_options(
        Path::new(&need(inputs, "--inputs")?),
        Path::new(&need(mesh, "--mesh")?),
        Path::new(&need(output, "--output")?),
        views,
        tile,
        &crate::control::Control::none(),
        coherent,
        balance,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coherent_selection_keeps_visibility_and_reduces_label_seams() {
        let mut chosen = vec![0, 1, 0, 3];
        let candidates = vec![
            [(0, 1.0), (1, 0.99), (u32::MAX, 0.0)],
            [(1, 1.0), (0, 0.99), (u32::MAX, 0.0)],
            [(0, 1.0), (1, 0.99), (u32::MAX, 0.0)],
            [(u32::MAX, 0.0); 3],
        ];
        let adjacent = vec![[1, usize::MAX, usize::MAX], [0, 2, usize::MAX], [1, 3, usize::MAX], [2, usize::MAX, usize::MAX]];
        let before = seam_edges(&chosen, &adjacent, 3);
        coherent_choices(&mut chosen, &candidates, &adjacent, 3);
        assert!(seam_edges(&chosen, &adjacent, 3) < before);
        assert_eq!(chosen[3], 3, "unobserved face must stay grey");
        for i in 0..3 {
            assert!(candidates[i].iter().any(|c| c.0 as usize == chosen[i]));
        }
    }
    #[test]
    fn exposure_correction_matches_shared_samples_and_leaves_isolated_views() {
        let a: Vec<_> = (0..80).map(|i| Some(0.2 + i as f32 * 0.003)).collect();
        let b: Vec<_> = a.iter().map(|v| v.map(|v| v * 1.2)).collect();
        let gains = exposure_gains(&[a.clone(), b.clone(), vec![None; 80]]);
        assert!((gains[0] / gains[1] - 1.2).abs() < 1e-4);
        assert_eq!(gains[2], 1.0);
        assert!(gains.iter().all(|g| (0.8..=1.25).contains(g)));
        assert!(exposure_gains(&[a.clone(), a]).iter().all(|g| (*g - 1.0).abs() < 1e-6));
    }
    #[test]
    fn visibility_uses_front_surface_and_perspective_depth() {
        let far = [[0., 0., 4.], [4., 0., 4.], [0., 4., 4.]];
        let near = [[0., 0., 1.], [4., 0., 2.], [0., 4., 2.]];
        let z = zbuffer(&[far, near], 5, 5);
        assert!((z[6] - 4. / 3.).abs() < 1e-12);
        assert!(z[24].is_infinite());
    }
    #[test]
    fn actual_export_textures_front_faces_but_not_occluded_faces() {
        let root = format!("mem:/texture-test-{}", std::process::id());
        let root = Path::new(&root);
        crate::storage::create_dir_all(root).unwrap();
        Rgb::filled(64, 64, [210, 30, 20]).save(&root.join("photo.png")).unwrap();
        crate::storage::save_png(root.join("mask.png"), 64, 64, 1, &vec![255; 4096]).unwrap();
        let cameras = json!({"views":[{"name":"own-camera","image":"photo.png","mask":"mask.png","k":[20,20,32,32],"rotation":[[1,0,0],[0,1,0],[0,0,1]],"translation":[0,0,0]}]});
        crate::storage::write_new(root.join("cameras.json"), serde_json::to_vec(&cameras).unwrap()).unwrap();
        let vertices = [[-1., -1., 1.], [-1., 1., 1.], [1., 1., 1.], [1., -1., 1.], [-0.2, -0.2, 2.], [-0.2, 0.2, 2.], [0.2, -0.2, 2.]];
        crate::stl::write_binary(&root.join("mesh.stl"), "test", &vertices, &[[0, 1, 2], [0, 2, 3], [4, 5, 6]]).unwrap();
        let before = crate::storage::read(root.join("mesh.stl")).unwrap();
        let report = run(root, &root.join("mesh.stl"), &root.join("output"), 4, 128).unwrap();
        assert_eq!(report["untextured_triangles"], 1);
        assert_eq!(report["triangles"], 3);
        assert_eq!(before, crate::storage::read(root.join("mesh.stl")).unwrap());
        let enhanced =
            run_with_options(root, &root.join("mesh.stl"), &root.join("enhanced"), 4, 128, &crate::control::Control::none(), true, true)
                .unwrap();
        assert_eq!(enhanced["untextured_triangles"], 1);
        assert_eq!(enhanced["geometry_changed"], false);
        assert_eq!(before, crate::storage::read(root.join("mesh.stl")).unwrap());
        // A feature-enhanced image must not override calibrated original RGB.
        Rgb::filled(64, 64, [19, 67, 183]).save(&root.join("original.png")).unwrap();
        let mut originals = cameras.clone();
        originals["views"][0]["texture_image"] = json!("original.png");
        crate::storage::write(root.join("cameras.json"), serde_json::to_vec(&originals).unwrap()).unwrap();
        let original_report = run(root, &root.join("mesh.stl"), &root.join("original-output"), 4, 128).unwrap();
        let atlas = Rgb::open(&root.join("original-output/atlas.png")).unwrap();
        assert!(atlas.data.as_chunks::<3>().0.contains(&[19, 67, 183]));
        assert!(!atlas.data.as_chunks::<3>().0.contains(&[210, 30, 20]));
        assert_eq!(original_report["untextured_triangles"], report["untextured_triangles"]);
        assert_eq!(before, crate::storage::read(root.join("mesh.stl")).unwrap());
        assert!(run(root, &root.join("mesh.stl"), &root.join("output"), 4, 128).is_err());
        crate::storage::remove_dir_all(root).unwrap();
    }
    #[test]
    fn glb_has_embedded_texture_and_correct_chunk_boundaries() {
        let positions = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let blob = glb(&positions, &[[0., 0., 1.]; 3], &[[0., 0.]; 3], &[0, 1, 2], b"test-png", Some("Test attribution")).unwrap();
        assert_eq!(&blob[..4], b"glTF");
        assert_eq!(u32::from_le_bytes(blob[8..12].try_into().unwrap()) as usize, blob.len());
        let len = u32::from_le_bytes(blob[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&blob[20..20 + len]).unwrap();
        assert_eq!(doc["asset"]["copyright"], "Test attribution");
        let bin = 28 + len;
        let off = doc["bufferViews"][4]["byteOffset"].as_u64().unwrap() as usize;
        assert_eq!(&blob[bin + off..bin + off + 8], b"test-png");
        assert_eq!(doc["accessors"][0]["max"], json!([1., 1., 0.]));
    }
}
