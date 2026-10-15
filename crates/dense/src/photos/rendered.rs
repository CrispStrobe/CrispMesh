//! Rendered turntable captures of textured meshes: `crispmesh-dense render`.
//!
//! A textured Wavefront `.obj` (as the Google Scanned Objects come, CC BY 4.0)
//! stands on a light, faintly textured disc in front of a light backdrop; a
//! camera with a distorting lens circles it on one ring, as a turntable capture
//! would see it (the light turns with the camera, so the object turns under a
//! fixed light). Every pixel's ray is undistorted with the lens and cast
//! against the mesh through a bounding volume hierarchy, so texturing is
//! perspective-correct and the lens model is exact. Lambert and ambient light,
//! a hard shadow from the one light, optional blur and noise.
//!
//! Writes `photos/shot_NNN.png`, `masks/shot_NNN.png` (the exact object silhouette, where at least
//! half of a pixel's samples meet the object; for `--masks import:DIR` when the masks are not the
//! question), `lens.json` (the lens the photos were made
//! with, `crispmesh_lens_calibration_v1`), `truth.json` (cameras, placement)
//! and `reference.ply` (the mesh as placed, for `scan_evaluate --no-platform`;
//! evaluation only). Without `--mesh`, `--sphere` renders a tessellated unit
//! sphere with the texture of `synthetic --capture`, to check the renderer
//! against that capture.

#![allow(clippy::needless_range_loop)]

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context};
use serde_json::json;

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: V3) -> V3 {
    let n = dot(a, a).sqrt().max(1e-300);
    scale(a, 1.0 / n)
}

/// A triangle mesh with optional texture coordinates and vertex normals per corner.
pub struct Mesh {
    pub positions: Vec<V3>,
    pub triangles: Vec<[usize; 3]>,
    /// Per triangle corner: texture coordinate (u, v with v up, as in `.obj`).
    pub uv: Option<Vec<[[f64; 2]; 3]>>,
    pub normals: Option<Vec<[V3; 3]>>,
    pub texture: Option<image::RgbImage>,
}

