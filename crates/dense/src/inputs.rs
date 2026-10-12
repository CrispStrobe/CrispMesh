//! The inputs directory of the dense stage: `cameras.json`, photos, masks and
//! `sparse_points.npy`, prepared exactly as `Stereo.__init__` does in
//! `scripts/turntable_mesh/multiscale_stereo.py`.
//!
//! Numerics follow NumPy 1.x as the reference runs it: grey is the float32 mean
//! of the three channels over 255; the stretch range comes from
//! `np.percentile` (linear interpolation, evaluated in float64 on float32
//! differences) and is applied in float32.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde::Deserialize;

use crate::config::DenseConfig;

/// Worker threads for CPU-side loops (the machine is shared: at most 4).
pub const THREADS: usize = 4;

/// Row-major single-channel image.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane<T> {
    pub width: usize,
    pub height: usize,
    pub data: Vec<T>,
}

impl<T: Copy + Default> Plane<T> {
    pub fn new(width: usize, height: usize) -> Self {
        Plane { width, height, data: vec![T::default(); width * height] }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> T {
        self.data[y * self.width + x]
    }
}

/// Pinhole camera in float32, as the reference keeps it on the device:
/// `camera = rotation * world + translation`, `k = [fx, fy, cx, cy]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub rotation: [[f32; 3]; 3],
    pub translation: [f32; 3],
    pub k: [f32; 4],
}

impl Camera {
    /// `points @ R.T + t` in float32, left to right.
    #[inline]
    pub fn to_camera(&self, p: [f32; 3]) -> [f32; 3] {
        let r = &self.rotation;
        [
            p[0] * r[0][0] + p[1] * r[0][1] + p[2] * r[0][2] + self.translation[0],
            p[0] * r[1][0] + p[1] * r[1][1] + p[2] * r[1][2] + self.translation[1],
            p[0] * r[2][0] + p[1] * r[2][1] + p[2] * r[2][2] + self.translation[2],
        ]
    }

