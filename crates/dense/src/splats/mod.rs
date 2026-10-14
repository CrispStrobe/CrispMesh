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

/// The trainable splats: positions, rotations (w, x, y, z), log scales, logit opacities, SH coefficients.
pub struct Splats {
    pub count: u32,
    pub means: wgpu::Buffer,
    pub quats: wgpu::Buffer,
    pub scales: wgpu::Buffer,
    pub opacities: wgpu::Buffer,
    pub sh: wgpu::Buffer,
}

impl Splats {
    pub fn new(gpu: &Gpu, means: &[f32], quats: &[f32], scales: &[f32], opacities: &[f32], sh: &[f32]) -> Result<Self> {
        let count = opacities.len();
        ensure!(
            means.len() == count * 3 && quats.len() == count * 4 && scales.len() == count * 3 && sh.len() == count * SH as usize,
            "splat arrays differ in length"
        );
        Ok(Splats {
            count: count as u32,
            means: gpu.upload("splat means", means),
            quats: gpu.upload("splat quats", quats),
            scales: gpu.upload("splat scales", scales),
            opacities: gpu.upload("splat opacities", opacities),
            sh: gpu.upload("splat sh", sh),
        })
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

/// Renders the splats into `frame.pixels` (spherical harmonics up to `degree`); returns the number of intersections.
pub async fn render(gpu: &Gpu, kernels: &Kernels, splats: &Splats, camera: &Camera, degree: u32, frame: &mut Frame) -> Result<u32> {
    let n = splats.count;
    let (tiles_x, tiles_y) = camera.tiles();
    let r = camera.rotation;
    let t = camera.translation;
    let c = camera.centre();
    let params = CameraParams {
        rotation: std::array::from_fn(|i| [r[i][0], r[i][1], r[i][2], t[i]]),
        intrinsics: camera.intrinsics,
        position: [c[0], c[1], c[2], 0.0],
        count: n,
        width: camera.width,
        height: camera.height,
        degree,
        tiles_x,
        tiles_y,
        near: 0.2,
        far: 1e10,
    };
    let uniform = gpu.uniform("splat camera", &params);
    gpu.run(
        &kernels.project,
        &[
            &uniform,
            &splats.means,
            &splats.quats,
            &splats.scales,
            &splats.opacities,
            &splats.sh,
            &frame.projected,
            &frame.depth_sort.keys[0],
        ],
        n as u64,
    )
    .await?;
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