/// Reads an `.obj` with its first `map_Kd` texture (polygons are fanned into triangles).
pub fn read_obj(path: &Path) -> anyhow::Result<Mesh> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let folder = path.parent().unwrap_or(Path::new("."));
    let (mut v, mut vt, mut vn) = (Vec::new(), Vec::new(), Vec::new());
    let (mut triangles, mut uv, mut normals) = (Vec::new(), Vec::new(), Vec::new());
    let (mut all_uv, mut all_normals) = (true, true);
    let mut texture_path: Option<PathBuf> = None;
    let number = |s: &str| s.parse::<f64>().map_err(|e| anyhow!("{}: {s}: {e}", path.display()));
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let p: Vec<f64> = parts.take(3).map(number).collect::<anyhow::Result<_>>()?;
                ensure!(p.len() == 3, "{}: short vertex", path.display());
                v.push([p[0], p[1], p[2]]);
            }
            Some("vt") => {
                let p: Vec<f64> = parts.take(2).map(number).collect::<anyhow::Result<_>>()?;
                ensure!(p.len() == 2, "{}: short texture coordinate", path.display());
                vt.push([p[0], p[1]]);
            }
            Some("vn") => {
                let p: Vec<f64> = parts.take(3).map(number).collect::<anyhow::Result<_>>()?;
                ensure!(p.len() == 3, "{}: short normal", path.display());
                vn.push(unit([p[0], p[1], p[2]]));
            }
            Some("f") => {
                let corners: Vec<(usize, Option<usize>, Option<usize>)> = parts
                    .map(|corner| {
                        let mut fields = corner.split('/');
                        let index = |field: Option<&str>, count: usize| -> anyhow::Result<Option<usize>> {
                            match field {
                                None | Some("") => Ok(None),
                                Some(text) => {
                                    let i: i64 = text.parse().map_err(|e| anyhow!("{}: {text}: {e}", path.display()))?;
                                    let i = if i < 0 { count as i64 + i } else { i - 1 };
                                    ensure!(i >= 0 && (i as usize) < count, "{}: index {text} out of range", path.display());
                                    Ok(Some(i as usize))
                                }
                            }
                        };
                        let p = index(fields.next(), v.len())?.ok_or_else(|| anyhow!("{}: face without vertex", path.display()))?;
                        Ok((p, index(fields.next(), vt.len())?, index(fields.next(), vn.len())?))
                    })
                    .collect::<anyhow::Result<_>>()?;
                for k in 1..corners.len().saturating_sub(1) {
                    let c = [corners[0], corners[k], corners[k + 1]];
                    triangles.push([c[0].0, c[1].0, c[2].0]);
                    match (c[0].1, c[1].1, c[2].1) {
                        (Some(a), Some(b), Some(d)) => uv.push([vt[a], vt[b], vt[d]]),
                        _ => {
                            all_uv = false;
                            uv.push([[0.0; 2]; 3]);
                        }
                    }
                    match (c[0].2, c[1].2, c[2].2) {
                        (Some(a), Some(b), Some(d)) => normals.push([vn[a], vn[b], vn[d]]),
                        _ => {
                            all_normals = false;
                            normals.push([[0.0; 3]; 3]);
                        }
                    }
                }
            }
            Some("mtllib") => {
                let name = line.trim_start()["mtllib".len()..].trim();
                let mtl = folder.join(name);
                if let Ok(material) = std::fs::read_to_string(&mtl) {
                    for m in material.lines() {
                        let m = m.trim();
                        if let Some(rest) = m.strip_prefix("map_Kd") {
                            let file = rest.split_whitespace().last().unwrap_or_default();
                            texture_path.get_or_insert(mtl.parent().unwrap_or(folder).join(file));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    ensure!(!triangles.is_empty(), "{}: no faces", path.display());
    // Google Scanned Objects name the texture beside the model but keep it in ../materials/textures.
    let texture_path = texture_path.map(|file| {
        let beside = folder.join("../materials/textures").join(file.file_name().unwrap_or_default());
        if !file.exists() && beside.exists() {
            beside
        } else {
            file
        }
    });
    let texture = match &texture_path {
        Some(file) => Some(image::open(file).with_context(|| file.display().to_string())?.to_rgb8()),
        None => None,
    };
    Ok(Mesh { positions: v, triangles, uv: (all_uv && texture.is_some()).then_some(uv), normals: all_normals.then_some(normals), texture })
}

/// A unit sphere tessellated from an icosahedron, `level` times subdivided.
pub fn sphere(level: usize) -> Mesh {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut positions: Vec<V3> = [
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
    .map(|p| unit(*p))
    .collect();
    let mut triangles: Vec<[usize; 3]> = vec![
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
    for _ in 0..level {
        let mut middle = std::collections::HashMap::new();
        let mut split = |a: usize, b: usize, positions: &mut Vec<V3>| -> usize {
            *middle.entry((a.min(b), a.max(b))).or_insert_with(|| {
                positions.push(unit(scale(add(positions[a], positions[b]), 0.5)));
                positions.len() - 1
            })
        };
        let mut finer = Vec::with_capacity(triangles.len() * 4);
        for [a, b, c] in triangles {
            let (ab, bc, ca) = (split(a, b, &mut positions), split(b, c, &mut positions), split(c, a, &mut positions));
            finer.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        triangles = finer;
    }
    let normals = triangles.iter().map(|t| [positions[t[0]], positions[t[1]], positions[t[2]]]).collect();
    Mesh { positions, triangles, uv: None, normals: Some(normals), texture: None }
}

/// Bounding volume hierarchy over triangles; leaves of at most four.
struct Bvh {
    nodes: Vec<Node>,
    order: Vec<usize>,
}

struct Node {
    lo: V3,
    hi: V3,
    /// Leaf: first index into `order` and count; inner: children at `first` and `first + 1`.
    first: usize,
    count: usize,
}

impl Bvh {
    fn build(corners: &[[V3; 3]]) -> Bvh {
        let centroids: Vec<V3> = corners.iter().map(|c| scale(add(add(c[0], c[1]), c[2]), 1.0 / 3.0)).collect();
        let mut order: Vec<usize> = (0..corners.len()).collect();
        let mut nodes = vec![Node { lo: [0.0; 3], hi: [0.0; 3], first: 0, count: corners.len() }];
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let (first, count) = (nodes[index].first, nodes[index].count);
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for &t in &order[first..first + count] {
                for p in corners[t] {
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
            }
            nodes[index].lo = lo;
            nodes[index].hi = hi;
            if count <= 4 {
                continue;
            }
            let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap_or(0);
            order[first..first + count].sort_unstable_by(|&a, &b| centroids[a][axis].total_cmp(&centroids[b][axis]));
            let half = count / 2;
            let left = nodes.len();
            nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], first, count: half });
            nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], first: first + half, count: count - half });
            nodes[index].first = left;
            nodes[index].count = 0;
            stack.push(left);
            stack.push(left + 1);
        }
        Bvh { nodes, order }
    }

    /// Nearest hit: (ray parameter, triangle, barycentric b1, b2), or any hit before `limit` when `any`.
    fn hit(&self, corners: &[[V3; 3]], origin: V3, direction: V3, limit: f64, any: bool) -> Option<(f64, usize, f64, f64)> {
        let inverse = direction.map(|d| if d.abs() > 1e-300 { 1.0 / d } else { 1e300 });
        let mut best: Option<(f64, usize, f64, f64)> = None;
        let mut nearest = limit;
        let mut stack = [0usize; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top]];
            let (mut enter, mut leave) = (0.0f64, nearest);
            for a in 0..3 {
                let (t0, t1) = ((node.lo[a] - origin[a]) * inverse[a], (node.hi[a] - origin[a]) * inverse[a]);
                enter = enter.max(t0.min(t1));
                leave = leave.min(t0.max(t1));
            }
            if enter > leave {
                continue;
            }
            if node.count == 0 {
                if top + 2 > stack.len() {
                    continue;
                }
                stack[top] = node.first;
                stack[top + 1] = node.first + 1;
                top += 2;
                continue;
            }
            for &t in &self.order[node.first..node.first + node.count] {
                let [a, b, c] = corners[t];
                let (e1, e2) = (sub(b, a), sub(c, a));
                let p = cross(direction, e2);
                let det = dot(e1, p);
                if det.abs() < 1e-18 {
                    continue;
                }
                let s = sub(origin, a);
                let u = dot(s, p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = cross(s, e1);
                let w = dot(direction, q) / det;
                if w < 0.0 || u + w > 1.0 {
                    continue;
                }
                let distance = dot(e2, q) / det;
                if distance > 1e-9 && distance < nearest {
                    nearest = distance;
                    best = Some((distance, t, u, w));
                    if any {
                        return best;
                    }
                }
            }
        }
        best
    }
}