    /// Camera centre `-R.T @ t` in float32.
    pub fn centre(&self) -> [f32; 3] {
        let (r, t) = (&self.rotation, &self.translation);
        [
            -(r[0][0] * t[0] + r[1][0] * t[1] + r[2][0] * t[2]),
            -(r[0][1] * t[0] + r[1][1] * t[1] + r[2][1] * t[2]),
            -(r[0][2] * t[0] + r[1][2] * t[1] + r[2][2] * t[2]),
        ]
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ViewRow {
    pub name: String,
    pub image: String,
    /// Optional unenhanced, undistorted RGB for texture export; dense matching uses `image`.
    #[serde(default)]
    pub texture_image: Option<String>,
    pub mask: String,
    #[serde(default)]
    pub width: usize,
    #[serde(default)]
    pub height: usize,
    pub k: [f64; 4],
    pub rotation: [[f64; 3]; 3],
    pub translation: [f64; 3],
}

#[derive(Deserialize)]
struct CameraFile {
    views: Vec<ViewRow>,
}

pub struct Inputs {
    pub rows: Vec<ViewRow>,
    pub cameras: Vec<Camera>,
    /// Contrast-normalised grey per view, native size.
    pub gray: Vec<Plane<f32>>,
    /// Position of each grey plane's first pixel in its photo: (0, 0) until [`Inputs::crop_gray`].
    pub gray_origin: Vec<[i64; 2]>,
    /// Object mask per view (0 or 1), native size. Mask repair rewrites these.
    pub masks: Vec<Plane<u8>>,
    /// Common-canvas crop per view: `[x0, y0, x1, y1]`, may leave the image.
    pub boxes: Vec<[i64; 4]>,
    /// Longest side of the common canvas.
    pub longest: i64,
    pub sparse: Vec<[f64; 3]>,
    /// Angle in degrees between the viewing directions of every view pair.
    pub angles: Vec<Vec<f64>>,
    /// Views allowed to disagree during mask repair.
    pub repair_loose: i64,
}

/// Runs `work(index)` for every index on up to [`THREADS`] threads; results in order.
/// Without threads (wasm32) it is a plain loop; in the threaded browser package
/// (`+atomics`, rayon's pool started by the host) it runs on that pool. There
/// `work` may read files of the in-memory tree but not files the host hands
/// over through its source: those callbacks exist only on the calling thread.
#[cfg(target_arch = "wasm32")]
pub fn parallel_map<T: Send>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    #[cfg(target_feature = "atomics")]
    if count > 1 && rayon::current_num_threads() > 1 {
        use rayon::prelude::*;
        let work = &work;
        return (0..count).into_par_iter().map(|n| work(n)).collect();
    }
    (0..count).map(work).collect()
}

/// Files that [`parallel_map`] closures read, made readable on the pool: in the
/// threaded browser package a pool worker cannot call the host's file source,
/// so files the tree does not hold yet are read here, on the calling thread, and
/// stored in the tree. Returns the ones copied, for [`release`] once the work is
/// done. Elsewhere it does nothing.
pub fn hold_for_pool(paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    #[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
    if rayon::current_num_threads() > 1 {
        let mut stored: std::collections::HashMap<PathBuf, std::collections::HashSet<String>> = Default::default();
        let mut held = Vec::new();
        for path in paths {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let names = stored.entry(parent.clone()).or_insert_with(|| {
                crate::storage::memory_files(&parent).into_iter().filter_map(|(key, _)| key.rsplit('/').next().map(String::from)).collect()
            });
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if names.contains(&name) {
                continue;
            }
            let bytes = crate::storage::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            crate::storage::write_owned(path, bytes)?;
            names.insert(name);
            held.push(path.clone());
        }
        return Ok(held);
    }
    let _ = paths;
    Ok(Vec::new())
}

/// Removes what [`hold_for_pool`] copied into the tree.
pub fn release(held: Vec<PathBuf>) {
    for path in held {
        let _ = crate::storage::remove_file(path);
    }
}

/// Runs `work(index)` for every index on up to [`THREADS`] threads; results in order.
#[cfg(not(target_arch = "wasm32"))]
pub fn parallel_map<T: Send>(count: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut slots: Vec<Option<T>> = (0..count).map(|_| None).collect();
    let results = std::sync::Mutex::new(&mut slots);
    std::thread::scope(|scope| {
        for _ in 0..THREADS.min(count.max(1)) {
            scope.spawn(|| loop {
                let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if n >= count {
                    break;
                }
                let value = work(n);
                results.lock().unwrap()[n] = Some(value);
            });
        }
    });
    slots.into_iter().map(|s| s.expect("worker finished")).collect()
}

/// Python's `round()`: nearest, ties to even.
pub fn round_half_even(value: f64) -> f64 {
    let floor = value.floor();
    let rest = value - floor;
    if rest > 0.5 || (rest == 0.5 && floor.rem_euclid(2.0) == 1.0) {
        floor + 1.0
    } else {
        floor
    }
}

/// `np.percentile(values, q)` with linear interpolation, for sorted float64 data.
pub fn percentile_sorted_f64(sorted: &[f64], q: f64) -> f64 {
    let (previous, next, gamma) = percentile_position(sorted.len(), q);
    let (a, b) = (sorted[previous], sorted[next]);
    lerp(a, b, b - a, gamma)
}

/// The same for sorted float32 data: NumPy 1.x subtracts in float32 and
/// interpolates in float64.
pub fn percentile_sorted_f32(sorted: &[f32], q: f64) -> f64 {
    let (previous, next, gamma) = percentile_position(sorted.len(), q);
    let (a, b) = (sorted[previous], sorted[next]);
    lerp(a as f64, b as f64, (b - a) as f64, gamma)
}

fn percentile_position(count: usize, q: f64) -> (usize, usize, f64) {
    let last = count - 1;
    let position = last as f64 * (q / 100.0);
    if position >= last as f64 {
        return (last, last, 0.0);
    }
    if position < 0.0 {
        return (0, 0, 0.0);
    }
    let previous = position.floor();
    (previous as usize, previous as usize + 1, position - previous)
}

fn lerp(a: f64, b: f64, difference: f64, t: f64) -> f64 {
    if t >= 0.5 {
        b - difference * (1.0 - t)
    } else {
        a + difference * t
    }
}

/// `np.median` of float64 values.
pub fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// Pillow's `convert("L")` of an 8-bit RGB pixel.
fn pil_luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16) as u8
}

/// A photo as 8-bit RGB, like `Image.open(path).convert("RGB")` for 8-bit
/// files (alpha is dropped, grey is repeated). Deeper files are reduced to 8
/// bits by the decoder, which Pillow does differently; none of the pipeline's
/// inputs are.
fn load_rgb(path: &Path) -> anyhow::Result<image::RgbImage> {
    let decoded = crate::storage::open_image(path).with_context(|| path.display().to_string())?;
    Ok(decoded.to_rgb8())
}

