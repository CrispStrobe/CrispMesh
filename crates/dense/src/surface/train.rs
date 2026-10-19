//! Training of the neural surface on a packed scene (`scripts/neural_surface/pack_scene.py`: photos and masks
//! cropped around the object, cameras, our silhouette hull as points with normals), as the MLX prototype does:
//! start on the hull, then 20 000 steps of rendering against the photos; the mesh is the network's zero surface.

use std::path::Path;

use anyhow::{anyhow, bail, ensure, Context, Result};
use rayon::prelude::*;

use super::*;
use crate::mesh::cubes;
use crate::mesh::gaussian::gaussian_filter;

/// Settings; the defaults are the prototype's (`neus_facto_mlx.py`).
#[derive(Clone, Debug)]
pub struct Settings {
    pub steps: u32,
    /// End early on the schedule of `steps` (0: no).
    pub stop: u32,
    pub rays: u32,
    pub parts: u32,
    pub hull_steps: u32,
    pub sampling: Sampling,
    pub hidden: u32,
    pub features: u32,
    pub table_log2: u32,
    pub rate: f32,
    pub rate_end: f32,
    pub warm_up: u32,
    pub anneal_end: u32,
    pub eikonal: f32,
    pub mask: f32,
    pub variance: f32,
    pub mesh: u32,
    pub snapshot: u32,
    pub log_every: u32,
    pub seed: u64,
}

impl Settings {
    /// The settings of a quality level (standard: the reference run's).
    pub fn for_quality(quality: super::options::Quality) -> Self {
        use super::options::Quality;
        let standard = Settings::default();
        match quality {
            Quality::Fast => Settings { steps: 6000, hull_steps: 400, mesh: 384, ..standard },
            Quality::Standard => standard,
            Quality::Best => Settings { steps: 30000, rays: 2048, parts: 8, mesh: 768, ..standard },
        }
    }

    /// Applies one `NAME=VALUE`; false if the name is not a setting of the neural surface.
    pub fn set(&mut self, name: &str, value: &str) -> Result<bool> {
        use super::options::{number, whole};
        let int = || whole(name, value);
        match name {
            "steps" => self.steps = int()?,
            "stop" => self.stop = int()?,
            "rays" => self.rays = int()?,
            "parts" => self.parts = int()?,
            "hull" => self.hull_steps = int()?,
            "band" => self.sampling.band = number(name, value)?,
            "coarse" => self.sampling.coarse = int()?,
            "added" => self.sampling.added = int()?,
            "width" => self.hidden = int()?,
            "features" => self.features = int()?,
            "table" => self.table_log2 = int()?,
            "mesh" => self.mesh = int()?,
            "snapshot" => self.snapshot = int()?,
            "log" => self.log_every = int()?,
            "seed" => self.seed = int()? as u64,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            steps: 20000,
            stop: 0,
            rays: 1024,
            parts: 4,
            hull_steps: 600,
            sampling: Sampling { coarse: 32, added: 8, rounds: 4, band: 0.05 },
            hidden: 64,
            features: 64,
            table_log2: 19,
            rate: 5e-4,
            rate_end: 0.05,
            warm_up: 500,
            anneal_end: 50000,
            eikonal: 0.1,
            mask: 0.1,
            variance: 0.3,
            mesh: 512,
            snapshot: 0,
            log_every: 100,
            seed: 0,
        }
    }
}

/// xoshiro256** seeded through SplitMix64.
pub struct Random([u64; 4]);

