//! Plane scoring and independent per-pixel coordinate refinement.
//! The neighboring-plane experiment is compiled only for research builds.

#![allow(clippy::needless_range_loop, clippy::neg_cmp_op_on_partial_ord)]

use bytemuck::{Pod, Zeroable};

use crate::gpu::{pack_bits, Gpu, Kernel};
use crate::hull::Hull;
use crate::inputs::Plane;

use super::level::LevelView;

/// Neighbour views a plane is scored against, at most.
pub const MAX_NEIGHBOURS: usize = 8;
/// Distance of the far propagation candidates, pixels.
pub const FAR: i32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct NeighbourParams {
    r0: [f32; 4],
    r1: [f32; 4],
    r2: [f32; 4],
    k: [f32; 4],
    extent: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    ref_k: [f32; 4],
    world_r0: [f32; 4],
    world_r1: [f32; 4],
    world_r2: [f32; 4],
    hull: [f32; 4],
    shape: [u32; 4],
    size: [u32; 4],
    pass: [u32; 4],
    gates: [f32; 4],
    range: [f32; 4],
    nbr: [NeighbourParams; MAX_NEIGHBOURS],
}

/// What the search does; [`Settings::new`] has the defaults.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Window radius (half the matching window) and the stride at which it is sampled.
    pub radius: u32,
    pub stride: u32,
    pub best_of: u32,
    pub min_variance: f32,
    pub window_fill: f32,
    /// Independent coordinate iterations (research mode: red-black iterations).
    pub iterations: u32,
    /// Inverse-depth perturbation of the first iteration; halved after each.
    pub inverse_step: f32,
    /// Normal perturbation of the first iteration; halved after each.
    pub normal_step: f32,
    /// Inverse-depth range a plane may take at its pixel (the view's hull bounds).
    pub inverse_range: (f32, f32),
    pub seed: u32,
}

/// The reference view and its neighbours in the extent the kernel reads.
struct Geometry {
    params: Params,
    gray: Vec<f32>,
    mask: Vec<u32>,
}

fn mask_words(view: &LevelView) -> Vec<u32> {
    let stride = view.width.div_ceil(32);
    let mut words = vec![0u32; stride * view.height];
    for y in 0..view.height {
        let row = pack_bits(view.mask.data[y * view.width..(y + 1) * view.width].iter().map(|&m| m != 0));
        words[y * stride..y * stride + row.len()].copy_from_slice(&row);
    }
    words
}

fn geometry(level: &[LevelView], view: usize, neighbours: &[usize], search: &Hull, settings: &Settings) -> Geometry {
    let reference = &level[view];
    let rf = |m: &[[f32; 3]; 3]| m.map(|row| row.map(f64::from));
    let (rr, tr) = (rf(&reference.camera.rotation), reference.camera.translation.map(f64::from));
    // world = R^T x - R^T t
    let mut world = [[0f32; 4]; 3];
    for a in 0..3 {
        let mut shift = 0.0;
        for b in 0..3 {
            world[a][b] = rr[b][a] as f32;
            shift -= rr[b][a] * tr[b];
        }
        world[a][3] = shift as f32;
    }
    let mut nbr = [NeighbourParams::default(); MAX_NEIGHBOURS];
    let (mut gray, mut mask) = (Vec::new(), Vec::new());
    for (n, &j) in neighbours.iter().take(MAX_NEIGHBOURS).enumerate() {
        let other = &level[j];
        let (rj, tj) = (rf(&other.camera.rotation), other.camera.translation.map(f64::from));
        // x_j = R_j R_ref^T x + (t_j - R_j R_ref^T t_ref)
        let mut rows = [[0f32; 4]; 3];
        for a in 0..3 {
            let mut shift = tj[a];
            for b in 0..3 {
                let value: f64 = (0..3).map(|c| rj[a][c] * rr[b][c]).sum();
                rows[a][b] = value as f32;
                shift -= value * tr[b];
            }
            rows[a][3] = shift as f32;
        }
        nbr[n] = NeighbourParams {
            r0: rows[0],
            r1: rows[1],
            r2: rows[2],
            k: other.camera.k,
            extent: [other.width as u32, other.height as u32, gray.len() as u32, mask.len() as u32],
        };
        gray.extend_from_slice(&other.gray.data);
        mask.extend(mask_words(other));
    }
    let params = Params {
        ref_k: reference.camera.k,
        world_r0: world[0],
        world_r1: world[1],
        world_r2: world[2],
        hull: [search.origin[0], search.origin[1], search.origin[2], search.voxel as f32],
        shape: [search.shape[0] as u32, search.shape[1] as u32, search.shape[2] as u32, 0],
        size: [reference.width as u32, reference.height as u32, neighbours.len().min(MAX_NEIGHBOURS) as u32, settings.radius],
        pass: [settings.stride.max(1), settings.best_of.min(4), 0, settings.seed],
        gates: [settings.min_variance, settings.window_fill, settings.inverse_step, settings.normal_step],
        range: [settings.inverse_range.0, settings.inverse_range.1, FAR as f32, 0.0],
        nbr,
    };
    Geometry { params, gray, mask }
}