/// The lens the photos are made with (pixel-centre principal point, three radial terms).
#[derive(Debug, Clone, Copy)]
pub struct Lens {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub k: [f64; 3],
}

impl Lens {
    /// The normalised ray direction (x, y, 1) of a distorted pixel position.
    fn ray(&self, x: f64, y: f64) -> (f64, f64) {
        let (xd, yd) = ((x - self.cx) / self.fx, (y - self.cy) / self.fy);
        let (mut u, mut v) = (xd, yd);
        for _ in 0..12 {
            let r2 = u * u + v * v;
            let gain = 1.0 + r2 * (self.k[0] + r2 * (self.k[1] + r2 * self.k[2]));
            (u, v) = (xd / gain, yd / gain);
        }
        (u, v)
    }
}

/// What to render and how.
pub struct Settings {
    pub views: usize,
    pub elevation_deg: f64,
    pub width: usize,
    pub height: usize,
    /// Share of the frame height the object's bounding sphere fills.
    pub fill: f64,
    pub lens: Option<Lens>,
    /// Flat albedo, no disc, no light (the look of `synthetic --capture`, for checking).
    pub flat: bool,
    pub blur: f64,
    pub noise: f64,
    pub samples: usize,
    pub threads: usize,
    /// Distance from the object's centre; `None` picks it from `fill`.
    pub distance: Option<f64>,
    /// Grey level of the backdrop (0 to 1).
    pub backdrop: f64,
}

