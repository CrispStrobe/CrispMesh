//! Silhouette hull: port of `Hull`, `carve` and `Stereo.build_hull` from
//! `scripts/turntable_mesh/multiscale_stereo.py`.
//!
//! The grid geometry (box, voxel size, shape, origin) is float64 on the CPU as
//! in the reference. Voxel centres are float32 per axis, computed on the CPU
//! with the reference's operation order, so every kernel sees bit-identical
//! points. The coarse-candidate test is separable per axis and is evaluated on
//! the CPU for the same reason. Only the projection into the views runs on the
//! GPU, over lists of candidate voxels rather than a dense volume.

#![allow(clippy::needless_range_loop)]

use anyhow::bail;
use bytemuck::{Pod, Zeroable};
use serde::Serialize;

use crate::config::DenseConfig;
use crate::gpu::{Gpu, Kernel};
use crate::inputs::{percentile_sorted_f64, Camera, Inputs, Plane};

/// Voxels per dispatch.
pub const CHUNK: usize = 1 << 21;

/// A list of voxels of one grid on the device, in chunks one dispatch can take,
/// with the grid's centre coordinates. Kernels rebuild each centre from its linear index.
pub struct VoxelList {
    pub shape: [usize; 3],
    pub axes: wgpu::Buffer,
    /// Buffers of linear indices and the number of voxels in each.
    pub chunks: Vec<(wgpu::Buffer, usize)>,
    pub count: usize,
}

impl VoxelList {
    pub fn new(gpu: &Gpu, hull: &Hull, indices: &[u32]) -> Self {
        VoxelList {
            shape: hull.shape,
            axes: gpu.upload("voxel axes", &hull.axes()),
            chunks: indices.chunks(CHUNK).map(|chunk| (gpu.upload("voxel list", chunk), chunk.len())).collect(),
            count: indices.len(),
        }
    }
}

/// Occupancy grid in world space. `origin` is the corner of voxel (0, 0, 0).
#[derive(Debug, Clone)]
pub struct Hull {
    pub shape: [usize; 3],
    pub origin: [f32; 3],
    /// Edge length as the reference keeps it (a Python float); kernels use `voxel as f32`.
    pub voxel: f64,
    /// One bit per voxel, linear index `(x * ny + y) * nz + z`, least significant bit first.
    pub bits: Vec<u32>,
}

impl Hull {
    pub fn from_flags(flags: &[bool], shape: [usize; 3], origin: [f32; 3], voxel: f64) -> Self {
        Hull { shape, origin, voxel, bits: crate::gpu::pack_bits(flags.iter().copied()) }
    }

    pub fn len(&self) -> usize {
        self.shape[0] * self.shape[1] * self.shape[2]
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn occupied(&self, linear: usize) -> bool {
        (self.bits[linear / 32] >> (linear % 32)) & 1 == 1
    }

    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// `Hull.contains`: float32 `floor((world - origin) / voxel)` per axis.
    #[inline]
    pub fn contains(&self, world: [f32; 3]) -> bool {
        let voxel = self.voxel as f32;
        let mut linear = 0usize;
        for axis in 0..3 {
            let index = ((world[axis] - self.origin[axis]) / voxel).floor();
            if !(index >= 0.0 && index < self.shape[axis] as f32) {
                return false;
            }
            linear = linear * self.shape[axis] + index as usize;
        }
        self.occupied(linear)
    }

    /// Voxel centre coordinates along one axis: `(i + 0.5) * voxel + origin` in float32.
    pub fn axis(&self, axis: usize) -> Vec<f32> {
        let voxel = self.voxel as f32;
        (0..self.shape[axis]).map(|i| (i as f32 + 0.5) * voxel + self.origin[axis]).collect()
    }

    /// The three axes concatenated, as the kernels read them.
    pub fn axes(&self) -> Vec<f32> {
        (0..3).flat_map(|a| self.axis(a)).collect()
    }

    /// Linear indices of occupied voxels in C order (the order of `nonzero()`).
    pub fn indices(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.count());
        for (w, &word) in self.bits.iter().enumerate() {
            let mut rest = word;
            while rest != 0 {
                out.push((w * 32) as u32 + rest.trailing_zeros());
                rest &= rest - 1;
            }
        }
        out
    }