/// Planes of a depth map: the depth itself, normals from the surface `smooth` (central
/// differences of its back-projected points, one-sided at gaps), fronto-parallel where it has none.
pub fn initial_planes(view: &LevelView, depth: &Plane<f32>, smooth: &Plane<f32>) -> Vec<[f32; 4]> {
    let (w, h) = (view.width, view.height);
    let k = view.camera.k;
    let point = |x: usize, y: usize| -> Option<[f32; 3]> {
        let d = smooth.data[y * w + x];
        (d > 0.0).then(|| [(x as f32 + 0.5 - k[2]) / k[0] * d, (y as f32 + 0.5 - k[3]) / k[1] * d, d])
    };
    let difference = |a: Option<[f32; 3]>, b: Option<[f32; 3]>| match (a, b) {
        (Some(a), Some(b)) => Some([a[0] - b[0], a[1] - b[1], a[2] - b[2]]),
        _ => None,
    };
    let mut out = vec![[0f32; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            let d = if depth.data[p] > 0.0 { depth.data[p] } else { smooth.data[p] };
            if d <= 0.0 || view.mask.data[p] == 0 {
                continue;
            }
            let centre = point(x, y);
            let across = |forward: Option<[f32; 3]>, backward: Option<[f32; 3]>| {
                difference(forward, backward).or(difference(forward, centre)).or(difference(centre, backward))
            };
            let dx = across(if x + 1 < w { point(x + 1, y) } else { None }, if x > 0 { point(x - 1, y) } else { None });
            let dy = across(if y + 1 < h { point(x, y + 1) } else { None }, if y > 0 { point(x, y - 1) } else { None });
            let mut normal = [0.0, 0.0, -1.0];
            if let (Some(u), Some(v)) = (dx, dy) {
                let c = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let length = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                if length > 0.0 {
                    let ray = [(x as f32 + 0.5 - k[2]) / k[0], (y as f32 + 0.5 - k[3]) / k[1], 1.0];
                    let sign = if c[0] * ray[0] + c[1] * ray[1] + c[2] * ray[2] > 0.0 { -1.0 } else { 1.0 };
                    let n = c.map(|v| sign * v / length);
                    // Grazing normals score nothing; keep them at least 0.1 towards the camera.
                    let ray_length = (ray[0] * ray[0] + ray[1] * ray[1] + 1.0f32).sqrt();
                    if -(n[0] * ray[0] + n[1] * ray[1] + n[2] * ray[2]) / ray_length > 0.1 {
                        normal = n;
                    }
                }
            }
            out[p] = [d, normal[0], normal[1], normal[2]];
        }
    }
    out
}

pub struct Planes {
    evaluate: Kernel,
    sweep: Kernel,
    band: Option<Kernel>,
    hull: wgpu::Buffer,
    research: bool,
}

impl Planes {
    #[cfg(any(test, feature = "research-patchmatch"))]
    pub async fn new(gpu: &Gpu, search: &Hull) -> anyhow::Result<Self> {
        let source = include_str!("../shaders/patchmatch.wgsl");
        Ok(Planes {
            evaluate: gpu.kernel("patchmatch evaluate", source, "evaluate").await?,
            sweep: gpu.kernel("patchmatch sweep", source, "sweep").await?,
            band: None,
            hull: gpu.upload("patchmatch hull", &search.bits),
            research: true,
        })
    }

    /// Deterministic coordinate search. No neighboring pixel's plane is read.
    pub async fn independent(gpu: &Gpu, search: &Hull) -> anyhow::Result<Self> {
        let source = include_str!("../shaders/slanted.wgsl");
        Ok(Self {
            evaluate: gpu.kernel("independent plane evaluate", source, "evaluate").await?,
            sweep: gpu.kernel("independent plane refine", source, "sweep").await?,
            band: Some(gpu.kernel("coherent plane band", source, "band").await?),
            hull: gpu.upload("plane hull", &search.bits),
            research: false,
        })
    }

