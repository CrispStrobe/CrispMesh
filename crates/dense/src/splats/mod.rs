//! 2D Gaussian splatting (2DGS) on WebGPU: flat Gaussian disks optimised by rendering the photos, with surface terms
//! (depth distortion, agreement of the disks' normals with the normals of the rendered depth), after gsplat's 2DGS
//! (Apache-2.0, commit 937e299) and its example trainer, written anew. Checked against a dense PyTorch renderer of
//! the same model (`scripts/surface_splats/splat_reference.py`).
//!
//! Rendering one view: every splat is projected (`splat_project`), the splats are sorted by depth, each writes one
//! (tile, splat) pair per 16-pixel tile it covers in that order, the pairs are sorted stably by tile (so each tile's
//! splats stay front to back), and each tile is composited (`splat_raster`). The sort is a radix sort of 4-bit
//! digits (count, scan, stable scatter), from plain WebGPU without subgroup operations.

use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};

use crate::gpu::{Gpu, Kernel};

#[cfg(test)]
mod tests;

/// Floats per projected splat (see `splat_project.wgsl`).
pub const PROJECTED: u32 = 24;
/// Floats per rendered pixel (see `splat_raster.wgsl`).
pub const PIXEL: u32 = 16;
/// Floats per pixel of image gradients (see `splat_raster_backward.wgsl`).
pub const DPIXEL: u32 = 12;
/// Floats per splat of projected-value gradients (see `splat_raster_backward.wgsl`).
pub const DSPLAT: u32 = 20;
pub const TILE: u32 = 16;
/// Spherical-harmonics coefficients per splat (degree 3) times 3 colours.
pub const SH: u32 = 48;

/// Floats per splat of the trainable parameters: mean (3), quaternion w, x, y, z (4), log scales (3), logit
/// opacity, spherical harmonics (16 x 3, the first three the constant term), unused. Gradients and Adam moments use
/// the same layout, so one kernel updates everything and densification copies whole rows.
pub const PARAMS: u32 = 60;
/// Columns of each parameter group in a row of [`PARAMS`].
pub const MEAN: usize = 0;
pub const QUAT: usize = 3;
pub const SCALE: usize = 7;
pub const OPACITY: usize = 10;
pub const SH0: usize = 11;
pub const SHN: usize = 14;

/// The trainable splats.
pub struct Splats {
    pub count: u32,
    pub params: wgpu::Buffer,
}

/// One row of [`PARAMS`] per splat from separate arrays (SH: 16 x 3 per splat).
pub fn pack(means: &[f32], quats: &[f32], scales: &[f32], opacities: &[f32], sh: &[f32]) -> Result<Vec<f32>> {
    let count = opacities.len();
    ensure!(
        means.len() == count * 3 && quats.len() == count * 4 && scales.len() == count * 3 && sh.len() == count * SH as usize,
        "splat arrays differ in length"
    );
    let mut rows = vec![0f32; count * PARAMS as usize];
    for g in 0..count {
        let row = &mut rows[g * PARAMS as usize..(g + 1) * PARAMS as usize];
        row[MEAN..MEAN + 3].copy_from_slice(&means[g * 3..g * 3 + 3]);
        row[QUAT..QUAT + 4].copy_from_slice(&quats[g * 4..g * 4 + 4]);
        row[SCALE..SCALE + 3].copy_from_slice(&scales[g * 3..g * 3 + 3]);
        row[OPACITY] = opacities[g];
        row[SH0..SH0 + SH as usize].copy_from_slice(&sh[g * SH as usize..(g + 1) * SH as usize]);
    }
    Ok(rows)
}

impl Splats {
    pub fn new(gpu: &Gpu, rows: &[f32]) -> Result<Self> {
        ensure!(rows.len() % PARAMS as usize == 0, "splat rows must have {PARAMS} floats");
        Ok(Splats { count: (rows.len() / PARAMS as usize) as u32, params: gpu.upload("splats", rows) })
    }
}