    #[inline]
    pub fn unravel(&self, linear: u32) -> [usize; 3] {
        let linear = linear as usize;
        let (ny, nz) = (self.shape[1], self.shape[2]);
        [linear / (ny * nz), (linear / nz) % ny, linear % nz]
    }

    /// The hull grown by one voxel in every direction (`max_pool3d(…, 3, 1, 1)`).
    pub fn grown(&self) -> Hull {
        let [nx, ny, nz] = self.shape;
        let mut flags: Vec<bool> = (0..self.len()).map(|n| self.occupied(n)).collect();
        let strides = [ny * nz, nz, 1];
        for axis in 0..3 {
            let (stride, extent) = (strides[axis], self.shape[axis]);
            let source = flags.clone();
            for linear in 0..flags.len() {
                let position = (linear / stride) % extent;
                if (position > 0 && source[linear - stride]) || (position + 1 < extent && source[linear + stride]) {
                    flags[linear] = true;
                }
            }
        }
        let _ = nx;
        Hull::from_flags(&flags, self.shape, self.origin, self.voxel)
    }
}

/// Binary dilation with a `(2 * radius + 1)` square, zero outside the image
/// (`max_pool2d(mask, 2r + 1, 1, r) > 0`).
pub fn dilate(mask: &Plane<u8>, radius: usize) -> Plane<u8> {
    spread(mask, radius, 1)
}

/// Binary erosion with the same square; pixels outside the image do not erode
/// (`-max_pool2d(-mask, 2r + 1, 1, r) > 0`).
pub fn erode(mask: &Plane<u8>, radius: usize) -> Plane<u8> {
    spread(mask, radius, 0)
}

/// Spreads `value` pixels by `radius` along rows, then columns.
fn spread(mask: &Plane<u8>, radius: usize, value: u8) -> Plane<u8> {
    if radius == 0 {
        return mask.clone();
    }
    let (w, h) = (mask.width, mask.height);
    let mut rows = mask.clone();
    for y in 0..h {
        spread_line(&mask.data[y * w..(y + 1) * w], 1, w, radius, value, &mut rows.data[y * w..(y + 1) * w]);
    }
    let mut out = rows.clone();
    let mut line = vec![0u8; h];
    let mut column = vec![0u8; h];
    for x in 0..w {
        for y in 0..h {
            column[y] = rows.data[y * w + x];
        }
        spread_line(&column, 1, h, radius, value, &mut line);
        for y in 0..h {
            out.data[y * w + x] = line[y];
        }
    }
    out
}

fn spread_line(source: &[u8], stride: usize, count: usize, radius: usize, value: u8, out: &mut [u8]) {
    let far = usize::MAX / 2;
    let mut distance = far;
    for n in 0..count {
        distance = if source[n * stride] == value { 0 } else { distance.saturating_add(1).min(far) };
        out[n * stride] = if distance <= radius { value } else { 1 - value };
    }
    distance = far;
    for n in (0..count).rev() {
        distance = if source[n * stride] == value { 0 } else { distance.saturating_add(1).min(far) };
        if distance <= radius {
            out[n * stride] = value;
        }
    }
}

/// True when no mask pixel lies on the image border, i.e. the view shows the whole object.
pub fn clear_of_border(mask: &Plane<u8>) -> bool {
    let (w, h) = (mask.width, mask.height);
    let row = |y: usize| mask.data[y * w..(y + 1) * w].iter().any(|&m| m != 0);
    let column = |x: usize| (0..h).any(|y| mask.data[y * w + x] != 0);
    !(row(0) || row(h - 1) || column(0) || column(w - 1))
}