    /// Search a full inverse-depth band using each centre's own surface plane.
    /// The output binding contains depths, not NCC scores. No neighbouring
    /// pixel's prior depth is used to lift individual brightness samples; the
    /// initial normal is estimated from the local prior, then held fixed per pixel.
    #[allow(clippy::too_many_arguments)]
    pub async fn begin_band(
        &self,
        gpu: &Gpu,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        search: &Hull,
        init: &Plane<f32>,
        settings: &Settings,
        half: u32,
        min_score: f32,
        peak_margin: f32,
    ) -> anyhow::Result<crate::gpu::PendingRead<f32>> {
        anyhow::ensure!((2..=32).contains(&half), "coherent band half-width must be 2..32");
        anyhow::ensure!(
            neighbours.len() <= MAX_NEIGHBOURS && settings.best_of <= 4,
            "coherent band supports at most 8 neighbours and best_of 4"
        );
        let reference = &level[view];
        let pixels = reference.width * reference.height;
        anyhow::ensure!((init.width, init.height) == (reference.width, reference.height), "coherent prior size mismatch");
        let mut geometry = geometry(level, view, neighbours, search, settings);
        geometry.params.pass[2] = half;
        geometry.params.range[2] = peak_margin;
        geometry.params.range[3] = min_score;
        let budget = gpu.binding_budget() as usize;
        anyhow::ensure!(
            pixels * 16 <= budget && geometry.gray.len() * 4 <= budget && geometry.mask.len() * 4 <= budget,
            "coherent patch buffers exceed GPU binding budget; reduce pyramid size or disable native_level"
        );
        let planes = initial_planes(reference, init, init);
        let params = gpu.uniform("coherent band params", &geometry.params);
        let ref_gray = gpu.upload("coherent reference", &reference.gray.data);
        let ref_mask = gpu.upload("coherent mask", &mask_words(reference));
        let nbr_gray = gpu.upload("coherent neighbours", &geometry.gray);
        let nbr_mask = gpu.upload("coherent neighbour masks", &geometry.mask);
        let plane_buffer = gpu.upload("coherent initial planes", &planes);
        let depths = gpu.zeroed("coherent depths", (pixels * 4) as u64);
        gpu.run(
            self.band.as_ref().ok_or_else(|| anyhow::anyhow!("coherent band unavailable in research propagation solver"))?,
            &[&params, &ref_gray, &ref_mask, &nbr_gray, &nbr_mask, &self.hull, &plane_buffer, &depths],
            pixels as u64,
        )
        .await?;
        Ok(gpu.begin_read::<f32>(&depths, pixels))
    }

    /// Scores planes and runs independent coordinate iterations, or research-only
    /// red-black propagation when constructed with `new`. Zero iterations scores only.
    /// Returns the planes and their scores.
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        gpu: &Gpu,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        search: &Hull,
        planes: &[[f32; 4]],
        settings: &Settings,
    ) -> anyhow::Result<(Vec<[f32; 4]>, Vec<f32>)> {
        let reference = &level[view];
        let (w, h) = (reference.width, reference.height);
        let pixels = w * h;
        let geometry = geometry(level, view, neighbours, search, settings);
        let ref_gray = gpu.upload("pm reference grey", &reference.gray.data);
        let ref_mask = gpu.upload("pm reference mask", &mask_words(reference));
        let nbr_gray = gpu.upload("pm neighbour grey", &geometry.gray);
        let nbr_mask = gpu.upload("pm neighbour masks", &geometry.mask);
        let plane_buffer = gpu.upload("pm planes", planes);
        let scores = gpu.zeroed("pm scores", (pixels * 4) as u64);
        let bind = |params: &wgpu::Buffer| {
            [
                params.clone(),
                ref_gray.clone(),
                ref_mask.clone(),
                nbr_gray.clone(),
                nbr_mask.clone(),
                self.hull.clone(),
                plane_buffer.clone(),
                scores.clone(),
            ]
        };
        let params = gpu.uniform("pm params", &geometry.params);
        let buffers = bind(&params);
        gpu.run(&self.evaluate, &buffers.iter().collect::<Vec<_>>(), pixels as u64).await?;
        let half = (w.div_ceil(2) * h) as u64;
        for iteration in 0..settings.iterations {
            for colour in 0..if self.research { 2u32 } else { 1u32 } {
                let mut p = geometry.params;
                let shrink = 0.5f32.powi(iteration as i32);
                p.pass[2] = colour;
                p.pass[3] = settings.seed.wrapping_add(iteration * 2 + colour).wrapping_mul(2654435761);
                p.gates[2] = settings.inverse_step * shrink;
                p.gates[3] = settings.normal_step * shrink;
                let params = gpu.uniform("pm params", &p);
                let buffers = bind(&params);
                gpu.run(&self.sweep, &buffers.iter().collect::<Vec<_>>(), if self.research { half } else { pixels as u64 }).await?;
            }
        }
        let planes_read = gpu.begin_read::<[f32; 4]>(&plane_buffer, pixels);
        let scores_read = gpu.begin_read::<f32>(&scores, pixels);
        let planes = gpu.finish_read(planes_read).await?;
        let scores = gpu.finish_read(scores_read).await?;
        Ok((planes, scores))
    }
}

/// Depth map of planes whose score reaches `min_score`.
pub fn depth_of(planes: &[[f32; 4]], scores: &[f32], width: usize, height: usize, min_score: f32) -> Plane<f32> {
    let data = planes.iter().zip(scores).map(|(plane, &s)| if s >= min_score && plane[0] > 0.0 { plane[0] } else { 0.0 }).collect();
    Plane { width, height, data }
}