/// Albedo of the disc under the object: light, with faint blotches and a fine grain.
fn disc_albedo(p: V3) -> [f64; 3] {
    let n = crate::stereo::capture::texture(scale(p, 3.0));
    let g = 0.70 + 0.35 * (n - 0.06) / 0.44;
    [g * 0.98, g * 0.97, g * 0.93]
}

fn bilinear(texture: &image::RgbImage, u: f64, v: f64) -> [f64; 3] {
    let (w, h) = (texture.width() as f64, texture.height() as f64);
    let x = (u - u.floor()) * w - 0.5;
    let y = (1.0 - (v - v.floor())) * h - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: f64, yi: f64| {
        let xi = (xi.rem_euclid(w)) as u32;
        let yi = yi.clamp(0.0, h - 1.0) as u32;
        texture.get_pixel(xi, yi).0.map(|c| c as f64 / 255.0)
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    [0, 1, 2].map(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
}

/// Renders the capture into `output` (a fresh directory). Returns the truth record.
pub fn render(mesh: &Mesh, settings: &Settings, output: &Path, source: &str) -> anyhow::Result<serde_json::Value> {
    ensure!(settings.views >= 8 && settings.width >= 160 && settings.height >= 120, "at least 8 views of 160 x 120 pixels");
    ensure!(!output.exists(), "output directory exists: {}", output.display());
    // Placement: the object's footprint centred on the axis, its base on the disc, its middle at the origin.
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &mesh.positions {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let shift = [-(lo[0] + hi[0]) / 2.0, -(lo[1] + hi[1]) / 2.0, -(lo[2] + hi[2]) / 2.0];
    let placed: Vec<V3> = mesh.positions.iter().map(|p| add(*p, shift)).collect();
    let radius = placed.iter().map(|p| dot(*p, *p).sqrt()).fold(0.0, f64::max);
    let base = (lo[2] - hi[2]) / 2.0;
    let footprint = placed.iter().map(|p| p[0].hypot(p[1])).fold(0.0, f64::max);
    let disc_radius = 1.6 * footprint.max(0.3 * radius);
    let corners: Vec<[V3; 3]> = mesh.triangles.iter().map(|t| [placed[t[0]], placed[t[1]], placed[t[2]]]).collect();
    let bvh = Bvh::build(&corners);

    let lens = settings.lens.unwrap_or_else(|| {
        let focal = 2.0 * settings.height as f64;
        Lens {
            fx: focal,
            fy: focal * 1.002,
            cx: settings.width as f64 / 2.0 - 0.7,
            cy: settings.height as f64 / 2.0 + 0.4,
            k: [-0.06, 0.02, -0.004],
        }
    });
    let distance = settings.distance.unwrap_or_else(|| {
        // The bounding sphere fills `fill` of the frame height.
        let half = (settings.height as f64 * settings.fill / 2.0) / lens.fy;
        radius / half.atan().sin()
    });
    std::fs::create_dir_all(output.join("photos")).with_context(|| output.display().to_string())?;
    std::fs::create_dir_all(output.join("masks")).with_context(|| output.display().to_string())?;
    let mut truth_views = Vec::new();
    for n in 0..settings.views {
        let (rotation, translation) =
            crate::stereo::synthetic::camera(360.0 * n as f64 / settings.views as f64, settings.elevation_deg, distance);
        let centre =
            [0, 1, 2].map(|a| -(rotation[0][a] * translation[0] + rotation[1][a] * translation[1] + rotation[2][a] * translation[2]));
        // The light turns with the camera. Direction towards the light in camera terms: up (y is down),
        // to the left, and back towards the camera.
        let in_camera = unit([-0.4, -0.8, -0.45]);
        let light = unit([0, 1, 2].map(|a| rotation[0][a] * in_camera[0] + rotation[1][a] * in_camera[1] + rotation[2][a] * in_camera[2]));
        let (w, h, samples) = (settings.width, settings.height, settings.samples.max(1));
        let offsets: Vec<(f64, f64)> = match samples {
            1 => vec![(0.5, 0.5)],
            _ => vec![(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)],
        };
        // Colour of the ray through (x, y), and whether it meets the object first.
        let shade = |x: f64, y: f64| -> ([f64; 3], bool) {
            let (u, v) = lens.ray(x, y);
            let direction = unit([0, 1, 2].map(|a| u * rotation[0][a] + v * rotation[1][a] + rotation[2][a]));
            let object = bvh.hit(&corners, centre, direction, f64::INFINITY, false);
            // The disc: the plane z = base inside `disc_radius`, seen from above only.
            let disc = if settings.flat || direction[2] >= 0.0 {
                None
            } else {
                let t = (base - centre[2]) / direction[2];
                let p = add(centre, scale(direction, t));
                (t > 0.0 && p[0].hypot(p[1]) <= disc_radius).then_some((t, p))
            };
            let lit = |point: V3, normal: V3| -> f64 {
                if settings.flat {
                    return 1.0;
                }
                let facing = dot(normal, light).max(0.0);
                let shadowed =
                    facing > 0.0 && bvh.hit(&corners, add(point, scale(normal, 1e-6 * radius)), light, f64::INFINITY, true).is_some();
                0.38 + 0.62 * if shadowed { 0.0 } else { facing }
            };
            match (object, disc) {
                (Some((t, triangle, b1, b2)), d) if d.is_none_or(|(td, _)| t <= td) => {
                    let point = add(centre, scale(direction, t));
                    let b0 = 1.0 - b1 - b2;
                    let [a, b, c] = corners[triangle];
                    let mut normal = match &mesh.normals {
                        Some(normals) => unit(add(
                            add(scale(normals[triangle][0], b0), scale(normals[triangle][1], b1)),
                            scale(normals[triangle][2], b2),
                        )),
                        None => unit(cross(sub(b, a), sub(c, a))),
                    };
                    if dot(normal, direction) > 0.0 {
                        normal = scale(normal, -1.0);
                    }
                    let albedo = match (&mesh.texture, &mesh.uv) {
                        (Some(texture), Some(uv)) => {
                            let [ta, tb, tc] = uv[triangle];
                            bilinear(texture, b0 * ta[0] + b1 * tb[0] + b2 * tc[0], b0 * ta[1] + b1 * tb[1] + b2 * tc[1])
                        }
                        // Untextured: the solid texture of `synthetic --capture` on the unit-sized object.
                        _ => [crate::stereo::capture::texture(scale(point, 1.0 / radius)); 3],
                    };
                    let light = lit(point, normal);
                    (albedo.map(|c| c * light), true)
                }
                (_, Some((_, p))) => {
                    let light = lit(p, [0.0, 0.0, 1.0]);
                    (disc_albedo(p).map(|c| c * light), false)
                }
                _ => ([settings.backdrop, settings.backdrop, settings.backdrop * 1.01], false),
            }
        };
        let rows: Vec<(Vec<f32>, Vec<u8>)> = {
            let next = std::sync::atomic::AtomicUsize::new(0);
            let mut rows: Vec<Option<(Vec<f32>, Vec<u8>)>> = (0..h).map(|_| None).collect();
            let done = std::sync::Mutex::new(&mut rows);
            std::thread::scope(|scope| {
                for _ in 0..settings.threads.max(1) {
                    scope.spawn(|| loop {
                        let y = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if y >= h {
                            break;
                        }
                        let mut row = vec![0f32; w * 3];
                        let mut coverage = vec![0u8; w];
                        for x in 0..w {
                            let (mut sum, mut hits) = ([0.0; 3], 0);
                            for &(sx, sy) in &offsets {
                                let (c, hit) = shade(x as f64 - 0.5 + sx, y as f64 - 0.5 + sy);
                                for i in 0..3 {
                                    sum[i] += c[i];
                                }
                                hits += hit as usize;
                            }
                            for i in 0..3 {
                                row[x * 3 + i] = (sum[i] / offsets.len() as f64) as f32;
                            }
                            coverage[x] = if 2 * hits >= offsets.len() { 255 } else { 0 };
                        }
                        done.lock().unwrap()[y] = Some((row, coverage));
                    });
                }
            });
            rows.into_iter().map(|r| r.expect("rendered")).collect()
        };
        let (rows, coverage): (Vec<Vec<f32>>, Vec<Vec<u8>>) = rows.into_iter().unzip();
        let mut pixels: Vec<f32> = rows.concat();
        if settings.blur > 0.0 {
            for channel in 0..3 {
                let plane: Vec<f32> = pixels.iter().skip(channel).step_by(3).copied().collect();
                let blurred = blur(&plane, w, h, settings.blur);
                for (i, value) in blurred.into_iter().enumerate() {
                    pixels[i * 3 + channel] = value;
                }
            }
        }
        let mut seed = (n as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let bytes: Vec<u8> = pixels
            .iter()
            .map(|&value| {
                let mut value = value as f64;
                if settings.noise > 0.0 {
                    // Box-Muller from a splitmix stream: deterministic grain.
                    let mut next = || {
                        seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                        let mut z = seed;
                        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
                    };
                    let (a, b) = (next().max(1e-12), next());
                    value += settings.noise * (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos();
                }
                (255.0 * value.clamp(0.0, 1.0) + 0.5) as u8
            })
            .collect();
        let name = format!("shot_{n:03}.png");
        crate::storage::save_png(output.join("photos").join(&name), w, h, 3, &bytes)?;
        crate::storage::save_png(output.join("masks").join(&name), w, h, 1, &coverage.concat())?;
        truth_views.push(json!({"photo": name, "centre": centre, "rotation": rotation}));
    }
    let calibration = json!({
        "schema": "crispmesh_lens_calibration_v1", "model": "radialk3", "calibration_width": settings.width, "calibration_height": settings.height,
        "fx": lens.fx, "fy": lens.fy, "cx": lens.cx, "cy": lens.cy, "k1": lens.k[0], "k2": lens.k[1], "k3": lens.k[2],
        "principal_point_convention": "pixel_centre", "sensor_width_mm": 36.0,
        "provenance": {"source": "crispmesh-dense render: the lens the photos were rendered with"},
    });
    std::fs::write(output.join("lens.json"), serde_json::to_string_pretty(&calibration)? + "\n")?;
    write_ply(&output.join("reference.ply"), &placed, &mesh.triangles)?;
    let truth = json!({
        "source": source, "views": truth_views, "orbit_radius": distance, "elevation_degrees": settings.elevation_deg,
        "object_radius": radius, "disc_radius": disc_radius, "disc_height": base, "placement_shift": shift,
        "camera_convention": "x right, y down, z forward (right-handed); rotation maps world to camera; z up in the world",
        "reference": "reference.ply: the mesh as placed (same frame as the cameras), for evaluation only",
    });
    std::fs::write(output.join("truth.json"), serde_json::to_string_pretty(&truth)? + "\n")?;
    Ok(truth)
}

fn write_ply(path: &Path, positions: &[V3], triangles: &[[usize; 3]]) -> anyhow::Result<()> {
    let mut bytes = format!(
        "ply\nformat binary_little_endian 1.0\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\nelement face {}\nproperty list uchar int vertex_indices\nend_header\n",
        positions.len(),
        triangles.len()
    )
    .into_bytes();
    for p in positions {
        for c in p {
            bytes.extend((*c as f32).to_le_bytes());
        }
    }
    for t in triangles {
        bytes.push(3);
        for &i in t {
            bytes.extend((i as i32).to_le_bytes());
        }
    }
    std::fs::write(path, bytes).with_context(|| path.display().to_string())
}

fn blur(source: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    let reach = (3.0 * sigma).ceil() as isize;
    let weights: Vec<f32> = (-reach..=reach).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp() as f32).collect();
    let total: f32 = weights.iter().sum();
    let pass = |data: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0f32; data.len()];
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0f32;
                for (k, weight) in weights.iter().enumerate() {
                    let o = k as isize - reach;
                    let (xx, yy) = if horizontal {
                        ((x as isize + o).clamp(0, width as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + o).clamp(0, height as isize - 1) as usize)
                    };
                    sum += weight * data[yy * width + xx];
                }
                out[y * width + x] = sum / total;
            }
        }
        out
    };
    pass(&pass(source, true), false)
}