/// A mask as booleans, like `np.asarray(Image.open(path).convert("L")) > 127`.
fn load_mask(path: &Path) -> anyhow::Result<Plane<u8>> {
    let decoded = crate::storage::open_image(path).with_context(|| path.display().to_string())?;
    let (width, height) = (decoded.width() as usize, decoded.height() as usize);
    let luma: Vec<u8> = match decoded {
        image::DynamicImage::ImageLuma8(l) => l.into_raw(),
        image::DynamicImage::ImageLumaA8(la) => la.pixels().map(|p| p.0[0]).collect(),
        image::DynamicImage::ImageRgb8(rgb) => rgb.pixels().map(|p| pil_luma(p.0[0], p.0[1], p.0[2])).collect(),
        image::DynamicImage::ImageRgba8(rgba) => rgba.pixels().map(|p| pil_luma(p.0[0], p.0[1], p.0[2])).collect(),
        other => other.to_luma8().into_raw(),
    };
    Ok(Plane { width, height, data: luma.into_iter().map(|v| (v > 127) as u8).collect() })
}

/// The mask of a view and the bounds of its object pixels `[x0, y0, x1, y1]` (inclusive).
fn load_view_mask(row: &ViewRow) -> anyhow::Result<(Plane<u8>, [i64; 4])> {
    let mask = load_mask(Path::new(&row.mask))?;
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for y in 0..mask.height {
        for x in 0..mask.width {
            if mask.data[y * mask.width + x] != 0 {
                x0 = x0.min(x as i64);
                x1 = x1.max(x as i64);
                y0 = y0.min(y as i64);
                y1 = y1.max(y as i64);
            }
        }
    }
    if x1 < x0 {
        bail!("mask must be nonempty and match its image: {}", row.name);
    }
    Ok((mask, [x0, y0, x1, y1]))
}

/// The grey plane of a view, stretched by the object's grey percentiles; only the part inside `keep`
/// (`[x0, y0, x1, y1)` in photo pixels, clamped to the photo) when given. Returns the plane and its origin.
fn load_view_gray(row: &ViewRow, mask: &Plane<u8>, percentiles: &[f64], keep: Option<[i64; 4]>) -> anyhow::Result<(Plane<f32>, [i64; 2])> {
    let rgb = load_rgb(Path::new(&row.image))?;
    let (width, height) = (rgb.width() as usize, rgb.height() as usize);
    if (mask.width, mask.height) != (width, height) {
        bail!("mask must be nonempty and match its image: {}", row.name);
    }
    // rgb.mean(2) / 255 on float32: the channel sum is exact, then two divisions.
    let grey = |p: &image::Rgb<u8>| (p.0[0] as f32 + p.0[1] as f32 + p.0[2] as f32) / 3.0 / 255.0;
    let mut inside: Vec<f32> = rgb.pixels().zip(&mask.data).filter(|(_, &m)| m != 0).map(|(p, _)| grey(p)).collect();
    inside.sort_unstable_by(f32::total_cmp);
    let lo = percentile_sorted_f32(&inside, percentiles[0]);
    let hi = percentile_sorted_f32(&inside, percentiles[1]);
    // NumPy 1.x casts both float64 scalars to float32 before the array arithmetic.
    let (low, range) = (lo as f32, (hi - lo).max(1e-6) as f32);
    let [x0, y0, x1, y1] = keep.unwrap_or([0, 0, width as i64, height as i64]);
    let (x0, y0) = (x0.clamp(0, width as i64) as usize, y0.clamp(0, height as i64) as usize);
    let (x1, y1) = (x1.clamp(x0 as i64, width as i64) as usize, y1.clamp(y0 as i64, height as i64) as usize);
    let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        data.extend((x0..x1).map(|x| (grey(rgb.get_pixel(x as u32, y as u32)) - low) / range));
    }
    Ok((Plane { width: x1 - x0, height: y1 - y0, data }, [x0 as i64, y0 as i64]))
}

/// Whole grey photos are kept until the masks are repaired as long as they take at most this much
/// memory; beyond it (many or very large photos) each keeps its canvas and a quarter canvas around it.
const WHOLE_GRAY_BUDGET: usize = 2 << 30;

/// Camera rows of an inputs directory with absolute image and mask paths (relative ones resolve to the directory).
pub fn load_views(directory: &Path) -> anyhow::Result<Vec<ViewRow>> {
    let text = crate::storage::read_to_string(directory.join("cameras.json"))
        .with_context(|| directory.join("cameras.json").display().to_string())?;
    let mut rows = serde_json::from_str::<CameraFile>(&text).context("cameras.json")?.views;
    for row in &mut rows {
        for path in [&mut row.image, &mut row.mask].into_iter().chain(row.texture_image.iter_mut()) {
            if !Path::new(path.as_str()).is_absolute() {
                *path = directory.join(path.as_str()).to_string_lossy().to_string();
            }
        }
    }
    Ok(rows)
}