/// Violation of one view by one point, as in `carve`: reference for tests and diagnostics.
pub fn violates(camera: &Camera, mask: &Plane<u8>, whole: bool, point: [f32; 3]) -> bool {
    let p = camera.to_camera(point);
    let z = p[2].max(1e-6);
    let u = (p[0] / z * camera.k[0] + camera.k[2] - 0.5).round_ties_even();
    let v = (p[1] / z * camera.k[1] + camera.k[3] - 0.5).round_ties_even();
    let inside = u >= 0.0 && u < mask.width as f32 && v >= 0.0 && v < mask.height as f32 && p[2] > 0.0;
    let hit = inside && mask.at(u as usize, v as usize) != 0;
    if whole {
        !(inside && hit)
    } else {
        inside && !hit
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CarveParams {
    count: u32,
    views: u32,
    nx: u32,
    ny: u32,
    nz: u32,
    pad: [u32; 3],
}

/// Camera rows as kernels read them: rotation rows with translation in w, then k.
pub fn camera_rows(camera: &Camera) -> [[f32; 4]; 4] {
    let (r, t) = (&camera.rotation, &camera.translation);
    [[r[0][0], r[0][1], r[0][2], t[0]], [r[1][0], r[1][1], r[1][2], t[1]], [r[2][0], r[2][1], r[2][2], t[2]], camera.k]
}

struct MaskBatch {
    views: u32,
    cameras: wgpu::Buffer,
    frames: wgpu::Buffer,
    masks: wgpu::Buffer,
}

/// Dilated masks on the device, split so each batch fits one storage binding.
struct Silhouettes {
    batches: Vec<MaskBatch>,
    views: usize,
}

impl Silhouettes {
    fn new(gpu: &Gpu, cameras: &[Camera], masks: &[Plane<u8>]) -> Self {
        let budget_words = (gpu.binding_budget() / 4) as usize;
        let mut batches = Vec::new();
        let (mut rows, mut frames, mut words) = (Vec::<[f32; 4]>::new(), Vec::<[u32; 4]>::new(), Vec::<u32>::new());
        let flush = |rows: &mut Vec<[f32; 4]>, frames: &mut Vec<[u32; 4]>, words: &mut Vec<u32>, batches: &mut Vec<MaskBatch>| {
            if frames.is_empty() {
                return;
            }
            batches.push(MaskBatch {
                views: frames.len() as u32,
                cameras: gpu.upload("carve cameras", rows),
                frames: gpu.upload("carve frames", frames),
                masks: gpu.upload("carve masks", words),
            });
            rows.clear();
            frames.clear();
            words.clear();
        };
        for (camera, mask) in cameras.iter().zip(masks) {
            let stride = mask.width.div_ceil(32);
            if !frames.is_empty() && words.len() + stride * mask.height > budget_words {
                flush(&mut rows, &mut frames, &mut words, &mut batches);
            }
            frames.push([mask.width as u32, mask.height as u32, words.len() as u32, clear_of_border(mask) as u32]);
            rows.extend_from_slice(&camera_rows(camera));
            let start = words.len();
            words.resize(start + stride * mask.height, 0);
            for y in 0..mask.height {
                let row = &mask.data[y * mask.width..(y + 1) * mask.width];
                for (x, &value) in row.iter().enumerate() {
                    if value != 0 {
                        words[start + y * stride + x / 32] |= 1 << (x % 32);
                    }
                }
            }
        }
        flush(&mut rows, &mut frames, &mut words, &mut batches);
        Silhouettes { batches, views: cameras.len() }
    }
}

struct Carver<'a> {
    gpu: &'a Gpu,
    kernel: Kernel,
    voxels: wgpu::Buffer,
    counts: wgpu::Buffer,
}

impl<'a> Carver<'a> {
    async fn new(gpu: &'a Gpu) -> anyhow::Result<Self> {
        Ok(Carver {
            gpu,
            kernel: gpu.kernel("carve", include_str!("shaders/carve.wgsl"), "main").await?,
            voxels: gpu.zeroed("carve voxels", 4 * CHUNK as u64),
            counts: gpu.zeroed("carve counts", 4 * CHUNK as u64),
        })
    }

    /// Violation counts of the listed voxels (linear indices into `shape`).
    async fn count(&self, silhouettes: &Silhouettes, shape: [usize; 3], axes: &wgpu::Buffer, voxels: &[u32]) -> anyhow::Result<Vec<u32>> {
        let mut out = Vec::with_capacity(voxels.len());
        for chunk in voxels.chunks(CHUNK) {
            self.gpu.write(&self.voxels, chunk);
            self.gpu.clear(&self.counts);
            for batch in &silhouettes.batches {
                let params = self.gpu.uniform(
                    "carve params",
                    &CarveParams {
                        count: chunk.len() as u32,
                        views: batch.views,
                        nx: shape[0] as u32,
                        ny: shape[1] as u32,
                        nz: shape[2] as u32,
                        pad: [0; 3],
                    },
                );
                self.gpu
                    .run(
                        &self.kernel,
                        &[&params, &batch.cameras, &batch.frames, &batch.masks, axes, &self.voxels, &self.counts],
                        chunk.len() as u64,
                    )
                    .await?;
            }
            out.extend(self.gpu.read::<u32>(&self.counts, chunk.len()).await?);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HullReport {
    pub shape: [usize; 3],
    pub voxel: f64,
    pub occupied: usize,
    pub coarse_touches_box: bool,
}

pub struct HullState {
    /// Voxels at most `hull_allowed` views disagree with.
    pub hull: Hull,
    /// The hull grown by one voxel; bounds matching hypotheses.
    pub search: Hull,
    /// Violation count per voxel of the fine grid; the number of views outside the coarse candidate region.
    pub violations: Vec<u16>,
    /// Near and far camera-space depth of the hull per view, padded by three voxels.
    pub bounds: Vec<(f64, f64)>,
    pub report: HullReport,
}

/// `Stereo.build_hull`: a coarse carve to find the object box, then a fine carve inside it.
/// `voxel` is fixed by the first call and reused by later ones (after mask repair).
pub async fn build_hull(gpu: &Gpu, inputs: &Inputs, config: &DenseConfig, voxel: &mut Option<f64>) -> anyhow::Result<HullState> {
    let masks: Vec<Plane<u8>> = crate::inputs::parallel_map(inputs.count(), |n| dilate(&inputs.masks[n], config.hull_dilate as usize));
    let silhouettes = Silhouettes::new(gpu, &inputs.cameras, &masks);
    drop(masks);
    let carver = Carver::new(gpu).await?;
    let views = silhouettes.views;

    // Box of the sparse points, padded; 96 coarse voxels along its longest side.
    let (mut lo, mut hi) = ([0.0f64; 3], [0.0f64; 3]);
    for axis in 0..3 {
        let mut column: Vec<f64> = inputs.sparse.iter().map(|p| p[axis]).collect();
        column.sort_by(f64::total_cmp);
        lo[axis] = percentile_sorted_f64(&column, 1.0);
        hi[axis] = percentile_sorted_f64(&column, 99.0);
    }
    let extent = |lo: &[f64; 3], hi: &[f64; 3]| (0..3).map(|a| hi[a] - lo[a]).fold(f64::MIN, f64::max);
    let pad = 0.3 * extent(&lo, &hi);
    for axis in 0..3 {
        lo[axis] -= pad;
        hi[axis] += pad;
    }
    let coarse = extent(&lo, &hi) / 96.0;
    let coarse_shape = [0, 1, 2].map(|a| ((hi[a] - lo[a]) / coarse).ceil() as usize);
    let coarse_origin = lo.map(|v| v as f32);
    let mut candidate = Hull { shape: coarse_shape, origin: coarse_origin, voxel: coarse, bits: Vec::new() };
    let loose = config.hull_allowed.max(inputs.repair_loose).max(16) as u32;
    let all: Vec<u32> = (0..candidate.len() as u32).collect();
    let axes = gpu.upload("coarse axes", &candidate.axes());
    let counts = carver.count(&silhouettes, coarse_shape, &axes, &all).await?;
    let flags: Vec<bool> = counts.iter().map(|&c| c <= loose).collect();
    candidate.bits = crate::gpu::pack_bits(flags.iter().copied());
    let (mut low, mut high) = ([usize::MAX; 3], [0usize; 3]);
    for linear in candidate.indices() {
        let index = candidate.unravel(linear);
        for axis in 0..3 {
            low[axis] = low[axis].min(index[axis]);
            high[axis] = high[axis].max(index[axis]);
        }
    }
    if low[0] == usize::MAX {
        bail!("empty silhouette hull; check masks and cameras");
    }
    let touching = (0..3).any(|a| low[a] == 0 || high[a] == coarse_shape[a] - 1);
    let candidate = candidate.grown();

    // The fine grid: the occupied coarse box plus one coarse voxel on every side.
    let span = (0..3).map(|a| high[a] - low[a] + 1).max().unwrap();
    let voxel = *voxel.get_or_insert(span as f64 * coarse / config.grid as f64);
    let mut fine_lo = [0.0f64; 3];
    let mut shape = [0usize; 3];
    for axis in 0..3 {
        fine_lo[axis] = lo[axis] + (low[axis] as f64 - 1.0) * coarse;
        let fine_hi = fine_lo[axis] + (high[axis] - low[axis] + 3) as f64 * coarse;
        shape[axis] = ((fine_hi - fine_lo[axis]) / voxel).ceil() as usize;
    }
    if shape.iter().product::<usize>() > u32::MAX as usize {
        bail!("hull grid of {shape:?} voxels is too large");
    }
    let mut hull = Hull { shape, origin: fine_lo.map(|v| v as f32), voxel, bits: Vec::new() };
    let fine_axes: [Vec<f32>; 3] = [hull.axis(0), hull.axis(1), hull.axis(2)];
    // Coarse cell of every fine centre per axis, as `candidate.contains` computes it.
    let coarse32 = coarse as f32;
    let cells: [Vec<Option<usize>>; 3] = [0, 1, 2].map(|a| {
        fine_axes[a]
            .iter()
            .map(|&c| {
                let index = ((c - coarse_origin[a]) / coarse32).floor();
                (index >= 0.0 && index < coarse_shape[a] as f32).then_some(index as usize)
            })
            .collect()
    });
    let axes = gpu.upload("fine axes", &hull.axes());
    let mut violations = vec![views.min(u16::MAX as usize) as u16; hull.len()];
    let mut pending: Vec<u32> = Vec::with_capacity(CHUNK);
    // Counts the pending candidates and stores their violations.
    macro_rules! flush {
        () => {{
            let counts = carver.count(&silhouettes, shape, &axes, &pending).await?;
            for (&linear, &count) in pending.iter().zip(&counts) {
                violations[linear as usize] = count.min(u16::MAX as u32) as u16;
            }
            pending.clear();
        }};
    }
    let (ny, nz) = (shape[1], shape[2]);
    for (i, cx) in cells[0].iter().enumerate() {
        let Some(cx) = cx else { continue };
        for (j, cy) in cells[1].iter().enumerate() {
            let Some(cy) = cy else { continue };
            let row = (cx * coarse_shape[1] + cy) * coarse_shape[2];
            let base = (i * ny + j) * nz;
            for (k, cz) in cells[2].iter().enumerate() {
                if let Some(cz) = cz {
                    if candidate.occupied(row + cz) {
                        pending.push((base + k) as u32);
                    }
                }
            }
            if pending.len() + nz > CHUNK {
                flush!();
            }
        }
    }
    flush!();

    let allowed = config.hull_allowed as u16;
    hull.bits = crate::gpu::pack_bits(violations.iter().map(|&v| v <= allowed));
    let search = hull.grown();
    let occupied = hull.count();

    // Depth range of the hull per view from every seventh occupied voxel.
    let sample: Vec<[f32; 3]> = hull
        .indices()
        .into_iter()
        .step_by(7)
        .map(|linear| {
            let [i, j, k] = hull.unravel(linear);
            [fine_axes[0][i], fine_axes[1][j], fine_axes[2][k]]
        })
        .collect();
    if sample.is_empty() {
        bail!("empty silhouette hull; check masks and cameras");
    }
    let bounds = crate::inputs::parallel_map(views, |n| {
        let camera = &inputs.cameras[n];
        let (mut near, mut far) = (f32::MAX, f32::MIN);
        for &point in &sample {
            let z = camera.to_camera(point)[2];
            near = near.min(z);
            far = far.max(z);
        }
        (near as f64 - 3.0 * voxel, far as f64 + 3.0 * voxel)
    });
    let report = HullReport { shape, voxel, occupied, coarse_touches_box: touching };
    Ok(HullState { hull, search, violations, bounds, report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dilation_and_erosion_match_pooling() {
        let mut mask = Plane::<u8>::new(9, 7);
        mask.data[3 * 9 + 4] = 1;
        mask.data[0] = 1;
        let grown = dilate(&mask, 2);
        for y in 0..7 {
            for x in 0..9 {
                let near = |cx: i64, cy: i64| (x as i64 - cx).abs() <= 2 && (y as i64 - cy).abs() <= 2;
                assert_eq!(grown.at(x, y) != 0, near(4, 3) || near(0, 0), "{x},{y}");
            }
        }
        // Erosion ignores the outside, so a full image stays full and one hole grows.
        let mut full = Plane { width: 9, height: 7, data: vec![1u8; 63] };
        assert_eq!(erode(&full, 3), full);
        full.data[3 * 9 + 4] = 0;
        let eroded = erode(&full, 1);
        assert_eq!(eroded.data.iter().filter(|&&v| v == 0).count(), 9);
        assert!(!clear_of_border(&grown) && !clear_of_border(&eroded));
        mask.data[0] = 0;
        assert!(clear_of_border(&dilate(&mask, 2)) && !clear_of_border(&dilate(&mask, 3)));
    }

    /// GPU carve against the scalar reference on the synthetic sphere (`CRISPMESH_GPU_TESTS=1`).
    #[test]
    fn gpu_hull_matches_the_scalar_carve() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        use crate::stereo::{options, synthetic};
        let root = synthetic::temporary("hull", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        let gpu = Gpu::new().unwrap();
        let mut voxel = None;
        let state = crate::gpu::block_on(build_hull(&gpu, &inputs, &config, &mut voxel)).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        // Hull of a unit sphere seen from a ring: at least the sphere, not much more.
        let volume = state.hull.count() as f64 * state.hull.voxel.powi(3);
        assert!(volume > 4.0 && volume < 7.0, "{volume}");
        assert_eq!(state.report.occupied, state.hull.count());
        assert!(state.search.count() > state.hull.count());
        let masks: Vec<Plane<u8>> = inputs.masks.iter().map(|m| dilate(m, config.hull_dilate as usize)).collect();
        let whole: Vec<bool> = masks.iter().map(clear_of_border).collect();
        let axes = [state.hull.axis(0), state.hull.axis(1), state.hull.axis(2)];
        let (mut checked, mut different) = (0, 0);
        for linear in (0..state.hull.len()).step_by(37) {
            if state.violations[linear] as usize == inputs.count() {
                continue; // outside the coarse candidate region, or violating every view
            }
            let [i, j, k] = state.hull.unravel(linear as u32);
            let point = [axes[0][i], axes[1][j], axes[2][k]];
            let expected = (0..inputs.count()).filter(|&n| violates(&inputs.cameras[n], &masks[n], whole[n], point)).count();
            checked += 1;
            different += (expected != state.violations[linear] as usize) as usize;
        }
        assert!(checked > 5000, "{checked}");
        // Float rounding at pixel boundaries may differ between the GPU and the CPU.
        assert!(different * 2000 <= checked, "{different} of {checked} voxels differ");
        for (near, far) in &state.bounds {
            assert!(*near > 4.5 && *near < 5.2 && *far > 6.0 && *far < 7.6, "{near} {far}");
        }
    }

    #[test]
    fn hull_indexing_and_growth() {
        let shape = [3, 4, 5];
        let mut flags = vec![false; 60];
        flags[(4 + 2) * 5 + 3] = true; // voxel (1, 2, 3)
        let hull = Hull::from_flags(&flags, shape, [1.0, 2.0, 3.0], 0.5);
        assert_eq!(hull.indices(), vec![33]);
        assert_eq!(hull.unravel(33), [1, 2, 3]);
        assert!(hull.contains([1.75, 3.25, 4.75]));
        assert!(!hull.contains([1.25, 3.25, 4.75]));
        assert!(!hull.contains([0.9, 3.25, 4.75]) && !hull.contains([1.75, 3.25, 5.6]));
        assert_eq!(hull.axis(2), vec![3.25, 3.75, 4.25, 4.75, 5.25]);
        let grown = hull.grown();
        assert_eq!(grown.count(), 27);
        assert!(grown.occupied(5 + 2) && grown.occupied(2 * 20 + 3 * 5 + 4));
    }
}