/// A unit normal in 32 bits (octahedral, two 16-bit coordinates); 0 for none.
pub fn pack_normal(n: [f32; 3]) -> u32 {
    let l1 = n[0].abs() + n[1].abs() + n[2].abs();
    if !(l1 > 0.0) {
        return 0;
    }
    let (mut u, mut v) = (n[0] / l1, n[1] / l1);
    if n[2] < 0.0 {
        let (pu, pv) = (u, v);
        u = (1.0 - pv.abs()) * if pu >= 0.0 { 1.0 } else { -1.0 };
        v = (1.0 - pu.abs()) * if pv >= 0.0 { 1.0 } else { -1.0 };
    }
    let q = |c: f32| ((c.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65534.0).round() as u32 + 1;
    q(u) << 16 | q(v)
}

/// [`pack_normal`] undone; `None` for 0.
pub fn unpack_normal(word: u32) -> Option<[f32; 3]> {
    if word == 0 {
        return None;
    }
    let d = |c: u32| ((c - 1) as f32 / 65534.0 - 0.5) * 2.0;
    let (mut u, mut v) = (d(word >> 16), d(word & 0xffff));
    let z = 1.0 - u.abs() - v.abs();
    if z < 0.0 {
        let (pu, pv) = (u, v);
        u = (1.0 - pv.abs()) * if pu >= 0.0 { 1.0 } else { -1.0 };
        v = (1.0 - pu.abs()) * if pv >= 0.0 { 1.0 } else { -1.0 };
    }
    let l = (u * u + v * v + z * z).sqrt();
    Some([u / l, v / l, z / l])
}

/// `Stereo.consistent` with planes: a neighbour's depth at the exact projection is taken from the
/// plane of the pixel it lands in (depth and normal) instead of that pixel's depth, and with
/// `min_cos` above -1 the two normals must also agree to that cosine.
#[allow(clippy::too_many_arguments)]
pub fn consistent_planes(
    level: &[LevelView],
    depths: &[Plane<f32>],
    normals: &[Vec<u32>],
    neighbours: &[Vec<usize>],
    tolerance: f64,
    minimum: i64,
    min_cos: f32,
) -> Vec<Plane<f32>> {
    let tolerance = tolerance as f32;
    let to_world = |view: &LevelView, n: [f32; 3]| {
        let r = &view.camera.rotation;
        [0, 1, 2].map(|a| r[0][a] * n[0] + r[1][a] * n[1] + r[2][a] * n[2])
    };
    crate::inputs::parallel_map(level.len(), |i| {
        let (view, depth) = (&level[i], &depths[i]);
        let mut out = Plane::<f32>::new(depth.width, depth.height);
        for y in 0..depth.height {
            for x in 0..depth.width {
                let p = y * depth.width + x;
                let d = depth.data[p];
                if d <= 0.0 {
                    continue;
                }
                let world = view.unproject(x, y, d);
                let own = unpack_normal(normals[i][p]).map(|n| to_world(view, n));
                let mut votes = 0i64;
                for &j in &neighbours[i] {
                    let other_view = &level[j];
                    let (xy, z) = other_view.project(world);
                    let (qx, qy) = (xy[0].round_ties_even(), xy[1].round_ties_even());
                    if !(qx >= 0.0 && qy >= 0.0 && qx < other_view.width as f32 && qy < other_view.height as f32) {
                        continue;
                    }
                    let q = qy as usize * other_view.width + qx as usize;
                    let other = depths[j].data[q];
                    if other <= 0.0 {
                        continue;
                    }
                    let k = other_view.camera.k;
                    let mut at = other;
                    let theirs = unpack_normal(normals[j][q]);
                    if let Some(n) = theirs {
                        let ray = |px: f32, py: f32| [(px + 0.5 - k[2]) / k[0], (py + 0.5 - k[3]) / k[1], 1.0f32];
                        let (rq, rx) = (ray(qx, qy), ray(xy[0], xy[1]));
                        let along = n[0] * rx[0] + n[1] * rx[1] + n[2] * rx[2];
                        if along < -1e-6 {
                            let offset = other * (n[0] * rq[0] + n[1] * rq[1] + n[2] * rq[2]);
                            let candidate = offset / along;
                            // Only a nearby correction: grazing planes would extrapolate far.
                            if (candidate - other).abs() <= 4.0 * tolerance * other {
                                at = candidate;
                            }
                        }
                    }
                    if (at - z).abs() > tolerance * z {
                        continue;
                    }
                    if min_cos > -1.0 {
                        if let (Some(a), Some(b)) = (own, theirs.map(|n| to_world(other_view, n))) {
                            if a[0] * b[0] + a[1] * b[1] + a[2] * b[2] < min_cos {
                                continue;
                            }
                        }
                    }
                    votes += 1;
                }
                if votes >= minimum {
                    out.data[p] = d;
                }
            }
        }
        out
    })
}

/// Scalar implementation of the kernel's scoring, for tests.
pub mod reference {
    use super::*;

    const INVALID_SAMPLE: f32 = -1e30;

    pub struct Scorer<'a> {
        level: &'a [LevelView],
        view: usize,
        params: Params,
        geometry: Geometry,
        search: &'a Hull,
    }

    impl<'a> Scorer<'a> {
        pub fn new(level: &'a [LevelView], view: usize, neighbours: &[usize], search: &'a Hull, settings: &Settings) -> Self {
            let geometry = geometry(level, view, neighbours, search, settings);
            Scorer { level, view, params: geometry.params, geometry, search }
        }

        fn ray(&self, x: f32, y: f32) -> [f32; 3] {
            let k = self.params.ref_k;
            [(x + 0.5 - k[2]) / k[0], (y + 0.5 - k[3]) / k[1], 1.0]
        }

        fn in_mask(&self, x: i32, y: i32) -> bool {
            let r = &self.level[self.view];
            x >= 0 && y >= 0 && (x as usize) < r.width && (y as usize) < r.height && r.mask.data[y as usize * r.width + x as usize] != 0
        }

        fn sample(&self, j: usize, point: [f32; 3]) -> f32 {
            let n = &self.params.nbr[j];
            let dot = |row: &[f32; 4]| row[0] * point[0] + row[1] * point[1] + row[2] * point[2] + row[3];
            let (qx, qy, qz) = (dot(&n.r0), dot(&n.r1), dot(&n.r2));
            if !(qz > 0.0) {
                return INVALID_SAMPLE;
            }
            let (w, h) = (n.extent[0] as i32, n.extent[1] as i32);
            let ix = qx / qz * n.k[0] + n.k[2] - 0.5;
            let iy = qy / qz * n.k[1] + n.k[3] - 0.5;
            if !(ix > -2.0 && iy > -2.0 && ix < w as f32 + 1.0 && iy < h as f32 + 1.0) {
                return INVALID_SAMPLE;
            }
            let other = &self.geometry;
            let texel = |x: i32, y: i32| -> (f32, f32) {
                if x < 0 || y < 0 || x >= w || y >= h {
                    return (0.0, 0.0);
                }
                let stride = (w as usize).div_ceil(32);
                let word = other.mask[n.extent[3] as usize + y as usize * stride + x as usize / 32];
                ((word >> (x as usize % 32) & 1) as f32, other.gray[n.extent[2] as usize + y as usize * w as usize + x as usize])
            };
            let (x0f, y0f) = (ix.floor(), iy.floor());
            let (x0, y0) = (x0f as i32, y0f as i32);
            let weights = [
                (x0f + 1.0 - ix) * (y0f + 1.0 - iy),
                (ix - x0f) * (y0f + 1.0 - iy),
                (x0f + 1.0 - ix) * (iy - y0f),
                (ix - x0f) * (iy - y0f),
            ];
            let corners = [texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1)];
            let coverage: f32 = corners.iter().zip(weights).map(|(c, w)| c.0 * w).sum();
            if !(coverage >= 0.999) {
                return INVALID_SAMPLE;
            }
            corners.iter().zip(weights).map(|(c, w)| c.1 * w).sum()
        }

        fn in_hull(&self, point: [f32; 3]) -> bool {
            let p = &self.params;
            let row = |r: &[f32; 4]| r[0] * point[0] + r[1] * point[1] + r[2] * point[2] + r[3];
            self.search.contains([row(&p.world_r0), row(&p.world_r1), row(&p.world_r2)])
        }

        /// The kernel's `score`.
        pub fn score(&self, x: i32, y: i32, plane: [f32; 4]) -> f32 {
            let p = &self.params;
            let (depth, normal) = (plane[0], [plane[1], plane[2], plane[3]]);
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let r = self.ray(x as f32, y as f32);
            if !(depth > 0.0) || !(dot(normal, r) < 0.0) {
                return -2.0;
            }
            let inverse = 1.0 / depth;
            if inverse < p.range[0] || inverse > p.range[1] {
                return -2.0;
            }
            let centre = r.map(|v| v * depth);
            if !self.in_hull(centre) {
                return -2.0;
            }
            let offset = dot(normal, centre);
            let (radius, stride) = (p.size[3] as i32, p.pass[0] as i32);
            let side = ((2 * p.size[3]) / p.pass[0] + 1) as f32;
            let scale = 1.0 / (side * side);
            let reference = &self.level[self.view];
            let mut best = [-2.0f32; 4];
            for j in 0..p.size[2] as usize {
                let mut sums = [0f32; 6];
                let mut dy = -radius;
                while dy <= radius {
                    let mut dx = -radius;
                    while dx <= radius {
                        let (qx, qy) = (x + dx, y + dy);
                        dx += stride;
                        if !self.in_mask(qx, qy) {
                            continue;
                        }
                        let rq = self.ray(qx as f32, qy as f32);
                        let along = dot(normal, rq);
                        if !(along < -1e-6) {
                            continue;
                        }
                        let t = offset / along;
                        let value = self.sample(j, rq.map(|v| v * t));
                        if value == INVALID_SAMPLE {
                            continue;
                        }
                        let a = reference.gray.data[qy as usize * reference.width + qx as usize] - 0.5;
                        let b = value - 0.5;
                        for (sum, term) in sums.iter_mut().zip([1.0, a, b, a * a, b * b, a * b]) {
                            *sum += term;
                        }
                    }
                    dy += stride;
                }
                let fill = sums[0] * scale;
                let d = fill.max(1e-6);
                let (mr, mt) = (sums[1] * scale / d, sums[2] * scale / d);
                let (vr, vt) = ((sums[3] * scale / d - mr * mr).max(0.0), (sums[4] * scale / d - mt * mt).max(0.0));
                let mut s = -2.0f32;
                if fill >= p.gates[1] && vr >= p.gates[0] && vt >= p.gates[0] {
                    s = ((sums[5] * scale / d - mr * mt) / (vr * vt + 1e-12).sqrt()).clamp(-1.0, 1.0);
                }
                for rank in best.iter_mut() {
                    let lower = rank.min(s);
                    *rank = rank.max(s);
                    s = lower;
                }
            }
            let valid: Vec<f32> = best[..p.pass[1] as usize].iter().copied().filter(|&s| s > -1.5).collect();
            if valid.len() < 2 {
                -2.0
            } else {
                valid.iter().sum::<f32>() / valid.len() as f32
            }
        }

        /// The kernel's `evaluate` over all pixels.
        pub fn evaluate(&self, planes: &[[f32; 4]]) -> Vec<f32> {
            let r = &self.level[self.view];
            (0..r.width * r.height)
                .map(|p| {
                    let (x, y) = ((p % r.width) as i32, (p / r.width) as i32);
                    if self.in_mask(x, y) {
                        self.score(x, y, planes[p])
                    } else {
                        -2.0
                    }
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::build_hull;
    use crate::inputs::Inputs;
    use crate::stereo::level::build_level;
    use crate::stereo::{options, synthetic};

    #[test]
    fn coherent_band_matches_scalar_candidate_plane_search() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = synthetic::temporary("coherent-band", 24, 96).unwrap();
        let overrides = synthetic::SMALL.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let level = build_level(&inputs, 96);
        let view = 5;
        let neighbours = inputs.neighbours(view, config.neighbours as usize, &config).unwrap();
        let init = synthetic::sphere_depth(&level[view]);
        let search = Hull::from_flags(&vec![true; 64 * 64 * 64], [64; 3], [-1.6; 3], 0.05);
        let gpu = Gpu::new().unwrap();
        let solver = crate::gpu::block_on(Planes::independent(&gpu, &search)).unwrap();
        let settings = Settings {
            radius: 3,
            stride: 1,
            best_of: 2,
            min_variance: 0.0001,
            window_fill: 0.6,
            iterations: 0,
            inverse_step: 0.01,
            normal_step: 0.0,
            inverse_range: (0.001, 10.0),
            seed: 0,
        };
        let pending =
            crate::gpu::block_on(solver.begin_band(&gpu, &level, view, &neighbours, &search, &init, &settings, 4, 0.55, 0.0)).unwrap();
        let actual = crate::gpu::block_on(gpu.finish_read(pending)).unwrap();
        let planes = initial_planes(&level[view], &init, &init);
        let scorer = reference::Scorer::new(&level, view, &neighbours, &search, &settings);
        let mut compared = 0;
        for p in (0..actual.len()).step_by(11) {
            if planes[p][0] <= 0.0 || level[view].mask.data[p] == 0 {
                assert_eq!(actual[p], 0.0);
                continue;
            }
            let (x, y) = ((p % init.width) as i32, (p / init.width) as i32);
            let base = 1.0 / planes[p][0];
            let costs = (0..9)
                .map(|h| {
                    let mut plane = planes[p];
                    plane[0] = 1.0 / (base + (h as f32 - 4.0) * 0.01);
                    scorer.score(x, y, plane)
                })
                .collect::<Vec<_>>();
            let index = (0..9).max_by(|&a, &b| costs[a].total_cmp(&costs[b]).then(b.cmp(&a))).unwrap();
            let expected = if index > 0 && index < 8 && costs[index] >= 0.55 && costs[index - 1] > -1.5 && costs[index + 1] > -1.5 {
                let curve = costs[index - 1] - 2.0 * costs[index] + costs[index + 1];
                let offset = if curve < -1e-6 { (0.5 * (costs[index - 1] - costs[index + 1]) / curve).clamp(-0.5, 0.5) } else { 0.0 };
                1.0 / (base + (index as f32 - 4.0 + offset) * 0.01)
            } else {
                0.0
            };
            assert!((actual[p] - expected).abs() < 0.002, "pixel {p}: GPU{} scalar{expected}", actual[p]);
            compared += (expected > 0.0) as usize;
        }
        assert!(compared > 100, "{compared} valid comparisons");
    }

    #[test]
    fn normals_pack_into_32_bits() {
        for n in [[0.0f32, 0.0, -1.0], [0.6, -0.8, 0.0], [0.3, 0.4, 0.866], [-0.5, -0.5, -0.707]] {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = n.map(|v| v / l);
            let back = unpack_normal(pack_normal(n)).unwrap();
            assert!((0..3).all(|a| (back[a] - n[a]).abs() < 1e-3), "{n:?} {back:?}");
        }
        assert_eq!(unpack_normal(pack_normal([0.0; 3])), None);
    }

    /// NCC is invariant to an additive brightness offset. In particular, valid
    /// negative samples after percentile stretching must not become missing data.
    #[test]
    fn negative_brightness_is_valid_matching_data() {
        let root = synthetic::temporary("patchmatch-negative", 24, 96).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let level = build_level(&inputs, 96);
        let mut dark = level.clone();
        for view in &mut dark {
            for value in &mut view.gray.data {
                *value -= 2.0;
            }
        }
        let view = 5;
        let neighbours = inputs.neighbours(view, config.neighbours as usize, &config).unwrap();
        let exact = synthetic::sphere_depth(&level[view]);
        let planes = initial_planes(&level[view], &exact, &exact);
        let search = Hull::from_flags(&vec![true; 64 * 64 * 64], [64; 3], [-1.6; 3], 0.05);
        let gpu = crate::gpu::tests_enabled().then(|| Gpu::new().unwrap());
        for radius in [3, 5] {
            let settings = Settings {
                radius,
                stride: 2,
                best_of: config.best_of as u32,
                min_variance: config.min_variance as f32,
                window_fill: config.window_fill as f32,
                iterations: 0,
                inverse_step: 0.01,
                normal_step: 0.3,
                inverse_range: (1e-3, 10.0),
                seed: 7,
            };
            let bright = reference::Scorer::new(&level, view, &neighbours, &search, &settings).evaluate(&planes);
            let shifted = reference::Scorer::new(&dark, view, &neighbours, &search, &settings).evaluate(&planes);
            let valid = bright.iter().filter(|&&v| v > -1.5).count();
            assert!(valid > 1000);
            let close = bright.iter().zip(&shifted).filter(|(&a, &b)| (a - b).abs() < 0.01).count();
            assert!(close * 100 >= bright.len() * 99, "brightness offset changed {}/{} scores", bright.len() - close, bright.len());
            if let Some(gpu) = &gpu {
                let pm = crate::gpu::block_on(Planes::new(gpu, &search)).unwrap();
                let (_, found) = crate::gpu::block_on(pm.run(gpu, &dark, view, &neighbours, &search, &planes, &settings)).unwrap();
                let close = bright.iter().zip(&found).filter(|(&a, &b)| (a - b).abs() < 0.01).count();
                assert!(close * 100 >= bright.len() * 99, "GPU rejected or changed valid negative samples: {close}/{}", bright.len());
            }
        }
    }

    /// Planes of the exact sphere score near 1, a plane displaced along the ray scores lower, and
    /// the GPU kernel agrees with the scalar implementation (`CRISPMESH_GPU_TESTS=1` for the GPU part).
    #[test]
    fn plane_scores_match_the_scalar_implementation() {
        let root = synthetic::temporary("patchmatch", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let level = build_level(&inputs, 128);
        let view = 5;
        let neighbours = inputs.neighbours(view, config.neighbours as usize, &config).unwrap();
        let exact = synthetic::sphere_depth(&level[view]);
        // A box around the sphere as the search hull, so that only the planes decide.
        let (side, voxel) = (64usize, 0.05f64);
        let origin = [-(side as f32) * voxel as f32 / 2.0; 3];
        let search = Hull::from_flags(&vec![true; side * side * side], [side; 3], origin, voxel);
        let settings = Settings {
            radius: 3,
            stride: 1,
            best_of: config.best_of as u32,
            min_variance: config.min_variance as f32,
            window_fill: config.window_fill as f32,
            iterations: 4,
            inverse_step: 0.01,
            normal_step: 0.3,
            inverse_range: (1e-3, 10.0),
            seed: 7,
        };
        let planes = initial_planes(&level[view], &exact, &exact);
        let scorer = reference::Scorer::new(&level, view, &neighbours, &search, &settings);
        let scores = scorer.evaluate(&planes);
        let valid: Vec<f32> = scores.iter().copied().filter(|&s| s > -1.5).collect();
        assert!(valid.len() > 2000, "{} scored pixels", valid.len());
        let mut sorted = valid.clone();
        sorted.sort_by(f32::total_cmp);
        let median = sorted[sorted.len() / 2];
        assert!(median > 0.9, "median score of the exact planes {median}");
        let shifted: Vec<[f32; 4]> = planes.iter().map(|p| [p[0] * 1.03, p[1], p[2], p[3]]).collect();
        let worse = scorer.evaluate(&shifted);
        let lower = scores.iter().zip(&worse).filter(|(&a, &b)| a > -1.5 && b < a).count();
        assert!(lower * 10 > valid.len() * 8, "{lower} of {} lower when shifted", valid.len());

        if !crate::gpu::tests_enabled() {
            return;
        }
        let gpu = Gpu::new().unwrap();
        let pm = crate::gpu::block_on(Planes::new(&gpu, &search)).unwrap();
        let none = Settings { iterations: 0, ..settings };
        let (_, found) = crate::gpu::block_on(pm.run(&gpu, &level, view, &neighbours, &search, &shifted, &none)).unwrap();
        let close = worse.iter().zip(&found).filter(|(&a, &b)| (a - b).abs() < 1e-3 || (a < -1.5 && b < -1.5)).count();
        println!("scalar median {median}; GPU and scalar agree at {close} of {}", worse.len());
        assert!(close * 1000 >= worse.len() * 995, "GPU and scalar scores agree at {close} of {}", worse.len());
        // Iterations from the shifted planes recover the sphere.
        let (planes_after, scores_after) =
            crate::gpu::block_on(pm.run(&gpu, &level, view, &neighbours, &search, &shifted, &settings)).unwrap();
        let (mut near, mut total) = (0, 0);
        for p in 0..planes_after.len() {
            if exact.data[p] > 0.0 && scores_after[p] > 0.5 {
                total += 1;
                near += ((planes_after[p][0] - exact.data[p]).abs() < 0.01 * exact.data[p]) as usize;
            }
        }
        println!("after the sweeps: {near} of {total} within 1 %");
        assert!(total > 2000 && near * 10 > total * 9, "{near} of {total} within 1 % after the sweeps");
        let _ = build_hull;
    }
}

#[cfg(test)]
mod independent_tests {
    use super::*;
    use crate::inputs::Inputs;
    use crate::stereo::{level::build_level, options, synthetic};

    #[test]
    fn independent_refinement_improves_scores_without_borrowing_neighbour_planes() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = synthetic::temporary("independent-planes", 24, 96).unwrap();
        let config = options::build(None, &synthetic::SMALL.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let level = build_level(&inputs, 96);
        let view = 5;
        let neighbours = inputs.neighbours(view, config.neighbours as usize, &config).unwrap();
        let exact = synthetic::sphere_depth(&level[view]);
        let search = Hull::from_flags(&vec![true; 64 * 64 * 64], [64; 3], [-1.6; 3], 0.05);
        let settings = Settings {
            radius: 3,
            stride: 1,
            best_of: config.best_of as u32,
            min_variance: config.min_variance as f32,
            window_fill: config.window_fill as f32,
            iterations: 6,
            inverse_step: 0.01,
            normal_step: 0.3,
            inverse_range: (1e-3, 10.0),
            seed: 7,
        };
        let planes: Vec<_> = initial_planes(&level[view], &exact, &exact)
            .into_iter()
            .map(|mut p| {
                p[0] *= 1.03;
                p
            })
            .collect();
        let gpu = Gpu::new().unwrap();
        let solver = crate::gpu::block_on(Planes::independent(&gpu, &search)).unwrap();
        let (_, initial) =
            crate::gpu::block_on(solver.run(&gpu, &level, view, &neighbours, &search, &planes, &Settings { iterations: 0, ..settings }))
                .unwrap();
        let (found, scores) = crate::gpu::block_on(solver.run(&gpu, &level, view, &neighbours, &search, &planes, &settings)).unwrap();
        let valid: Vec<_> = initial.iter().enumerate().filter_map(|(p, &s)| (s > 0.5 && exact.data[p] > 0.0).then_some(p)).collect();
        assert!(valid.len() > 1000);
        assert!(valid.iter().all(|&p| scores[p] >= initial[p]));
        let improved = valid.iter().filter(|&&p| scores[p] > initial[p] + 0.01).count();
        assert!(improved * 2 > valid.len(), "only {improved}/{} improved", valid.len());
        let mean_error = valid.iter().map(|&p| (found[p][0] / exact.data[p] - 1.0).abs()).sum::<f32>() / valid.len() as f32;
        assert!(mean_error < 0.02, "initial 3% depth error, refined {mean_error}");
        // Change every other starting plane drastically; the unchanged pixels must
        // be bit-identical, even beside those changed planes.
        let mut disturbed = planes.clone();
        for (p, plane) in disturbed.iter_mut().enumerate() {
            if p % 7 != 0 {
                plane[0] *= 1.2;
            }
        }
        let (again, again_scores) =
            crate::gpu::block_on(solver.run(&gpu, &level, view, &neighbours, &search, &disturbed, &settings)).unwrap();
        for &p in &valid {
            if p % 7 == 0 {
                assert_eq!(found[p], again[p]);
                assert_eq!(scores[p], again_scores[p]);
            }
        }
        println!("Independent planes: {improved}/{} scores improved; mean relative depth error {mean_error}", valid.len());
    }
}