impl Inputs {
    pub fn load(directory: &Path, config: &DenseConfig) -> anyhow::Result<Self> {
        Self::load_with_masks(directory, config, None)
    }

    /// [`Inputs::load`] with the masks read from `masks/<view name>.png` instead of the files the views name.
    pub fn load_with_masks(directory: &Path, config: &DenseConfig, masks: Option<&Path>) -> anyhow::Result<Self> {
        let mut rows = load_views(directory)?;
        if let Some(masks) = masks {
            for row in &mut rows {
                row.mask = masks.join(format!("{}.png", row.name)).to_string_lossy().to_string();
            }
        }
        if (rows.len() as i64) < config.neighbours + 1 {
            bail!("fewer views than neighbours + 1");
        }
        let sparse_array = crate::npz::read_npy(&directory.join("sparse_points.npy"))?;
        if sparse_array.shape.len() != 2 || sparse_array.shape[1] != 3 || sparse_array.shape[0] == 0 {
            bail!("sparse_points.npy must be N x 3");
        }
        let sparse: Vec<[f64; 3]> = sparse_array.to_f64().as_chunks::<3>().0.to_vec();
        let repair_loose = if config.repair_loose != 0 {
            config.repair_loose
        } else {
            (config.hull_allowed + 1).max(round_half_even(0.2 * rows.len() as f64) as i64)
        };
        let cameras = rows
            .iter()
            .map(|r| Camera {
                rotation: r.rotation.map(|row| row.map(|v| v as f32)),
                translation: r.translation.map(|v| v as f32),
                k: r.k.map(|v| v as f32),
            })
            .collect();

        let files: Vec<PathBuf> = rows.iter().flat_map(|r| [PathBuf::from(&r.image), PathBuf::from(&r.mask)]).collect();
        let held = hold_for_pool(&files)?;
        let loaded: Vec<anyhow::Result<(Plane<u8>, [i64; 4])>> = parallel_map(rows.len(), |n| load_view_mask(&rows[n]));
        let (mut masks, mut boxes) = (Vec::new(), Vec::new());
        let pad = config.crop_padding;
        for view in loaded {
            let (mask, [x0, y0, x1, y1]) = view?;
            boxes.push([x0 - pad, y0 - pad, x1 + pad + 1, y1 + pad + 1]);
            masks.push(mask);
        }
        // One common canvas for every view, centred on each mask box.
        let mut cw = boxes.iter().map(|b| b[2] - b[0]).max().unwrap();
        let mut ch = boxes.iter().map(|b| b[3] - b[1]).max().unwrap();
        cw += cw % 2;
        ch += ch % 2;
        for b in &mut boxes {
            let (x, y) = ((b[0] + b[2] - cw).div_euclid(2), (b[1] + b[3] - ch).div_euclid(2));
            *b = [x, y, x + cw, y + ch];
        }
        let whole: usize = masks.iter().map(|m| m.width * m.height * std::mem::size_of::<f32>()).sum();
        let (mx, my) = (cw / 4, ch / 4);
        let keep = |n: usize| (whole > WHOLE_GRAY_BUDGET).then(|| [boxes[n][0] - mx, boxes[n][1] - my, boxes[n][2] + mx, boxes[n][3] + my]);
        let grays: Vec<anyhow::Result<(Plane<f32>, [i64; 2])>> =
            parallel_map(rows.len(), |n| load_view_gray(&rows[n], &masks[n], &config.stretch_percentiles, keep(n)));
        release(held);
        let (mut gray, mut gray_origin) = (Vec::new(), Vec::new());
        for view in grays {
            let (plane, origin) = view?;
            gray.push(plane);
            gray_origin.push(origin);
        }

        // Viewing directions from the median sparse point, in float64 like NumPy.
        let mut median = [0.0; 3];
        for (axis, value) in median.iter_mut().enumerate() {
            let mut column: Vec<f64> = sparse.iter().map(|p| p[axis]).collect();
            *value = median_f64(&mut column);
        }
        let rays: Vec<[f64; 3]> = rows
            .iter()
            .map(|row| {
                let (r, t) = (&row.rotation, &row.translation);
                let mut ray = [0.0; 3];
                for (axis, value) in ray.iter_mut().enumerate() {
                    *value = -(r[0][axis] * t[0] + r[1][axis] * t[1] + r[2][axis] * t[2]) - median[axis];
                }
                let norm = (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt();
                ray.map(|v| v / norm)
            })
            .collect();
        let angles = rays
            .iter()
            .map(|a| rays.iter().map(|b| (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0).acos().to_degrees()).collect())
            .collect();
        Ok(Inputs { rows, cameras, gray, gray_origin, masks, boxes, longest: cw.max(ch), sparse, angles, repair_loose })
    }

    /// Keeps of every grey image only the part inside its canvas box. The
    /// pyramid levels read nothing else, and megapixel photos of a small
    /// object are mostly backdrop; mask repair and the sheets, which look at
    /// whole photos, must have run before.
    pub fn crop_gray(&mut self) {
        for n in 0..self.gray.len() {
            let b = self.boxes[n];
            let (width, height) = (self.masks[n].width as i64, self.masks[n].height as i64);
            let (x0, y0) = (b[0].clamp(0, width), b[1].clamp(0, height));
            let (x1, y1) = (b[2].clamp(0, width), b[3].clamp(0, height));
            let [ox, oy] = self.gray_origin[n];
            let plane = &self.gray[n];
            if [ox, oy] == [x0, y0] && plane.width as i64 == (x1 - x0).max(0) && plane.height as i64 == (y1 - y0).max(0) {
                continue;
            }
            // The plane holds at least the canvas part of the photo (whole, or loaded with a margin).
            let (w, h) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
            let mut data = Vec::with_capacity(w * h);
            for y in 0..h {
                let start = (y0 - oy + y as i64) as usize * plane.width + (x0 - ox) as usize;
                data.extend_from_slice(&plane.data[start..start + w]);
            }
            self.gray[n] = Plane { width: w, height: h, data };
            self.gray_origin[n] = [x0, y0];
        }
    }

    /// The stretched grey value of a photo pixel, `None` where the grey plane holds no data.
    pub fn gray_at(&self, view: usize, x: usize, y: usize) -> Option<f32> {
        let [ox, oy] = self.gray_origin[view];
        let (gx, gy) = (x as i64 - ox, y as i64 - oy);
        let plane = &self.gray[view];
        (gx >= 0 && gy >= 0 && (gx as usize) < plane.width && (gy as usize) < plane.height)
            .then(|| plane.data[gy as usize * plane.width + gx as usize])
    }

    pub fn count(&self) -> usize {
        self.rows.len()
    }

    /// Up to `count` nearest views within the allowed angle range, nearest first.
    pub fn neighbours(&self, view: usize, count: usize, config: &DenseConfig) -> anyhow::Result<Vec<usize>> {
        let angles = &self.angles[view];
        let mut order: Vec<usize> = (0..angles.len()).collect();
        order.sort_by(|&a, &b| angles[a].total_cmp(&angles[b]).then(a.cmp(&b)));
        let order: Vec<usize> =
            order.into_iter().filter(|&j| j != view && config.minimum_angle <= angles[j] && angles[j] <= config.maximum_angle).collect();
        if order.len() < 2 {
            return Err(anyhow!("view has fewer than two neighbours in the allowed angle range: {}", self.rows[view].name));
        }
        Ok(order.into_iter().take(count).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_and_percentiles_follow_python() {
        assert_eq!(round_half_even(14.5), 14.0);
        assert_eq!(round_half_even(15.5), 16.0);
        assert_eq!(round_half_even(14.6), 15.0);
        assert_eq!(round_half_even(-0.5), 0.0);
        // np.percentile([0, 1, 2, 3, 10], [1, 50, 99]) == [0.04, 2.0, 9.72]
        let values = [0.0, 1.0, 2.0, 3.0, 10.0];
        assert!((percentile_sorted_f64(&values, 1.0) - 0.04).abs() < 1e-15);
        assert_eq!(percentile_sorted_f64(&values, 50.0), 2.0);
        assert!((percentile_sorted_f64(&values, 99.0) - 9.72).abs() < 1e-12);
        assert_eq!(percentile_sorted_f64(&values, 100.0), 10.0);
        assert_eq!(median_f64(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn parallel_map_keeps_order() {
        assert_eq!(parallel_map(100, |n| n * n), (0..100).map(|n| n * n).collect::<Vec<_>>());
        assert!(parallel_map(0, |n| n).is_empty());
    }

    #[test]
    fn luma_matches_pillow() {
        assert_eq!(pil_luma(255, 255, 255), 255);
        assert_eq!(pil_luma(255, 0, 0), 76);
        assert_eq!(pil_luma(0, 255, 0), 150);
        assert_eq!(pil_luma(0, 0, 255), 29);
    }
}