const USAGE: &str = "usage: crispmesh-dense render (--mesh MODEL.obj | --sphere) --output DIR [--views N (72)] [--elevation DEG (20)]
       [--width W (1749)] [--height H (1155)] [--fill F (0.7)] [--calibration LENS.json] [--distance D]
       [--samples 1|4 (4)] [--blur SIGMA_PX (0)] [--noise SIGMA (0)] [--backdrop GREY (0.86)] [--flat] [--threads N (4)]";

/// The `render` command.
pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    let (mut mesh, mut sphere_level, mut output, mut calibration) = (None, None, None, None);
    let mut settings = Settings {
        views: 72,
        elevation_deg: 20.0,
        width: 1749,
        height: 1155,
        fill: 0.7,
        lens: None,
        flat: false,
        blur: 0.0,
        noise: 0.0,
        samples: 4,
        threads: 4,
        distance: None,
        backdrop: 0.86,
    };
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().cloned().ok_or_else(|| anyhow!("{flag} needs a value\n{USAGE}"));
        match flag.as_str() {
            "--mesh" => mesh = Some(PathBuf::from(value()?)),
            "--sphere" => sphere_level = Some(6),
            "--output" => output = Some(PathBuf::from(value()?)),
            "--calibration" => calibration = Some(PathBuf::from(value()?)),
            "--views" => settings.views = value()?.parse()?,
            "--elevation" => settings.elevation_deg = value()?.parse()?,
            "--width" => settings.width = value()?.parse()?,
            "--height" => settings.height = value()?.parse()?,
            "--fill" => settings.fill = value()?.parse()?,
            "--distance" => settings.distance = Some(value()?.parse()?),
            "--samples" => settings.samples = value()?.parse()?,
            "--blur" => settings.blur = value()?.parse()?,
            "--noise" => settings.noise = value()?.parse()?,
            "--threads" => settings.threads = value()?.parse()?,
            "--backdrop" => settings.backdrop = value()?.parse()?,
            "--flat" => settings.flat = true,
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    let output = output.ok_or_else(|| anyhow!("--output is required\n{USAGE}"))?;
    if let Some(path) = calibration {
        let scaled = crate::photos::calibration::scale_calibration(
            &crate::photos::calibration::load_calibration(&path)?,
            settings.width as u32,
            settings.height as u32,
        )?;
        settings.lens = Some(Lens { fx: scaled.fx, fy: scaled.fy, cx: scaled.cx, cy: scaled.cy, k: scaled.k });
    }
    let (mesh, source) = match (mesh, sphere_level) {
        (Some(path), None) => (read_obj(&path)?, path.display().to_string()),
        (None, Some(level)) => (sphere(level), "unit sphere (icosahedron subdivided 6 times)".to_string()),
        _ => bail!("give --mesh MODEL.obj or --sphere\n{USAGE}"),
    };
    let started = std::time::Instant::now();
    render(&mesh, &settings, &output, &source)?;
    println!("{} views in {:.1} s: {}", settings.views, started.elapsed().as_secs_f64(), output.join("photos").display());
    println!("calibration: {}", output.join("lens.json").display());
    Ok(())
}