impl Random {
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        Random(std::array::from_fn(|_| {
            x = x.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^ (z >> 31)
        }))
    }
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }
    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn below(&mut self, n: usize) -> usize {
        (((self.next_u64() >> 32) * n as u64) >> 32) as usize
    }
    pub fn normal(&mut self) -> f64 {
        let (u, v) = (self.uniform().max(1e-300), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

pub struct View {
    pub width: usize,
    pub height: usize,
    /// RGB, black outside the mask.
    pub image: Vec<u8>,
    pub mask: Vec<bool>,
    pub intrinsics: [f64; 4],
    pub rotation: [[f64; 3]; 3],
    /// Camera centre in the unit-sphere frame.
    pub origin: [f64; 3],
}

pub struct Scene {
    pub views: Vec<View>,
    pub centre: [f64; 3],
    /// Scene units per world unit: 1 / (1.15 radius).
    pub unit: f64,
    /// Hull points and outward normals in the unit-sphere frame.
    pub hull_points: Vec<[f64; 3]>,
    pub hull_normals: Vec<[f64; 3]>,
}

fn vec3(value: &serde_json::Value) -> Result<[f64; 3]> {
    let a = value.as_array().ok_or_else(|| anyhow!("expected three numbers"))?;
    ensure!(a.len() == 3, "expected three numbers");
    Ok(std::array::from_fn(|i| a[i].as_f64().unwrap_or(f64::NAN)))
}

/// Reads a pack: scene.json, images/, masks/, hull_points.npy, hull_normals.npy.
pub fn load_pack(pack: &Path) -> Result<Scene> {
    let scene: serde_json::Value = serde_json::from_slice(&crate::storage::read(pack.join("scene.json")).context("scene.json")?)?;
    let centre = vec3(&scene["centre"])?;
    let unit = 1.0 / (1.15 * scene["radius"].as_f64().ok_or_else(|| anyhow!("radius"))?);
    let frames = scene["frames"].as_array().ok_or_else(|| anyhow!("frames"))?;
    let views = frames
        .par_iter()
        .map(|frame| -> Result<View> {
            let path = |key: &str| pack.join(frame[key].as_str().unwrap_or_default());
            let rgb = crate::storage::open_image(path("image")).with_context(|| format!("{}", path("image").display()))?.to_rgb8();
            let grey = crate::storage::open_image(path("mask")).with_context(|| format!("{}", path("mask").display()))?.to_luma8();
            ensure!(rgb.dimensions() == grey.dimensions(), "photo and mask sizes differ");
            let (width, height) = (rgb.width() as usize, rgb.height() as usize);
            let mask: Vec<bool> = grey.into_raw().into_iter().map(|v| v > 127).collect();
            let mut image = rgb.into_raw();
            for (i, inside) in mask.iter().enumerate() {
                if !inside {
                    image[i * 3..i * 3 + 3].fill(0);
                }
            }
            let rows = frame["rotation"].as_array().ok_or_else(|| anyhow!("rotation"))?;
            ensure!(rows.len() == 3, "rotation");
            let rotation = [vec3(&rows[0])?, vec3(&rows[1])?, vec3(&rows[2])?];
            let t = vec3(&frame["translation"])?;
            let number = |key: &str| frame[key].as_f64().ok_or_else(|| anyhow!("{key}"));
            // Camera centre -R^T t, in the unit-sphere frame.
            let origin = std::array::from_fn(|i| (-(0..3).map(|j| rotation[j][i] * t[j]).sum::<f64>() - centre[i]) * unit);
            Ok(View {
                width,
                height,
                image,
                mask,
                intrinsics: [number("fx")?, number("fy")?, number("cx")?, number("cy")?],
                rotation,
                origin,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let read = |name: &str| -> Result<Vec<[f64; 3]>> {
        let array = crate::npz::read_npy(&pack.join(name)).with_context(|| name.to_string())?;
        ensure!(array.shape.len() == 2 && array.shape[1] == 3, "{name}: expected N x 3");
        Ok(array.to_f64().chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
    };
    let hull_points = read("hull_points.npy")?.into_iter().map(|p| std::array::from_fn(|i| (p[i] - centre[i]) * unit)).collect();
    Ok(Scene { views, centre, unit, hull_points, hull_normals: read("hull_normals.npy")? })
}

/// Maps over the views in parallel, except in a browser: there the photos come through a file source that only the
/// main thread can reach, so they are read one after the other.
fn each_view<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    #[cfg(not(target_arch = "wasm32"))]
    return items.par_iter().map(f).collect();
    #[cfg(target_arch = "wasm32")]
    return items.iter().map(f).collect();
}

struct PhotoWindow {
    image: Vec<u8>,
    mask: Vec<bool>,
    origin: [usize; 2],
    scale: [f64; 2],
    area_divisor: Option<usize>,
}

impl PhotoWindow {
    fn intrinsics(&self, k: [f64; 4]) -> [f64; 4] {
        let [x, y] = self.origin.map(|x| x as f64);
        if let Some(div) = self.area_divisor {
            let d = div as f64;
            [k[0] / d, k[1] / d, (k[2] - x) / d, (k[3] - y) / d]
        } else {
            let [sx, sy] = self.scale;
            [k[0] * sx, k[1] * sy, (k[2] - x) * sx, (k[3] - y) * sy]
        }
    }
}

/// Fit each observed crop to the common training canvas without inventing
/// background outside the photo. Existing integer-area reduction is unchanged
/// when the common source window fits; smaller crops use their valid extent.
fn photo_window(rgb: image::RgbImage, mask: image::GrayImage, bounds: [usize; 4], cw: usize, ch: usize, div: usize) -> PhotoWindow {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let (sw, sh) = (cw.min(w), ch.min(h));
    let x0 = (((bounds[0] + bounds[2]) / 2) as i64 - (sw / 2) as i64).clamp(0, (w - sw) as i64) as usize;
    let y0 = (((bounds[1] + bounds[3]) / 2) as i64 - (sh / 2) as i64).clamp(0, (h - sh) as i64) as usize;
    let (width, height) = (cw / div, ch / div);
    let mut image = vec![0u8; width * height * 3];
    let mut inside = vec![false; width * height];
    if sw == cw && sh == ch {
        // Preserve the established area-average pixels and mask threshold.
        let (rgb, mask) = (rgb.into_raw(), mask.into_raw());
        let block = (div * div) as f64;
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0f64; 4];
                for dy in 0..div {
                    for dx in 0..div {
                        let pixel = (y0 + y * div + dy) * w + x0 + x * div + dx;
                        for k in 0..3 {
                            sum[k] += rgb[pixel * 3 + k] as f64;
                        }
                        sum[3] += mask[pixel] as f64;
                    }
                }
                let m = sum[3] / block >= 127.5;
                inside[y * width + x] = m;
                for k in 0..3 {
                    image[(y * width + x) * 3 + k] = if m { (sum[k] / block).round() as u8 } else { 0 };
                }
            }
        }
    } else {
        // Resize only existing pixels. Padding would incorrectly supervise
        // unknown pixels beyond a clipped photo as empty background.
        use image::imageops::{crop_imm, resize, FilterType};
        let rgb = resize(
            &crop_imm(&rgb, x0 as u32, y0 as u32, sw as u32, sh as u32).to_image(),
            width as u32,
            height as u32,
            FilterType::Triangle,
        );
        let mask = resize(
            &crop_imm(&mask, x0 as u32, y0 as u32, sw as u32, sh as u32).to_image(),
            width as u32,
            height as u32,
            FilterType::Triangle,
        );
        for (i, (color, mask)) in rgb.pixels().zip(mask.pixels()).enumerate() {
            inside[i] = mask[0] >= 128;
            if inside[i] {
                image[i * 3..i * 3 + 3].copy_from_slice(&color.0);
            }
        }
    }
    PhotoWindow {
        image,
        mask: inside,
        origin: [x0, y0],
        scale: [width as f64 / sw as f64, height as f64 / sh as f64],
        area_divisor: (sw == cw && sh == ch).then_some(div),
    }
}

/// A scene straight from an inputs directory (cameras.json with each view's original photo `texture_image`, else
/// `image`, and `mask`; `photos` and `masks` replace their folders; `div` 0: chosen from `crop`), as `pack_scene.py` packs it: every view cropped to one
/// common window around its mask (48 pixels of margin, a multiple of 16 times `div`), reduced by `div` (block
/// means, as OpenCV's area resize for a whole factor; masks kept where the mean is at least 128), with the crop's
/// intrinsics. A smaller input crop is resampled over its valid extent onto the
/// common canvas, with separate horizontal/vertical camera scales. No unknown
/// pixels beyond the photo become background. There are 100 000 area-weighted samples of the hull mesh with their triangles' normals.
pub fn scene_from_inputs(
    inputs: &Path,
    photos: Option<&Path>,
    masks: Option<&Path>,
    hull_mesh: &Path,
    div: usize,
    crop: usize,
    seed: u64,
) -> Result<Scene> {
    // div 0: the whole factor that brings the crop window to at most `crop` pixels across.
    let cameras: serde_json::Value = serde_json::from_slice(&crate::storage::read(inputs.join("cameras.json")).context("cameras.json")?)?;
    let views = cameras["views"].as_array().ok_or_else(|| anyhow!("cameras.json has no views"))?;
    let mask_path = |view: &serde_json::Value| -> std::path::PathBuf {
        let name = view["name"].as_str().unwrap_or_default();
        match masks {
            Some(dir) => dir.join(format!("{name}.png")),
            None => inputs.join(view["mask"].as_str().unwrap_or_default()),
        }
    };
    let read_mask = |view: &serde_json::Value| -> Result<image::GrayImage> {
        let path = mask_path(view);
        Ok(crate::storage::open_image(&path).with_context(|| path.display().to_string())?.to_luma8())
    };
    // The common window: the largest mask box plus margins, in steps of 16 div.
    let boxes = each_view(views, |view| -> Result<[usize; 4]> {
        let mask = read_mask(view)?;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (x, y, value) in mask.enumerate_pixels() {
            if value[0] > 127 {
                let (x, y) = (x as usize, y as usize);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        ensure!(x1 > 0, "empty mask for {}", view["name"]);
        Ok([x0, y0, x1, y1])
    })
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    let margin = 48;
    let widest = boxes.iter().map(|b| (b[2] - b[0]).max(b[3] - b[1])).max().unwrap_or(0) + 2 * margin;
    let div = if div == 0 { widest.div_ceil(crop.max(1)).max(1) } else { div };
    let unit = 16 * div;
    let cw = (boxes.iter().map(|b| b[2] - b[0]).max().unwrap_or(0) + 2 * margin).div_ceil(unit) * unit;
    let ch = (boxes.iter().map(|b| b[3] - b[1]).max().unwrap_or(0) + 2 * margin).div_ceil(unit) * unit;
    let (width, height) = (cw / div, ch / div);
    let triangles = crate::stl::read_binary(hull_mesh)?.0;
    let area = |t: &crate::stl::Triangle| {
        let (a, b, c) = (t[0].map(f64::from), t[1].map(f64::from), t[2].map(f64::from));
        let n = cross(sub(b, a), sub(c, a));
        (n, (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt())
    };
    let mut cumulative = Vec::with_capacity(triangles.len());
    let mut total = 0.0;
    for t in &triangles {
        total += area(t).1;
        cumulative.push(total);
    }
    ensure!(total > 0.0, "the hull mesh has no area");
    let mut random = Random::new(seed ^ 0x5eed);
    let mut points = Vec::with_capacity(100_000);
    let mut normals = Vec::with_capacity(100_000);
    for _ in 0..100_000 {
        let pick = random.uniform() * total;
        let t = &triangles[cumulative.partition_point(|c| *c < pick).min(triangles.len() - 1)];
        let (mut u, mut v) = (random.uniform(), random.uniform());
        if u + v > 1.0 {
            (u, v) = (1.0 - u, 1.0 - v);
        }
        let (a, b, c) = (t[0].map(f64::from), t[1].map(f64::from), t[2].map(f64::from));
        points.push(std::array::from_fn::<f64, 3, _>(|i| a[i] + u * (b[i] - a[i]) + v * (c[i] - a[i])));
        let (n, length) = area(t);
        normals.push(n.map(|x| x / length.max(1e-30)));
    }
    let low: [f64; 3] = std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
    let high: [f64; 3] = std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max));
    let centre: [f64; 3] = std::array::from_fn(|i| 0.5 * (low[i] + high[i]));
    let radius = points.iter().map(|p| (0..3).map(|i| (p[i] - centre[i]).powi(2)).sum::<f64>().sqrt()).fold(0.0, f64::max);
    let unit_scale = 1.0 / (1.15 * radius);
    let pairs: Vec<(&serde_json::Value, &[usize; 4])> = views.iter().zip(&boxes).collect();
    let views = each_view(&pairs, |(view, b)| -> Result<View> {
        let (view, b) = (*view, *b);
        let photo = Path::new(view["texture_image"].as_str().or_else(|| view["image"].as_str()).unwrap_or_default());
        let photo = match photos {
            Some(dir) => dir.join(photo.file_name().unwrap_or_default()),
            None => inputs.join(photo),
        };
        let rgb = crate::storage::open_image(&photo).with_context(|| photo.display().to_string())?.to_rgb8();
        let mask = read_mask(view)?;
        ensure!(rgb.dimensions() == mask.dimensions(), "photo and mask sizes differ for {}", view["name"]);
        let window = photo_window(rgb, mask, *b, cw, ch, div);
        let k = view["k"].as_array().ok_or_else(|| anyhow!("view without k"))?;
        let k: Vec<f64> = k.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect();
        ensure!(k.len() == 4, "k must be fx, fy, cx, cy");
        let rows = view["rotation"].as_array().ok_or_else(|| anyhow!("rotation"))?;
        ensure!(rows.len() == 3, "rotation");
        let rotation = [vec3(&rows[0])?, vec3(&rows[1])?, vec3(&rows[2])?];
        let t = vec3(&view["translation"])?;
        let origin = std::array::from_fn(|i| (-(0..3).map(|j| rotation[j][i] * t[j]).sum::<f64>() - centre[i]) * unit_scale);
        let intrinsics = window.intrinsics([k[0], k[1], k[2], k[3]]);
        Ok(View { width, height, image: window.image, mask: window.mask, intrinsics, rotation, origin })
    })
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    let hull_points = points.iter().map(|p| std::array::from_fn(|i| (p[i] - centre[i]) * unit_scale)).collect();
    Ok(Scene { views, centre, unit: unit_scale, hull_points, hull_normals: normals })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Exact nearest neighbours (a k-d tree over the points).
pub(crate) struct Tree {
    points: Vec<[f64; 3]>,
    /// Point indices in tree order; node i of a range is its median, split on axis depth % 3.
    order: Vec<u32>,
}

impl Tree {
    pub(crate) fn new(points: &[[f64; 3]]) -> Self {
        let mut order: Vec<u32> = (0..points.len() as u32).collect();
        fn build(points: &[[f64; 3]], order: &mut [u32], depth: usize) {
            if order.len() <= 1 {
                return;
            }
            let axis = depth % 3;
            let middle = order.len() / 2;
            order.select_nth_unstable_by(middle, |a, b| points[*a as usize][axis].total_cmp(&points[*b as usize][axis]));
            let (left, right) = order.split_at_mut(middle);
            build(points, left, depth + 1);
            build(points, &mut right[1..], depth + 1);
        }
        build(points, &mut order, 0);
        Tree { points: points.to_vec(), order }
    }

    /// The nearest point and its squared distance. The far side of a split is searched only when the distance to
    /// that side's region (accumulated per axis along the path) is below the best so far.
    pub(crate) fn nearest(&self, q: [f64; 3]) -> (usize, f64) {
        fn search(tree: &Tree, range: &[u32], depth: usize, q: &[f64; 3], offsets: &mut [f64; 3], region: f64, best: &mut (usize, f64)) {
            if range.is_empty() {
                return;
            }
            let middle = range.len() / 2;
            let index = range[middle] as usize;
            let p = tree.points[index];
            let d2 = (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum::<f64>();
            if d2 < best.1 {
                *best = (index, d2);
            }
            let axis = depth % 3;
            let delta = q[axis] - p[axis];
            let (near, far) = if delta < 0.0 { (&range[..middle], &range[middle + 1..]) } else { (&range[middle + 1..], &range[..middle]) };
            search(tree, near, depth + 1, q, offsets, region, best);
            let old = offsets[axis];
            let far_region = region - old * old + delta * delta;
            if far_region < best.1 {
                offsets[axis] = delta;
                search(tree, far, depth + 1, q, offsets, far_region, best);
                offsets[axis] = old;
            }
        }
        let mut best = (0, f64::INFINITY);
        search(self, &self.order, 0, &q, &mut [0.0; 3], 0.0, &mut best);
        best
    }
}

impl Tree {
    /// The `k` nearest points' squared distances, nearest first (the query point itself included if it is one).
    pub(crate) fn nearest_k(&self, q: [f64; 3], k: usize) -> Vec<f64> {
        #[allow(clippy::too_many_arguments)]
        fn search(
            tree: &Tree,
            range: &[u32],
            depth: usize,
            q: &[f64; 3],
            offsets: &mut [f64; 3],
            region: f64,
            best: &mut Vec<f64>,
            k: usize,
        ) {
            if range.is_empty() {
                return;
            }
            let middle = range.len() / 2;
            let p = tree.points[range[middle] as usize];
            let d2 = (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum::<f64>();
            if best.len() < k || d2 < best[best.len() - 1] {
                let at = best.partition_point(|b| *b <= d2);
                best.insert(at, d2);
                best.truncate(k);
            }
            let axis = depth % 3;
            let delta = q[axis] - p[axis];
            let (near, far) = if delta < 0.0 { (&range[..middle], &range[middle + 1..]) } else { (&range[middle + 1..], &range[..middle]) };
            search(tree, near, depth + 1, q, offsets, region, best, k);
            let old = offsets[axis];
            let far_region = region - old * old + delta * delta;
            if best.len() < k || far_region < best[best.len() - 1] {
                offsets[axis] = delta;
                search(tree, far, depth + 1, q, offsets, far_region, best, k);
                offsets[axis] = old;
            }
        }
        let mut best = Vec::with_capacity(k + 1);
        search(self, &self.order, 0, &q, &mut [0.0; 3], 0.0, &mut best, k);
        best
    }
}

/// Signed distance to the hull on a resolution^3 grid over [-1, 1]^3 (index (x r + y) r + z), outside positive:
/// distance to the nearest hull point, signed by its normal, smoothed with a Gaussian of 0.7 cells.
pub fn hull_grid(scene: &Scene, resolution: usize) -> Vec<f32> {
    let tree = Tree::new(&scene.hull_points);
    let axis: Vec<f64> = (0..resolution).map(|i| -1.0 + 2.0 * i as f64 / (resolution - 1) as f64).collect();
    let mut grid: Vec<f32> = (0..resolution * resolution * resolution)
        .into_par_iter()
        .map(|n| {
            let q = [axis[n / (resolution * resolution)], axis[n / resolution % resolution], axis[n % resolution]];
            let (index, d2) = tree.nearest(q);
            let (p, normal) = (scene.hull_points[index], scene.hull_normals[index]);
            let side = (0..3).map(|i| (q[i] - p[i]) * normal[i]).sum::<f64>();
            (d2.sqrt() * if side < 0.0 { -1.0 } else { 1.0 }) as f32
        })
        .collect();
    gaussian_filter(&mut grid, [resolution; 3], 0.7);
    grid
}

/// [`hull_grid`] with its inside checked away from the surface. The nearest sample's normal decides the side, and
/// beyond a thin or sharp part the nearest sample can sit on an edge whose normal points sideways: a strip of space
/// outside the hull then reads as inside (in the splat mesh, which takes the hull's side wherever the band ends, a
/// spike). Points that read as inside and lie more than `far` from the hull are therefore tested by the winding
/// number of the samples (about 1 inside, 0 outside; up to 4 096 samples, each standing for the area its spacing
/// gives) and turned outside when it is below one half. Near the surface the grid is [`hull_grid`]'s.
pub fn hull_grid_checked(scene: &Scene, resolution: usize, far: f64) -> Vec<f32> {
    let points = &scene.hull_points;
    let tree = Tree::new(points);
    let probe = (points.len() / 2000).max(1);
    let spacing: Vec<f64> = points.iter().step_by(probe).map(|p| tree.nearest_k(*p, 2).get(1).copied().unwrap_or(0.0).sqrt()).collect();
    let h = spacing.iter().sum::<f64>() / spacing.len().max(1) as f64;
    let stride = (points.len() / 4096).max(1);
    let area = 4.0 * h * h * stride as f64;
    let samples: Vec<([f64; 3], [f64; 3])> = points.iter().zip(&scene.hull_normals).step_by(stride).map(|(p, n)| (*p, *n)).collect();
    let axis: Vec<f64> = (0..resolution).map(|i| -1.0 + 2.0 * i as f64 / (resolution - 1) as f64).collect();
    let mut grid: Vec<f32> = (0..resolution * resolution * resolution)
        .into_par_iter()
        .map(|n| {
            let q = [axis[n / (resolution * resolution)], axis[n / resolution % resolution], axis[n % resolution]];
            let (index, d2) = tree.nearest(q);
            let (p, normal) = (points[index], scene.hull_normals[index]);
            let side = (0..3).map(|i| (q[i] - p[i]) * normal[i]).sum::<f64>();
            let distance = d2.sqrt();
            if side >= 0.0 {
                return distance as f32;
            }
            if distance > far {
                let winding: f64 = samples
                    .iter()
                    .map(|(p, n)| {
                        let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                        let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                        (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]) / (r2 * r2.sqrt()).max(1e-30)
                    })
                    .sum::<f64>()
                    * area
                    / (4.0 * std::f64::consts::PI);
                if winding < 0.5 {
                    return distance as f32;
                }
            }
            -distance as f32
        })
        .collect();
    gaussian_filter(&mut grid, [resolution; 3], 0.7);
    grid
}

/// The prototype's trilinear read of a grid over [-1, 1]^3 (clamped to the grid).
pub fn trilinear(grid: &[f32], resolution: usize, p: [f32; 3]) -> f32 {
    let r = resolution;
    let g = p.map(|v| ((v + 1.0) * (0.5 * (r - 1) as f32)).clamp(0.0, r as f32 - 1.001));
    let i = g.map(|v| v.floor());
    let f: [f32; 3] = std::array::from_fn(|k| g[k] - i[k]);
    let c = i.map(|v| v as usize);
    let mut total = 0.0;
    for corner in 0..8 {
        let o = [corner >> 2, (corner >> 1) & 1, corner & 1];
        let w: f32 = (0..3).map(|k| if o[k] == 1 { f[k] } else { 1.0 - f[k] }).product();
        total += grid[((c[0] + o[0]) * r + (c[1] + o[1])) * r + (c[2] + o[2])] * w;
    }
    total
}

/// The geometry network's initial parameters as the prototype makes them (geometric initialisation towards a
/// sphere, bias -0.5; colour layers Kaiming-uniform), from our random numbers; the table uniform in +-1e-3.
pub fn initial(shape: &Shape, variance: f32, random: &mut Random) -> (Vec<f32>, Vec<f32>) {
    let table: Vec<f32> = (0..shape.table_floats()).map(|_| ((random.uniform() * 2.0 - 1.0) * 1e-3) as f32).collect();
    let (layers, count, _) = layers(shape);
    let mut parameters = vec![0f32; count as usize + 1];
    let h = shape.hidden as f64;
    for (index, layer) in layers.iter().enumerate() {
        let (inputs, outputs) = (layer.inputs as usize, layer.outputs as usize);
        let mut v = vec![0f64; inputs * outputs];
        let mut bias = vec![0f64; outputs];
        match index {
            0 => (0..outputs).for_each(|o| (0..3).for_each(|i| v[o * inputs + i] = random.normal() * 2f64.sqrt() / h.sqrt())),
            1 => v.iter_mut().for_each(|x| *x = random.normal() * 2f64.sqrt() / h.sqrt()),
            2 => {
                v.iter_mut().for_each(|x| *x = std::f64::consts::PI.sqrt() / h.sqrt() + random.normal() * 1e-4);
                bias.fill(-0.5);
            }
            _ => {
                let bound = (6.0 / inputs as f64).sqrt();
                v.iter_mut().for_each(|x| *x = (random.uniform() * 2.0 - 1.0) * bound);
            }
        }
        for o in 0..outputs {
            let row = &v[o * inputs..(o + 1) * inputs];
            parameters[layer.g as usize + o] = row.iter().map(|x| x * x).sum::<f64>().sqrt() as f32;
            parameters[layer.b as usize + o] = bias[o] as f32;
            for (i, x) in row.iter().enumerate() {
                parameters[layer.v as usize + o * inputs + i] = *x as f32;
            }
        }
    }
    parameters[count as usize] = variance;
    (table, parameters)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FitParams {
    count: u32,
    geo: u32,
    inv_count: f32,
    eikonal: f32,
}

/// Fills a batch with random rays: origins (with stratified offsets) and ray data.
fn draw_rays(scene: &Scene, random: &mut Random, count: usize) -> (Vec<f32>, Vec<f32>) {
    let mut origins = vec![0f32; count * 8];
    let mut data = vec![0f32; count * 8];
    for r in 0..count {
        let view = &scene.views[random.below(scene.views.len())];
        let (y, x) = (random.below(view.height), random.below(view.width));
        let [fx, fy, cx, cy] = view.intrinsics;
        let c = [(x as f64 + 0.5 - cx) / fx, (y as f64 + 0.5 - cy) / fy, 1.0];
        let mut d: [f64; 3] = std::array::from_fn(|i| (0..3).map(|j| view.rotation[j][i] * c[j]).sum());
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        d.iter_mut().for_each(|v| *v /= length);
        let pixel = y * view.width + x;
        for i in 0..3 {
            origins[r * 8 + i] = view.origin[i] as f32;
            data[r * 8 + i] = d[i] as f32;
            data[r * 8 + 3 + i] = view.image[pixel * 3 + i] as f32 / 255.0;
        }
        origins[r * 8 + 3] = random.uniform() as f32;
        data[r * 8 + 6] = if view.mask[pixel] { 1.0 } else { 0.0 };
    }
    (origins, data)
}

/// Writes the network's zero surface (within the hull band) as STL in world coordinates; returns the triangles.
#[allow(clippy::too_many_arguments)]
pub async fn save_mesh(
    gpu: &Gpu,
    kernels: &Kernels,
    model: &Model,
    batch: &Batch,
    scene: &Scene,
    hull: Option<(&[f32], usize)>,
    band: f32,
    resolution: usize,
    path: &Path,
) -> Result<usize> {
    let capacity = batch.samples() as usize;
    let axis: Vec<f32> = (0..resolution).map(|i| -1.0 + 2.0 * i as f32 / (resolution - 1) as f32).collect();
    let total = resolution * resolution * resolution;
    let mut values = vec![0f32; total];
    let mut start = 0;
    while start < total {
        let count = capacity.min(total - start);
        let points: Vec<f32> = (start..start + count)
            .flat_map(|n| [axis[n / (resolution * resolution)], axis[n / resolution % resolution], axis[n % resolution]])
            .collect();
        gpu.write(&batch.points, &points);
        distances(gpu, kernels, model, batch, count as u32).await?;
        let out: Vec<f32> = gpu.read(&batch.z3, count).await?;
        values[start..start + count].copy_from_slice(&out);
        start += count;
    }
    if let Some((grid, r)) = hull {
        values.par_iter_mut().enumerate().for_each(|(n, v)| {
            let p = [axis[n / (resolution * resolution)], axis[n / resolution % resolution], axis[n % resolution]];
            *v = v.max(trilinear(grid, r, p) - band);
        });
    }
    let mesh = cubes::extract(&values, [resolution; 3], 1);
    if mesh.faces.is_empty() {
        return Ok(0);
    }
    let spacing = 2.0 / (resolution - 1) as f64;
    let vertices: Vec<[f64; 3]> =
        mesh.vertices.iter().map(|v| std::array::from_fn(|i| (v[i] as f64 * spacing - 1.0) / scene.unit + scene.centre[i])).collect();
    crate::stl::write_binary(path, "crispmesh neural surface (webgpu)", &vertices, &mesh.faces)?;
    Ok(mesh.faces.len())
}

/// Fits the geometry network to the hull's distance (L1 plus 0.1 eikonal), `steps` Adam steps at 1e-3.
#[allow(clippy::too_many_arguments)]
async fn fit_hull(
    say: &dyn Fn(&str),
    gpu: &Gpu,
    kernels: &Kernels,
    model: &Model,
    batch: &Batch,
    scene: &Scene,
    grid: &[f32],
    resolution: usize,
    steps: u32,
    random: &mut Random,
    start: &web_time::Instant,
) -> Result<()> {
    let fit = gpu.kernel("surface_fit", include_str!("../shaders/surface_fit.wgsl"), "main").await?;
    let per_part = batch.samples() as usize;
    let total = 65536usize;
    let parts = total.div_ceil(per_part);
    let geo = 1 + model.shape.features;
    for step in 0..steps {
        model.begin_step(gpu, kernels).await?;
        let mut loss = 0.0;
        for part in 0..parts {
            let count = per_part.min(total - part * per_part);
            let mut points = Vec::with_capacity(count * 3);
            let mut targets = Vec::with_capacity(count);
            for n in 0..count {
                let p: [f32; 3] = if part * per_part + n < total / 2 {
                    let h = scene.hull_points[random.below(scene.hull_points.len())];
                    std::array::from_fn(|i| ((h[i] + random.normal() * 0.03) as f32).clamp(-1.0, 1.0))
                } else {
                    std::array::from_fn(|_| (random.uniform() * 2.0 - 1.0) as f32)
                };
                points.extend_from_slice(&p);
                targets.push(trilinear(grid, resolution, p));
            }
            gpu.write(&batch.points, &points);
            gpu.write(&batch.targets, &targets);
            forward_geometry(gpu, kernels, model, batch, count as u32).await?;
            let uniform = gpu.uniform("surface fit", &FitParams { count: count as u32, geo, inv_count: 1.0 / total as f32, eikonal: 0.1 });
            gpu.run(&fit, &[&uniform, &batch.targets, &batch.z3, &batch.dz3, &batch.fit_losses], count as u64).await?;
            backward_geometry(gpu, kernels, model, batch, count as u32).await?;
            if step % 200 == 0 || step == steps - 1 {
                let losses: Vec<f32> = gpu.read(&batch.fit_losses, count).await?;
                loss += losses.iter().map(|v| *v as f64).sum::<f64>() / total as f64;
            }
        }
        model.end_gradients(gpu, kernels).await?;
        model.adam(gpu, kernels, 1e-3, step, false).await?;
        if step % 200 == 0 || step == steps - 1 {
            say(&format!("[{:6.0}s] hull fit {step}: {loss:.5}", start.elapsed().as_secs_f64()));
        }
    }
    model.reset_moments(gpu);
    Ok(())
}

/// Trains on a scene and writes meshes into `out`; resumes from `out/checkpoint.npz`. Progress to stdout.
pub async fn train(gpu: &Gpu, scene: &Scene, out: &Path, settings: &Settings) -> Result<()> {
    train_with(gpu, scene, out, settings, &|text| println!("{text}"), &|| Ok(())).await
}

/// [`train`] with progress lines to `say` and `check` called every step (an error stops the training).
pub async fn train_with(
    gpu: &Gpu,
    scene: &Scene,
    out: &Path,
    settings: &Settings,
    say: &dyn Fn(&str),
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    ensure!(settings.parts > 0 && settings.rays.is_multiple_of(settings.parts), "rays must be a multiple of parts");
    let start = web_time::Instant::now();
    let log = |start: &web_time::Instant, text: &str| say(&format!("[{:6.0}s] {text}", start.elapsed().as_secs_f64()));
    crate::storage::create_dir_all(out)?;
    let pixels: f64 = scene.views.iter().map(|v| v.mask.iter().filter(|m| **m).count() as f64 / v.mask.len() as f64).sum::<f64>()
        / scene.views.len() as f64;
    log(&start, &format!("{} views {}x{}, foreground {pixels:.3}", scene.views.len(), scene.views[0].width, scene.views[0].height));
    let shape = Shape { hidden: settings.hidden, features: settings.features, table_log2: settings.table_log2 };
    let mut random = Random::new(settings.seed);
    let kernels = Kernels::new(gpu).await?;
    gpu.set_batching(true);
    let checkpoint = out.join("checkpoint.npz");
    let (table, parameters, first_step) = if crate::storage::exists(&checkpoint) {
        let npz = crate::npz::Npz::read(&checkpoint)?;
        let step = npz.get("step")?.scalar()? as u32 + 1;
        (npz.get("table")?.to_f32(), npz.get("parameters")?.to_f32(), step)
    } else {
        let (t, p) = initial(&shape, settings.variance, &mut random);
        (t, p, 0)
    };
    let model = Model::new(gpu, shape, &table, &parameters)?;
    drop((table, parameters));
    let rays_per_part = settings.rays / settings.parts;
    let batch = Batch::new(gpu, &shape, rays_per_part, settings.sampling.per_ray());
    let resolution = 160;
    let use_hull = settings.sampling.band > 0.0 || settings.hull_steps > 0;
    let grid = if use_hull { hull_grid(scene, resolution) } else { Vec::new() };
    log(&start, "hull grid");
    let hull = use_hull.then(|| Hull { grid: gpu.upload("surface hull", &grid), resolution: resolution as u32 });
    let band_hull = if settings.sampling.band > 0.0 { hull.as_ref() } else { None };
    let mesh_hull = (settings.sampling.band > 0.0).then_some((grid.as_slice(), resolution));
    if first_step > 0 {
        log(&start, &format!("resumed after step {} (optimiser moments restart)", first_step - 1));
    } else if settings.hull_steps > 0 {
        fit_hull(say, gpu, &kernels, &model, &batch, scene, &grid, resolution, settings.hull_steps, &mut random, &start).await?;
        model.begin_step(gpu, &kernels).await?;
        let triangles =
            save_mesh(gpu, &kernels, &model, &batch, scene, mesh_hull, settings.sampling.band, 256, &out.join("mesh-hull.stl")).await?;
        log(&start, &format!("hull mesh: {triangles} triangles"));
    }
    let end = if settings.stop > 0 { settings.stop } else { settings.steps };
    let mut profile = Profile::new(std::env::var("CRISPMESH_SURFACE_PROFILE").is_ok_and(|v| v == "1"));
    let mut history = Vec::new();
    for step in first_step..end {
        check()?;
        let factor = if step < settings.warm_up {
            step as f32 / settings.warm_up as f32
        } else {
            let progress = (step - settings.warm_up) as f32 / (settings.steps - settings.warm_up) as f32;
            ((std::f32::consts::PI * progress).cos() + 1.0) * 0.5 * (1.0 - settings.rate_end) + settings.rate_end
        };
        let pass = Pass {
            anneal: (step as f32 / settings.anneal_end as f32).min(1.0),
            step_rays: rays_per_part * settings.parts,
            eikonal: settings.eikonal,
            mask: settings.mask,
        };
        model.begin_step(gpu, &kernels).await?;
        for _ in 0..settings.parts {
            let (origins, data) = draw_rays(scene, &mut random, rays_per_part as usize);
            gpu.write(&batch.origins, &origins);
            gpu.write(&batch.ray_data, &data);
            profile.mark(gpu, &batch, 0).await?;
            sample(gpu, &kernels, &model, &batch, &settings.sampling, band_hull).await?;
            profile.mark(gpu, &batch, 1).await?;
            forward_batch(gpu, &kernels, &model, &batch).await?;
            profile.mark(gpu, &batch, 2).await?;
            backward_batch(gpu, &kernels, &model, &batch, &pass).await?;
            profile.mark(gpu, &batch, 3).await?;
        }
        model.end_gradients(gpu, &kernels).await?;
        model.adam(gpu, &kernels, settings.rate * factor, step, false).await?;
        profile.mark(gpu, &batch, 4).await?;
        if profile.on && step % settings.log_every == 0 {
            log(&start, &profile.report());
        }
        if step % settings.log_every == 0 {
            if let Some(report) = gpu.profile_report() {
                log(&start, &format!("kernels since the last report: {report}"));
            }
        }
        if step % settings.log_every == 0 || step == settings.steps - 1 {
            let losses: Vec<f32> = gpu.read(&batch.losses, rays_per_part as usize * 4).await?;
            let variance: Vec<f32> = gpu.read(&model.parameters, model.variance as usize + 1).await?;
            let n = rays_per_part as f64;
            let sum = |k: usize| losses.iter().skip(k).step_by(4).map(|v| *v as f64).sum::<f64>();
            let (colour, eikonal, mask) = (sum(0) / (3.0 * n), sum(1) / (n * batch.per_ray as f64), sum(2) / n);
            let loss = colour + settings.eikonal as f64 * eikonal + settings.mask as f64 * mask;
            let sharpness = (variance[model.variance as usize] as f64 * 10.0).exp();
            history.push(serde_json::json!([step, loss, colour, eikonal, mask, sharpness]));
            log(
                &start,
                &format!("step {step}: loss {loss:.4} colour {colour:.4} eikonal {eikonal:.4} mask {mask:.4} sharpness {sharpness:.0}"),
            );
        }
        let low_disk = step % 100 == 0 && free_bytes(out).is_some_and(|free| free < (8.8 * (1u64 << 30) as f64) as u64);
        if (settings.snapshot > 0 && step > 0 && step % settings.snapshot == 0) || low_disk {
            save_checkpoint(gpu, &model, step, &checkpoint).await?;
            model.begin_step(gpu, &kernels).await?;
            let triangles = save_mesh(
                gpu,
                &kernels,
                &model,
                &batch,
                scene,
                mesh_hull,
                settings.sampling.band,
                256,
                &out.join(format!("mesh-{step}.stl")),
            )
            .await?;
            log(&start, &format!("snapshot: {triangles} triangles"));
        }
        if low_disk {
            bail!("less than 8.8 GiB free on the disk; run again to resume");
        }
    }
    let train_seconds = start.elapsed().as_secs_f64();
    model.begin_step(gpu, &kernels).await?;
    let resolution = if settings.stop > 0 { 256 } else { settings.mesh as usize };
    let triangles =
        save_mesh(gpu, &kernels, &model, &batch, scene, mesh_hull, settings.sampling.band, resolution, &out.join("mesh.stl")).await?;
    let summary = serde_json::json!({"steps": settings.steps, "rays": settings.rays, "views": scene.views.len(), "train_seconds": train_seconds,
        "seconds": start.elapsed().as_secs_f64(), "triangles": triangles, "adapter": gpu.describe(), "history": history});
    crate::storage::write(out.join("summary.json"), serde_json::to_string_pretty(&summary)?)?;
    log(&start, &format!("done: {triangles} triangles, training {train_seconds:.0}s"));
    Ok(())
}

/// Time per phase of a step, when switched on (each mark waits for the GPU).
struct Profile {
    on: bool,
    last: web_time::Instant,
    seconds: [f64; 5],
    steps: u32,
}

impl Profile {
    fn new(on: bool) -> Self {
        Profile { on, last: web_time::Instant::now(), seconds: [0.0; 5], steps: 0 }
    }

    /// Ends phase `phase - 1` (phase 0 starts a part; phase 4 ends the step).
    async fn mark(&mut self, gpu: &Gpu, batch: &Batch, phase: usize) -> Result<()> {
        if !self.on {
            return Ok(());
        }
        let _: Vec<f32> = gpu.read(&batch.losses, 1).await?;
        let now = web_time::Instant::now();
        if phase > 0 {
            self.seconds[phase - 1] += (now - self.last).as_secs_f64();
        }
        if phase == 4 {
            self.steps += 1;
        }
        self.last = now;
        Ok(())
    }

    fn report(&mut self) -> String {
        let n = self.steps.max(1) as f64;
        let [rays, sampler, forward, backward, adam] = self.seconds.map(|s| s / n);
        self.seconds = [0.0; 5];
        self.steps = 0;
        format!("per step: rays {rays:.3}s sampler {sampler:.3}s forward {forward:.3}s backward {backward:.3}s adam {adam:.3}s")
    }
}

fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(not(target_arch = "wasm32"))]
    return fs4::available_space(path).ok();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}

async fn save_checkpoint(gpu: &Gpu, model: &Model, step: u32, path: &Path) -> Result<()> {
    let table: Vec<f32> = gpu.read(&model.table, model.shape.table_floats() as usize).await?;
    let parameters: Vec<f32> = gpu.read(&model.parameters, model.variance as usize + 1).await?;
    use crate::npz::{Array, Data};
    let t = Array::new(&[table.len()], Data::F32(table));
    let p = Array::new(&[parameters.len()], Data::F32(parameters));
    let s = Array::scalar_f32(step as f32);
    crate::npz::write(path, &[("table", &t), ("parameters", &p), ("step", &s)], false)
}

/// `crispmesh-dense surface SOURCE OUT [quality=fast|standard|best] [NAME=VALUE ...]`
#[cfg(not(target_arch = "wasm32"))]
pub fn main(arguments: &[String]) -> Result<()> {
    const USAGE: &str =
        "usage: crispmesh-dense surface SOURCE OUT [quality=fast|standard|best] [steps=N stop=N rays=N parts=N hull=N band=X \
coarse=N added=N width=N features=N table=LOG2 mesh=N snapshot=N log=N seed=N] [hull_mesh=STL photos=DIR masks=DIR div=N crop=PIXELS]
SOURCE: a pack of scripts/neural_surface/pack_scene.py (scene.json), or an inputs directory (cameras.json) with
hull_mesh (our silhouette mesh), optionally photos and masks (folders replacing the inputs' original photos and
masks), div (photo reduction; default from crop) and crop (target width of the window around the object: 480 fast,
720 standard, 960 best). The quality level sets the defaults; single settings after it override them.";
    ensure!(arguments.len() >= 2, "{USAGE}");
    let (source, out) = (std::path::PathBuf::from(&arguments[0]), std::path::PathBuf::from(&arguments[1]));
    let (quality, items) = super::options::split(&arguments[2..]).map_err(|e| anyhow!("{e}\n{USAGE}"))?;
    let mut s = Settings::for_quality(quality);
    let mut from = super::options::Source::for_quality(quality, 960);
    for (name, value) in &items {
        if !s.set(name, value)? && !from.set(name, value)? {
            bail!("unknown setting {name}\n{USAGE}");
        }
    }
    let scene = from.scene(&source, s.seed)?;
    let gpu = Gpu::new()?;
    crate::gpu::block_on(train(&gpu, &scene, &out, &s))
}

#[cfg(test)]
mod crop_window_tests {
    use super::*;
    use image::{GrayImage, Luma, Rgb, RgbImage};

    #[test]
    fn existing_integer_area_pixels_and_camera_are_exactly_preserved() {
        let rgb = RgbImage::from_fn(4, 4, |x, y| Rgb([(x + 4 * y) as u8; 3]));
        let mask = GrayImage::from_fn(4, 4, |x, _| Luma([if x < 2 { 255 } else { 0 }]));
        let window = photo_window(rgb, mask, [0, 0, 4, 4], 4, 4, 2);
        assert_eq!(window.origin, [0, 0]);
        assert_eq!(window.image, vec![3, 3, 3, 0, 0, 0, 11, 11, 11, 0, 0, 0]);
        assert_eq!(window.mask, vec![true, false, true, false]);
        assert_eq!(window.intrinsics([80., 90., 1.7, 2.3]), [40., 45., 1.7 / 2., 2.3 / 2.]);
    }

    #[test]
    fn small_valid_crop_resizes_without_fabricating_background() {
        let rgb = RgbImage::from_pixel(4, 6, Rgb([19, 73, 111]));
        let mask = GrayImage::from_pixel(4, 6, Luma([255]));
        let window = photo_window(rgb, mask, [0, 0, 4, 6], 8, 8, 2);
        assert_eq!(window.origin, [0, 0]);
        assert_eq!(window.scale, [1., 2. / 3.]);
        assert_eq!(window.mask.len(), 16);
        assert!(window.mask.iter().all(|&x| x));
        assert_eq!(window.image, [19, 73, 111].repeat(16));
        assert_eq!(window.intrinsics([80., 90., 2., 3.]), [80., 60., 2., 2.]);
    }

    #[test]
    fn resize_camera_preserves_offcentre_rays_in_both_axes() {
        let rgb = RgbImage::from_pixel(100, 160, Rgb([81, 92, 103]));
        let mask = GrayImage::from_pixel(100, 160, Luma([255]));
        let window = photo_window(rgb, mask, [10, 40, 90, 112], 128, 128, 4);
        assert_eq!(window.origin, [0, 12]);
        let original = [113., 127., 13.5, 61.5];
        let prepared = window.intrinsics(original);
        for xy in [[0.5, 12.5], [99.5, 139.5], [31.1, 75.3]] {
            for axis in 0..2 {
                let resized = (xy[axis] - window.origin[axis] as f64) * window.scale[axis];
                let before = (xy[axis] - original[axis + 2]) / original[axis];
                let after = (resized - prepared[axis + 2]) / prepared[axis];
                assert!((before - after).abs() < 1e-14);
            }
        }
    }
}