/// A pinhole camera: camera = rotation world + translation, pixel centres at integer + 0.5.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub rotation: [[f32; 3]; 3],
    pub translation: [f32; 3],
    /// fx, fy, cx, cy
    pub intrinsics: [f32; 4],
    pub width: u32,
    pub height: u32,
}

impl Camera {
    pub fn centre(&self) -> [f32; 3] {
        let (r, t) = (self.rotation, self.translation);
        std::array::from_fn(|i| -(0..3).map(|j| r[j][i] * t[j]).sum::<f32>())
    }
    pub fn tiles(&self) -> (u32, u32) {
        (self.width.div_ceil(TILE), self.height.div_ceil(TILE))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraParams {
    rotation: [[f32; 4]; 3],
    intrinsics: [f32; 4],
    position: [f32; 4],
    count: u32,
    width: u32,
    height: u32,
    degree: u32,
    tiles_x: u32,
    tiles_y: u32,
    near: f32,
    far: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Quad {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

pub struct Kernels {
    project: Kernel,
    scan: Kernel,
    radix_count: Kernel,
    radix_scatter: Kernel,
    iota: Kernel,
    gather: Kernel,
    emit: Kernel,
    ranges: Kernel,
    raster: Kernel,
    raster_backward: Kernel,
    project_backward: Kernel,
    loss_prepare: Kernel,
    blur: Kernel,
    ssim: Kernel,
    surface: Kernel,
    loss_grad: Kernel,
    surface_backward: Kernel,
    median_grad: Kernel,
}

impl Kernels {
    pub async fn new(gpu: &Gpu) -> Result<Self> {
        Ok(Kernels {
            project: gpu.kernel("splat_project", include_str!("../shaders/splat_project.wgsl"), "main").await?,
            scan: gpu.kernel("splat_scan", include_str!("../shaders/splat_scan.wgsl"), "main").await?,
            radix_count: gpu.kernel("splat_radix_count", include_str!("../shaders/splat_radix_count.wgsl"), "main").await?,
            radix_scatter: gpu.kernel("splat_radix_scatter", include_str!("../shaders/splat_radix_scatter.wgsl"), "main").await?,
            iota: gpu.kernel("splat_iota", include_str!("../shaders/splat_iota.wgsl"), "main").await?,
            gather: gpu.kernel("splat_gather", include_str!("../shaders/splat_gather.wgsl"), "main").await?,
            emit: gpu.kernel("splat_emit", include_str!("../shaders/splat_emit.wgsl"), "main").await?,
            ranges: gpu.kernel("splat_ranges", include_str!("../shaders/splat_ranges.wgsl"), "main").await?,
            raster: gpu.kernel("splat_raster", include_str!("../shaders/splat_raster.wgsl"), "main").await?,
            raster_backward: gpu.kernel("splat_raster_backward", include_str!("../shaders/splat_raster_backward.wgsl"), "main").await?,
            project_backward: gpu.kernel("splat_project_backward", include_str!("../shaders/splat_project_backward.wgsl"), "main").await?,
            loss_prepare: gpu.kernel("splat_loss_prepare", include_str!("../shaders/splat_loss_prepare.wgsl"), "main").await?,
            blur: gpu.kernel("splat_blur", include_str!("../shaders/splat_blur.wgsl"), "main").await?,
            ssim: gpu.kernel("splat_ssim", include_str!("../shaders/splat_ssim.wgsl"), "main").await?,
            surface: gpu.kernel("splat_surface", include_str!("../shaders/splat_surface.wgsl"), "main").await?,
            loss_grad: gpu.kernel("splat_loss_grad", include_str!("../shaders/splat_loss_grad.wgsl"), "main").await?,
            surface_backward: gpu.kernel("splat_surface_backward", include_str!("../shaders/splat_surface_backward.wgsl"), "main").await?,
            median_grad: gpu.kernel("splat_median_grad", include_str!("../shaders/splat_median_grad.wgsl"), "main").await?,
        })
    }
}

/// Keys and values to sort, and their ping-pong partners.
pub struct SortBuffers {
    pub keys: [wgpu::Buffer; 2],
    pub values: [wgpu::Buffer; 2],
    histogram: wgpu::Buffer,
    total: wgpu::Buffer,
    capacity: u32,
}

impl SortBuffers {
    pub fn new(gpu: &Gpu, capacity: u32) -> Self {
        let capacity = capacity.max(1);
        let words = capacity as u64 * 4;
        SortBuffers {
            keys: [gpu.zeroed("sort keys a", words), gpu.zeroed("sort keys b", words)],
            values: [gpu.zeroed("sort values a", words), gpu.zeroed("sort values b", words)],
            histogram: gpu.zeroed("sort histogram", capacity.div_ceil(1024) as u64 * 16 * 4),
            total: gpu.zeroed("sort total", 4),
            capacity,
        }
    }
}

/// Exclusive prefix sum of the first `count` values in place; the sum lands in `total[0]`.
async fn scan(gpu: &Gpu, kernels: &Kernels, values: &wgpu::Buffer, total: &wgpu::Buffer, count: u32) -> Result<()> {
    let uniform = gpu.uniform("splat scan", &Quad { a: count, b: 0, c: 0, d: 0 });
    gpu.run(&kernels.scan, &[&uniform, values, total], crate::gpu::WORKGROUP as u64).await
}

/// Stable radix sort of `count` keys (their low `bits` bits) with their values, starting in buffers [0]; returns the
/// index of the pair of buffers that holds the result.
pub async fn radix_sort(gpu: &Gpu, kernels: &Kernels, sort: &SortBuffers, count: u32, bits: u32) -> Result<usize> {
    ensure!(count <= sort.capacity, "{count} keys exceed the sort capacity {}", sort.capacity);
    let blocks = count.div_ceil(1024);
    let mut current = 0;
    if count == 0 {
        return Ok(current);
    }
    for pass in 0..bits.div_ceil(4) {
        let uniform = gpu.uniform("splat radix", &Quad { a: count, b: pass * 4, c: blocks, d: 0 });
        let groups = blocks as u64 * crate::gpu::WORKGROUP as u64;
        gpu.run(&kernels.radix_count, &[&uniform, &sort.keys[current], &sort.histogram], groups).await?;
        scan(gpu, kernels, &sort.histogram, &sort.total, blocks * 16).await?;
        gpu.run(
            &kernels.radix_scatter,
            &[&uniform, &sort.keys[current], &sort.values[current], &sort.histogram, &sort.keys[1 - current], &sort.values[1 - current]],
            groups,
        )
        .await?;
        current = 1 - current;
    }
    Ok(current)
}

/// Buffers for rendering one view of `count` splats; intersections grow as needed.
pub struct Frame {
    pub projected: wgpu::Buffer,
    pub depth_sort: SortBuffers,
    counts: wgpu::Buffer,
    pub tile_sort: SortBuffers,
    pub ranges: wgpu::Buffer,
    pub pixels: wgpu::Buffer,
    /// Where the depth order and the tile-sorted pairs ended up after the last render.
    pub order: usize,
    pub pairs: usize,
    pub intersections: u32,
}

impl Frame {
    pub fn new(gpu: &Gpu, count: u32, camera: &Camera) -> Self {
        let (tx, ty) = camera.tiles();
        Frame {
            projected: gpu.zeroed("splat projected", count.max(1) as u64 * PROJECTED as u64 * 4),
            depth_sort: SortBuffers::new(gpu, count),
            counts: gpu.zeroed("splat counts", count.max(1) as u64 * 4),
            tile_sort: SortBuffers::new(gpu, count.max(1) * 4),
            ranges: gpu.zeroed("splat ranges", (tx * ty) as u64 * 2 * 4),
            pixels: gpu.zeroed("splat pixels", (camera.width * camera.height) as u64 * PIXEL as u64 * 4),
            order: 0,
            pairs: 0,
            intersections: 0,
        }
    }
}

fn camera_params(camera: &Camera, count: u32, degree: u32) -> CameraParams {
    let (tiles_x, tiles_y) = camera.tiles();
    let (r, t, c) = (camera.rotation, camera.translation, camera.centre());
    CameraParams {
        rotation: std::array::from_fn(|i| [r[i][0], r[i][1], r[i][2], t[i]]),
        intrinsics: camera.intrinsics,
        position: [c[0], c[1], c[2], 0.0],
        count,
        width: camera.width,
        height: camera.height,
        degree,
        tiles_x,
        tiles_y,
        near: 0.2,
        far: 1e10,
    }
}

/// Renders the splats into `frame.pixels` (spherical harmonics up to `degree`); returns the number of intersections.
pub async fn render(gpu: &Gpu, kernels: &Kernels, splats: &Splats, camera: &Camera, degree: u32, frame: &mut Frame) -> Result<u32> {
    let n = splats.count;
    let (tiles_x, tiles_y) = camera.tiles();
    let params = camera_params(camera, n, degree);
    let uniform = gpu.uniform("splat camera", &params);
    gpu.run(&kernels.project, &[&uniform, &splats.params, &frame.projected, &frame.depth_sort.keys[0]], n as u64).await?;
    let count = gpu.uniform("splat count", &Quad { a: n, b: tiles_x, c: 0, d: 0 });
    gpu.run(&kernels.iota, &[&count, &frame.depth_sort.values[0]], n as u64).await?;
    frame.order = radix_sort(gpu, kernels, &frame.depth_sort, n, 32).await?;
    let order = &frame.depth_sort.values[frame.order];
    gpu.run(&kernels.gather, &[&count, order, &frame.projected, &frame.counts], n as u64).await?;
    scan(gpu, kernels, &frame.counts, &frame.tile_sort.total, n).await?;
    let total: Vec<u32> = gpu.read(&frame.tile_sort.total, 1).await?;
    let intersections = total[0];
    if intersections > frame.tile_sort.capacity {
        frame.tile_sort = SortBuffers::new(gpu, intersections + intersections / 4);
    }
    gpu.run(
        &kernels.emit,
        &[&count, order, &frame.projected, &frame.counts, &frame.tile_sort.keys[0], &frame.tile_sort.values[0]],
        n as u64,
    )
    .await?;
    let bits = 32 - (tiles_x * tiles_y).saturating_sub(1).leading_zeros();
    frame.pairs = radix_sort(gpu, kernels, &frame.tile_sort, intersections, bits.max(1)).await?;
    gpu.clear(&frame.ranges);
    let uniform = gpu.uniform("splat ranges", &Quad { a: intersections, b: 0, c: 0, d: 0 });
    gpu.run(&kernels.ranges, &[&uniform, &frame.tile_sort.keys[frame.pairs], &frame.ranges], intersections as u64).await?;
    let uniform = gpu.uniform("splat raster", &Quad { a: camera.width, b: camera.height, c: tiles_x, d: 0 });
    gpu.run(
        &kernels.raster,
        &[&uniform, &frame.ranges, &frame.tile_sort.values[frame.pairs], &frame.projected, &frame.pixels],
        (tiles_x * tiles_y) as u64 * crate::gpu::WORKGROUP as u64,
    )
    .await?;
    frame.intersections = intersections;
    Ok(intersections)
}

/// Adds the gradients of the projected splats (`grads`, [`DSPLAT`] floats each, cleared by the caller) from the
/// image gradients `dpixels` ([`DPIXEL`] floats per pixel) of the last [`render`] into `frame`.
pub async fn raster_backward(
    gpu: &Gpu,
    kernels: &Kernels,
    camera: &Camera,
    frame: &Frame,
    dpixels: &wgpu::Buffer,
    grads: &wgpu::Buffer,
) -> Result<()> {
    let (tiles_x, tiles_y) = camera.tiles();
    let uniform = gpu.uniform("splat raster backward", &Quad { a: camera.width, b: camera.height, c: tiles_x, d: 0 });
    gpu.run(
        &kernels.raster_backward,
        &[&uniform, &frame.ranges, &frame.tile_sort.values[frame.pairs], &frame.projected, &frame.pixels, dpixels, grads],
        (tiles_x * tiles_y) as u64 * crate::gpu::WORKGROUP as u64,
    )
    .await
}

/// Adds the parameters' gradients (`out`, [`PARAMS`] floats per splat) from the projected values' gradients `grads`.
pub async fn project_backward(
    gpu: &Gpu,
    kernels: &Kernels,
    splats: &Splats,
    camera: &Camera,
    degree: u32,
    frame: &Frame,
    grads: &wgpu::Buffer,
    out: &wgpu::Buffer,
) -> Result<()> {
    let uniform = gpu.uniform("splat camera", &camera_params(camera, splats.count, degree));
    gpu.run(&kernels.project_backward, &[&uniform, &splats.params, &frame.projected, grads, out], splats.count as u64).await
}

/// Weights of the loss terms (our trainer's, after gsplat's 2DGS example): `l1` L1 + `ssim` (1 - SSIM) of the
/// colour against the photo (background outside the mask), `mask` L1 of the opacity against the mask, `normal`
/// disagreement of the splats' normals with the median depth's, `distortion` depth distortion.
#[derive(Clone, Copy, Debug)]
pub struct Weights {
    pub l1: f32,
    pub ssim: f32,
    pub mask: f32,
    pub normal: f32,
    pub distortion: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights { l1: 0.8, ssim: 0.2, mask: 0.1, normal: 0.05, distortion: 0.01 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageParams {
    width: u32,
    height: u32,
    pad: [u32; 2],
    a: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewParams {
    width: u32,
    height: u32,
    pad: [u32; 2],
    rotation: [[f32; 4]; 3],
    intrinsics: [f32; 4],
    origin: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LossParams {
    width: u32,
    height: u32,
    pad: [u32; 2],
    weights: [f32; 4],
    background: [f32; 4],
    rotation: [[f32; 4]; 3],
}

/// Per-pixel buffers of the loss for one image size.
pub struct Loss {
    /// The photo as RGBA8 with the mask in alpha.
    pub photo: wgpu::Buffer,
    maps: wgpu::Buffer,
    blur_tmp: wgpu::Buffer,
    stats: wgpu::Buffer,
    ssim_grads: wgpu::Buffer,
    ssim_blurred: wgpu::Buffer,
    pub ssim: wgpu::Buffer,
    surface: wgpu::Buffer,
    dsurface: wgpu::Buffer,
    ddiff: wgpu::Buffer,
    /// Gradients of the rendered images ([`DPIXEL`] per pixel), for [`raster_backward`].
    pub dpixels: wgpu::Buffer,
    /// Per pixel: |b - a| over the channels, -, |alpha - mask|, 1 - n . s alpha, distortion (SSIM in `ssim`).
    pub terms: wgpu::Buffer,
}

impl Loss {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let p = (width * height) as u64;
        let f = |label: &str, floats: u64| gpu.zeroed(label, floats * 4);
        Loss {
            photo: f("splat photo", p),
            maps: f("splat maps", p * 15),
            blur_tmp: f("splat blur", p * 15),
            stats: f("splat stats", p * 15),
            ssim_grads: f("splat ssim grads", p * 9),
            ssim_blurred: f("splat ssim blurred", p * 9),
            ssim: f("splat ssim", p),
            surface: f("splat surface", p * 8),
            dsurface: f("splat dsurface", p * 4),
            ddiff: f("splat ddiff", p * 6),
            dpixels: f("splat dpixels", p * DPIXEL as u64),
            terms: f("splat terms", p * 5),
        }
    }
}

fn view_params(camera: &Camera) -> ViewParams {
    let (r, c) = (camera.rotation, camera.centre());
    ViewParams {
        width: camera.width,
        height: camera.height,
        pad: [0; 2],
        rotation: std::array::from_fn(|i| [r[i][0], r[i][1], r[i][2], 0.0]),
        intrinsics: camera.intrinsics,
        origin: [c[0], c[1], c[2], 0.0],
    }
}

async fn blur(
    gpu: &Gpu,
    kernels: &Kernels,
    camera: &Camera,
    planes: u32,
    source: &wgpu::Buffer,
    tmp: &wgpu::Buffer,
    out: &wgpu::Buffer,
) -> Result<()> {
    let count = (camera.width * camera.height * planes) as u64;
    for (axis, from, to) in [(0, source, tmp), (1, tmp, out)] {
        let uniform = gpu.uniform("splat blur", &Quad { a: camera.width, b: camera.height, c: planes, d: axis });
        gpu.run(&kernels.blur, &[&uniform, from, to], count).await?;
    }
    Ok(())
}

/// The loss of the rendering in `frame` against `loss.photo`, and its gradients in `loss.dpixels`.
pub async fn loss_backward(
    gpu: &Gpu,
    kernels: &Kernels,
    camera: &Camera,
    frame: &Frame,
    loss: &Loss,
    weights: &Weights,
    background: [f32; 3],
) -> Result<()> {
    let p = (camera.width * camera.height) as u64;
    let bg = [background[0], background[1], background[2], 0.0];
    let uniform = gpu.uniform("splat prepare", &ImageParams { width: camera.width, height: camera.height, pad: [0; 2], a: bg });
    gpu.run(&kernels.loss_prepare, &[&uniform, &frame.pixels, &loss.photo, &loss.maps], p).await?;
    blur(gpu, kernels, camera, 15, &loss.maps, &loss.blur_tmp, &loss.stats).await?;
    let scale = -weights.ssim / (3.0 * p as f32);
    let uniform =
        gpu.uniform("splat ssim", &ImageParams { width: camera.width, height: camera.height, pad: [0; 2], a: [scale, 0.0, 0.0, 0.0] });
    gpu.run(&kernels.ssim, &[&uniform, &loss.stats, &loss.ssim_grads, &loss.ssim], p).await?;
    blur(gpu, kernels, camera, 9, &loss.ssim_grads, &loss.blur_tmp, &loss.ssim_blurred).await?;
    let view = gpu.uniform("splat view", &view_params(camera));
    gpu.run(&kernels.surface, &[&view, &frame.pixels, &loss.surface], p).await?;
    let r = camera.rotation;
    let uniform = gpu.uniform(
        "splat loss",
        &LossParams {
            width: camera.width,
            height: camera.height,
            pad: [0; 2],
            weights: [weights.l1, weights.mask, weights.normal, weights.distortion],
            background: bg,
            rotation: std::array::from_fn(|i| [r[i][0], r[i][1], r[i][2], 0.0]),
        },
    );
    gpu.run(
        &kernels.loss_grad,
        &[&uniform, &frame.pixels, &loss.maps, &loss.ssim_blurred, &loss.photo, &loss.surface, &loss.dpixels, &loss.dsurface, &loss.terms],
        p,
    )
    .await?;
    gpu.run(&kernels.surface_backward, &[&view, &frame.pixels, &loss.surface, &loss.dsurface, &loss.ddiff], p).await?;
    gpu.run(&kernels.median_grad, &[&view, &loss.ddiff, &loss.dpixels], p).await
}

/// The photo (RGB, 8 bits) and mask as one RGBA8 word per pixel.
pub fn pack_photo(rgb: &[u8], mask: &[bool]) -> Vec<u32> {
    mask.iter()
        .enumerate()
        .map(|(i, m)| u32::from_le_bytes([rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], if *m { 255 } else { 0 }]))
        .collect()
}
