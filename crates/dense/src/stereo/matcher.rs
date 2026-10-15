//! Photo-consistency matching on the GPU: ports of `Stereo.score`, `ncc`,
//! `_aggregate`, `_peak`, `sweep` and `refine`.
//!
//! Per reference view the cost volume (hypotheses x pixels) stays on the
//! device. Hypotheses are processed in chunks sized so that every binding fits
//! the default WebGPU limit; for each chunk every neighbour is warped and
//! scored in turn, keeping the running best-N per pixel, then the volume is
//! blurred along each hypothesis slice and reduced to one depth per pixel.
//! Only that depth map is read back.

#![allow(clippy::needless_range_loop)]

use anyhow::{anyhow, bail};
use bytemuck::{Pod, Zeroable};

use crate::config::DenseConfig;
use crate::gpu::{pack_bits, Gpu, Kernel};
use crate::hull::{camera_rows, Hull};
use crate::inputs::Plane;

use super::depth::{gauss_weights, linspace};
use super::level::LevelView;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WarpParams {
    reference: [[f32; 4]; 4],
    neighbour: [[f32; 4]; 4],
    hull: [f32; 4],
    grid: [f32; 4],
    shape: [u32; 4],
    size: [u32; 4],
    chunk: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct NccParams {
    size: [u32; 4],
    flags: [u32; 4],
    gates: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RowParams {
    size: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AggregateParams {
    size: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PeakParams {
    size: [u32; 4],
    values: [f32; 4],
    confidence: [f32; 4],
}

/// The views of one pyramid level on the device.
/// The depth map of one view on its way back from the GPU.
pub struct Pending {
    width: usize,
    height: usize,
    read: crate::gpu::PendingRead<f32>,
}

pub struct LevelBuffers {
    gray: Vec<wgpu::Buffer>,
    mask: Vec<wgpu::Buffer>,
}

/// Working buffers for one reference size and hypothesis count.
struct Work {
    pixels: usize,
    hypotheses: usize,
    chunk: usize,
    warped: wgpu::Buffer,
    sums: wgpu::Buffer,
    best: wgpu::Buffer,
    volume: wgpu::Buffer,
    numerator: wgpu::Buffer,
    denominator: wgpu::Buffer,
    depth: wgpu::Buffer,
}

pub struct Matcher<'a> {
    gpu: &'a Gpu,
    config: &'a DenseConfig,
    warp: Kernel,
    ncc_rows: Kernel,
    ncc_columns: Kernel,
    blur_rows: Kernel,
    blur_columns: Kernel,
    peak: Kernel,
    hull: wgpu::Buffer,
    hull_params: ([f32; 4], [u32; 3]),
    work: Option<Work>,
    coherent: Option<super::planes::Planes>,
    search: Hull,
}

impl<'a> Matcher<'a> {
    /// `search` is the grown hull that bounds every hypothesis.
    pub async fn new(gpu: &'a Gpu, config: &'a DenseConfig, search: &Hull) -> anyhow::Result<Self> {
        let aggregate = include_str!("../shaders/aggregate.wgsl");
        Ok(Matcher {
            gpu,
            config,
            warp: gpu.kernel("warp", include_str!("../shaders/warp.wgsl"), "main").await?,
            ncc_rows: gpu.kernel("ncc rows", include_str!("../shaders/ncc_rows.wgsl"), "main").await?,
            ncc_columns: gpu.kernel("ncc columns", include_str!("../shaders/ncc_columns.wgsl"), "main").await?,
            blur_rows: gpu.kernel("aggregate rows", aggregate, "horizontal").await?,
            blur_columns: gpu.kernel("aggregate columns", aggregate, "vertical").await?,
            peak: gpu.kernel("peak", include_str!("../shaders/peak.wgsl"), "main").await?,
            hull: gpu.upload("search hull", &search.bits),
            hull_params: (
                [search.origin[0], search.origin[1], search.origin[2], search.voxel as f32],
                [search.shape[0] as u32, search.shape[1] as u32, search.shape[2] as u32],
            ),
            work: None,
            coherent: if config.coherent_band { Some(super::planes::Planes::independent(gpu, search).await?) } else { None },
            search: search.clone(),
        })
    }

    pub fn upload(&self, level: &[LevelView]) -> LevelBuffers {
        LevelBuffers {
            gray: level.iter().map(|v| self.gpu.upload("level gray", &v.gray.data)).collect(),
            mask: level
                .iter()
                .map(|v| {
                    // One row per `ceil(width / 32)` words, as the kernels index it.
                    let stride = v.width.div_ceil(32);
                    let mut words = vec![0u32; stride * v.height];
                    for y in 0..v.height {
                        let row = pack_bits(v.mask.data[y * v.width..(y + 1) * v.width].iter().map(|&m| m != 0));
                        words[y * stride..y * stride + row.len()].copy_from_slice(&row);
                    }
                    self.gpu.upload("level mask", &words)
                })
                .collect(),
        }
    }

    /// Buffers for `hypotheses` slices of `pixels` pixels; reused while the sizes stay the same.
    fn prepare(&mut self, pixels: usize, hypotheses: usize) -> anyhow::Result<()> {
        if matches!(&self.work, Some(w) if w.pixels == pixels && w.hypotheses == hypotheses) {
            return Ok(());
        }
        self.work = None;
        let budget = self.gpu.binding_budget() as usize;
        let plane = 4 * pixels;
        let best_of = self.config.best_of as usize;
        // Per chunk: six row sums per pixel and the best-N list.
        let chunk = (budget / (6 * plane)).min(budget / (best_of * plane)).min(hypotheses);
        if chunk == 0 || hypotheses * plane > budget {
            bail!(
                "a level of {pixels} pixels with {hypotheses} hypotheses does not fit the GPU's storage bindings ({} MiB each)",
                budget >> 20
            );
        }
        let bytes = |slices: usize| (slices * plane) as u64;
        self.work = Some(Work {
            pixels,
            hypotheses,
            chunk,
            warped: self.gpu.zeroed("warped", bytes(chunk)),
            sums: self.gpu.zeroed("row sums", bytes(chunk * 6)),
            best: self.gpu.zeroed("best", bytes(chunk * best_of)),
            volume: self.gpu.zeroed("volume", bytes(hypotheses)),
            numerator: self.gpu.zeroed("blur numerator", bytes(hypotheses)),
            denominator: self.gpu.zeroed("blur denominator", bytes(hypotheses)),
            depth: self.gpu.zeroed("depth", bytes(1)),
        });
        Ok(())
    }

    /// Fills the cost volume: `Stereo.score` for every hypothesis. `offsets`
    /// are inverse depths (sweep) or offsets added to `1 / init` (refine).
    #[allow(clippy::too_many_arguments)]
    async fn score(
        &mut self,
        buffers: &LevelBuffers,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        window: i64,
        refine: bool,
        init: &wgpu::Buffer,
        offsets: &[f32],
    ) -> anyhow::Result<()> {
        let reference = &level[view];
        let pixels = reference.width * reference.height;
        self.prepare(pixels, offsets.len())?;
        let work = self.work.as_ref().expect("prepared");
        let offsets_buffer = self.gpu.upload("offsets", offsets);
        let mut first = 0;
        while first < offsets.len() {
            let count = work.chunk.min(offsets.len() - first);
            for (n, &j) in neighbours.iter().enumerate() {
                let other = &level[j];
                let params = self.gpu.uniform(
                    "warp params",
                    &WarpParams {
                        reference: camera_rows(&reference.camera),
                        neighbour: camera_rows(&other.camera),
                        hull: self.hull_params.0,
                        grid: [(2.0 / (other.width as f64 - 1.0)) as f32, (2.0 / (other.height as f64 - 1.0)) as f32, 0.0, 0.0],
                        shape: [self.hull_params.1[0], self.hull_params.1[1], self.hull_params.1[2], refine as u32],
                        size: [reference.width as u32, reference.height as u32, other.width as u32, other.height as u32],
                        chunk: [count as u32, first as u32, 0, 0],
                    },
                );
                self.gpu
                    .run(
                        &self.warp,
                        &[
                            &params,
                            &buffers.mask[view],
                            &self.hull,
                            init,
                            &buffers.gray[j],
                            &buffers.mask[j],
                            &offsets_buffer,
                            &work.warped,
                        ],
                        (count * pixels) as u64,
                    )
                    .await?;
                let size = [reference.width as u32, reference.height as u32, count as u32, window as u32];
                let params = self.gpu.uniform("ncc row params", &RowParams { size });
                self.gpu.run(&self.ncc_rows, &[&params, &buffers.gray[view], &work.warped, &work.sums], (count * pixels) as u64).await?;
                let params = self.gpu.uniform(
                    "ncc params",
                    &NccParams {
                        size,
                        flags: [(n == 0) as u32, (n + 1 == neighbours.len()) as u32, self.config.best_of as u32, first as u32],
                        gates: [self.config.min_variance as f32, self.config.window_fill as f32, 0.0, 0.0],
                    },
                );
                self.gpu
                    .run(&self.ncc_columns, &[&params, &work.warped, &work.sums, &work.best, &work.volume], (count * pixels) as u64)
                    .await?;
            }
            first += count;
        }
        Ok(())
    }

    /// `_aggregate` and `_peak`, then the depth of the best hypothesis per pixel.
    #[allow(clippy::too_many_arguments)]
    async fn reduce(
        &self,
        reference: &LevelView,
        hypotheses: usize,
        aggregate: f64,
        refine: bool,
        init: &wgpu::Buffer,
        values: [f32; 4],
    ) -> anyhow::Result<Pending> {
        let work = self.work.as_ref().ok_or_else(|| anyhow!("no cost volume"))?;
        let (width, height) = (reference.width, reference.height);
        let pixels = width * height;
        if aggregate != 0.0 {
            let weights = gauss_weights(aggregate);
            let params = self.gpu.uniform(
                "aggregate params",
                &AggregateParams { size: [width as u32, height as u32, hypotheses as u32, (weights.len() / 2) as u32] },
            );
            let weights = self.gpu.upload("aggregate weights", &weights);
            let bindings = [&params, &weights, &work.volume, &work.numerator, &work.denominator];
            self.gpu.run(&self.blur_rows, &bindings, (hypotheses * pixels) as u64).await?;
            self.gpu.run(&self.blur_columns, &bindings, (hypotheses * pixels) as u64).await?;
        }
        let params = self.gpu.uniform(
            "peak params",
            &PeakParams {
                size: [width as u32, height as u32, hypotheses as u32, refine as u32],
                values,
                confidence: [self.config.peak_min_margin as f32, 0.0, 0.0, 0.0],
            },
        );
        self.gpu.run(&self.peak, &[&params, &work.volume, init, &work.depth], pixels as u64).await?;
        Ok(Pending { width, height, read: self.gpu.begin_read::<f32>(&work.depth, pixels) })
    }

    /// The depth map of a view whose work was queued by [`Matcher::begin_refine`] or [`Matcher::begin_sweep`].
    pub async fn finish(&self, pending: Pending) -> anyhow::Result<Plane<f32>> {
        let Pending { width, height, read } = pending;
        Ok(Plane { width, height, data: self.gpu.finish_read(read).await? })
    }

    /// `Stereo.sweep`: a full inverse-depth sweep between the view's hull bounds.
    /// Returns the depth map and the size of one inverse-depth step.
    #[allow(clippy::too_many_arguments)]
    pub async fn sweep(
        &mut self,
        buffers: &LevelBuffers,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        bounds: (f64, f64),
        window: i64,
        aggregate: f64,
        min_score: f64,
    ) -> anyhow::Result<(Plane<f32>, f64)> {
        let (pending, step) = self.begin_sweep(buffers, level, view, neighbours, bounds, window, aggregate, min_score).await?;
        Ok((self.finish(pending).await?, step))
    }

    /// [`Matcher::sweep`] with its work queued and its readback started, not awaited.
    #[allow(clippy::too_many_arguments)]
    pub async fn begin_sweep(
        &mut self,
        buffers: &LevelBuffers,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        bounds: (f64, f64),
        window: i64,
        aggregate: f64,
        min_score: f64,
    ) -> anyhow::Result<(Pending, f64)> {
        let (near, far) = bounds;
        let planes = self.config.planes as usize;
        let inverse = linspace(1.0 / near, 1.0 / far, planes);
        let reference = &level[view];
        let unused = self.gpu.zeroed("no init", 4);
        self.score(buffers, level, view, neighbours, window, false, &unused, &inverse).await?;
        let step = (1.0 / far - 1.0 / near) / (planes as f64 - 1.0);
        let depth =
            self.reduce(reference, planes, aggregate, false, &unused, [step as f32, 0.0, min_score as f32, (1.0 / near) as f32]).await?;
        Ok((depth, step.abs()))
    }

    /// `Stereo.refine`: searches `half` steps of inverse depth either side of a smooth initial surface.
    #[allow(clippy::too_many_arguments)]
    pub async fn refine(
        &mut self,
        buffers: &LevelBuffers,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        init: &Plane<f32>,
        step: f64,
        half: i64,
        window: i64,
        aggregate: f64,
        min_score: f64,
    ) -> anyhow::Result<Plane<f32>> {
        let pending = self.begin_refine(buffers, level, view, neighbours, init, step, half, window, aggregate, min_score).await?;
        self.finish(pending).await
    }

    /// [`Matcher::refine`] with its work queued and its readback started, not awaited.
    #[allow(clippy::too_many_arguments)]
    pub async fn begin_refine(
        &mut self,
        buffers: &LevelBuffers,
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        init: &Plane<f32>,
        step: f64,
        half: i64,
        window: i64,
        aggregate: f64,
        min_score: f64,
    ) -> anyhow::Result<Pending> {
        let reference = &level[view];
        let offsets: Vec<f32> = (-half..=half).map(|n| n as f32 * step as f32).collect();
        if let Some(coherent) = &self.coherent {
            let settings = super::planes::Settings {
                radius: (window / 2) as u32,
                stride: 1,
                best_of: self.config.best_of as u32,
                min_variance: self.config.min_variance as f32,
                window_fill: self.config.window_fill as f32,
                iterations: 0,
                inverse_step: step as f32,
                normal_step: 0.0,
                inverse_range: (1e-6, 1e6),
                seed: 0,
            };
            let read = coherent
                .begin_band(
                    self.gpu,
                    level,
                    view,
                    neighbours,
                    &self.search,
                    init,
                    &settings,
                    half as u32,
                    min_score as f32,
                    self.config.peak_min_margin as f32,
                )
                .await?;
            return Ok(Pending { width: reference.width, height: reference.height, read });
        }
        let init_buffer = self.gpu.upload("init", &init.data);
        self.score(buffers, level, view, neighbours, window, true, &init_buffer, &offsets).await?;
        self.reduce(reference, offsets.len(), aggregate, true, &init_buffer, [step as f32, half as f32, min_score as f32, 0.0]).await
    }
}

/// Scalar implementation of the same matching, hypothesis by hypothesis: the
/// CPU counterpart of the kernels, slow, for tests and for checking kernels.
pub mod reference {
    use super::*;

    const INVALID: f32 = -1e30;

    /// The warp kernel for one hypothesis slice. `inverse(pixel)` is the hypothesis' inverse depth.
    fn warp(reference: &LevelView, other: &LevelView, search: &Hull, inverse: impl Fn(usize) -> f32) -> Vec<f32> {
        let (w, h) = (reference.width, reference.height);
        let grid = [(2.0 / (other.width as f64 - 1.0)) as f32, (2.0 / (other.height as f64 - 1.0)) as f32];
        let texel = |plane: &dyn Fn(usize) -> f32, x: i64, y: i64| {
            if x < 0 || y < 0 || x >= other.width as i64 || y >= other.height as i64 {
                0.0
            } else {
                plane(y as usize * other.width + x as usize)
            }
        };
        let mut out = vec![INVALID; w * h];
        for p in 0..w * h {
            if reference.mask.data[p] == 0 {
                continue;
            }
            let depth = 1.0 / inverse(p).max(1e-6);
            let world = reference.unproject(p % w, p / w, depth);
            if !search.contains(world) {
                continue;
            }
            let (xy, z) = other.project(world);
            if z <= 0.0 {
                continue;
            }
            let ix = (xy[0] * grid[0] - 1.0 + 1.0) / 2.0 * (other.width - 1) as f32;
            let iy = (xy[1] * grid[1] - 1.0 + 1.0) / 2.0 * (other.height - 1) as f32;
            if !(ix > -2.0 && iy > -2.0 && ix < other.width as f32 + 1.0 && iy < other.height as f32 + 1.0) {
                continue;
            }
            let (x0, y0) = (ix.floor(), iy.floor());
            let weights =
                [(x0 + 1.0 - ix) * (y0 + 1.0 - iy), (ix - x0) * (y0 + 1.0 - iy), (x0 + 1.0 - ix) * (iy - y0), (ix - x0) * (iy - y0)];
            let corners = [(0, 0), (1, 0), (0, 1), (1, 1)];
            let sample = |plane: &dyn Fn(usize) -> f32| -> f32 {
                corners.iter().zip(weights).map(|(&(dx, dy), weight)| texel(plane, x0 as i64 + dx, y0 as i64 + dy) * weight).sum()
            };
            if sample(&|q| other.mask.data[q] as f32) >= 0.999 {
                out[p] = sample(&|q| other.gray.data[q]);
            }
        }
        out
    }

    /// The NCC kernel for one hypothesis slice and one neighbour: -2 where unsupported.
    fn ncc(reference: &LevelView, warped: &[f32], window: i64, config: &DenseConfig) -> Vec<f32> {
        let (w, h) = (reference.width as i64, reference.height as i64);
        let half = window / 2;
        let scale = 1.0 / (window * window) as f32;
        let mut out = vec![-2.0f32; warped.len()];
        for p in 0..warped.len() {
            if warped[p] <= -1e29 {
                continue;
            }
            let (x, y) = (p as i64 % w, p as i64 / w);
            // Row by row, like the two kernels.
            let mut total = [0.0f32; 6];
            for qy in (y - half).max(0)..=(y + half).min(h - 1) {
                let mut row = [0.0f32; 6];
                for qx in (x - half).max(0)..=(x + half).min(w - 1) {
                    let q = (qy * w + qx) as usize;
                    if warped[q] > -1e29 {
                        let (a, b) = (reference.gray.data[q] - 0.5, warped[q] - 0.5);
                        for (sum, term) in row.iter_mut().zip([1.0, a, b, a * a, b * b, a * b]) {
                            *sum += term;
                        }
                    }
                }
                for (sum, term) in total.iter_mut().zip(row) {
                    *sum += term;
                }
            }
            let [count, sr, st, srr, stt, srt] = total;
            let fill = count * scale;
            let d = fill.max(1e-6);
            let (mr, mt) = (sr * scale / d, st * scale / d);
            let (vr, vt) = ((srr * scale / d - mr * mr).max(0.0), (stt * scale / d - mt * mt).max(0.0));
            if fill >= config.window_fill as f32 && vr >= config.min_variance as f32 && vt >= config.min_variance as f32 {
                out[p] = ((srt * scale / d - mr * mt) / (vr * vt + 1e-12).sqrt()).clamp(-1.0, 1.0);
            }
        }
        out
    }

    /// `Stereo.score` for one hypothesis slice: the mean of the best `best_of` neighbour scores.
    fn score(
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        search: &Hull,
        window: i64,
        config: &DenseConfig,
        inverse: &dyn Fn(usize) -> f32,
    ) -> Vec<f32> {
        let reference = &level[view];
        let pixels = reference.width * reference.height;
        let mut best = vec![vec![-2.0f32; pixels]; config.best_of as usize];
        for &j in neighbours {
            let mut scores = ncc(reference, &warp(reference, &level[j], search, inverse), window, config);
            for rank in best.iter_mut() {
                for p in 0..pixels {
                    let lower = rank[p].min(scores[p]);
                    rank[p] = rank[p].max(scores[p]);
                    scores[p] = lower;
                }
            }
        }
        (0..pixels)
            .map(|p| {
                let valid: Vec<f32> = best.iter().map(|rank| rank[p]).filter(|&s| s > -1.5).collect();
                if valid.len() >= 2 {
                    valid.iter().sum::<f32>() / valid.len() as f32
                } else {
                    -2.0
                }
            })
            .collect()
    }

    /// `_aggregate` on one slice.
    fn aggregate(slice: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
        if sigma == 0.0 {
            return slice.to_vec();
        }
        let plane = |data: Vec<f32>| Plane { width, height, data };
        let num = super::super::depth::gauss(&plane(slice.iter().map(|&v| if v > -1.5 { v } else { 0.0 }).collect()), sigma);
        let den = super::super::depth::gauss(&plane(slice.iter().map(|&v| (v > -1.5) as u8 as f32).collect()), sigma);
        (0..slice.len()).map(|p| if slice[p] > -1.5 && den.data[p] > 0.3 { num.data[p] / den.data[p].max(1e-6) } else { -2.0 }).collect()
    }

    /// `_peak` and the depth formula shared by `sweep` and `refine`.
    pub(super) fn peak(volume: &[Vec<f32>], p: usize, base: f32, shift: f32, step: f32, min_score: f32, min_margin: f32) -> f32 {
        let mut index = 0;
        for h in 1..volume.len() {
            if volume[h][p] > volume[index][p] {
                index = h;
            }
        }
        if index == 0 || index + 1 >= volume.len() {
            return 0.0;
        }
        let (best, left, right) = (volume[index][p], volume[index - 1][p], volume[index + 1][p]);
        if !(left > -1.5 && right > -1.5 && best >= min_score) {
            return 0.0;
        }
        if min_margin > 0.0 {
            let mut competitor = -2.0f32;
            for h in 0..volume.len() {
                let score = volume[h][p];
                if h.abs_diff(index) > 2
                    && score > -1.5
                    && score >= volume[h.saturating_sub(1)][p]
                    && score >= volume[(h + 1).min(volume.len() - 1)][p]
                {
                    let valley = volume[h.min(index)..=h.max(index)].iter().map(|slice| slice[p]).fold(score, f32::min);
                    if valley < score.min(best) - 0.02 {
                        competitor = competitor.max(score);
                    }
                }
            }
            if best - competitor < min_margin {
                return 0.0;
            }
        }
        let curve = left - 2.0 * best + right;
        let offset = if curve < -1e-6 { (0.5 * (left - right) / curve).clamp(-0.5, 0.5) } else { 0.0 };
        1.0 / (base + (index as f32 - shift + offset) * step).max(1e-6)
    }

    /// `Stereo.sweep` on the CPU.
    #[allow(clippy::too_many_arguments)]
    pub fn sweep(
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        bounds: (f64, f64),
        search: &Hull,
        window: i64,
        sigma: f64,
        min_score: f64,
        config: &DenseConfig,
    ) -> Plane<f32> {
        let (near, far) = bounds;
        let planes = config.planes as usize;
        let reference = &level[view];
        let (width, height) = (reference.width, reference.height);
        let volume: Vec<Vec<f32>> = linspace(1.0 / near, 1.0 / far, planes)
            .into_iter()
            .map(|inverse| aggregate(&score(level, view, neighbours, search, window, config, &|_| inverse), width, height, sigma))
            .collect();
        let step = ((1.0 / far - 1.0 / near) / (planes as f64 - 1.0)) as f32;
        let data = (0..width * height)
            .map(|p| peak(&volume, p, (1.0 / near) as f32, 0.0, step, min_score as f32, config.peak_min_margin as f32))
            .collect();
        Plane { width, height, data }
    }

    /// `Stereo.refine` on the CPU.
    #[allow(clippy::too_many_arguments)]
    pub fn refine(
        level: &[LevelView],
        view: usize,
        neighbours: &[usize],
        init: &Plane<f32>,
        step: f64,
        half: i64,
        search: &Hull,
        window: i64,
        sigma: f64,
        min_score: f64,
        config: &DenseConfig,
    ) -> Plane<f32> {
        let reference = &level[view];
        let (width, height) = (reference.width, reference.height);
        let base: Vec<f32> = init.data.iter().map(|&d| 1.0 / d.max(1e-6)).collect();
        if config.coherent_band {
            let settings = super::super::planes::Settings {
                radius: (window / 2) as u32,
                stride: 1,
                best_of: config.best_of as u32,
                min_variance: config.min_variance as f32,
                window_fill: config.window_fill as f32,
                iterations: 0,
                inverse_step: step as f32,
                normal_step: 0.0,
                inverse_range: (1e-6, 1e6),
                seed: 0,
            };
            let planes = super::super::planes::initial_planes(reference, init, init);
            let scorer = super::super::planes::reference::Scorer::new(level, view, neighbours, search, &settings);
            let volume = (-half..=half)
                .map(|n| {
                    (0..width * height)
                        .map(|p| {
                            let inverse = base[p] + n as f32 * step as f32;
                            if init.data[p] <= 0.0 || inverse <= 0.0 {
                                return -2.0;
                            }
                            let mut plane = planes[p];
                            plane[0] = 1.0 / inverse;
                            scorer.score((p % width) as i32, (p / width) as i32, plane)
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let data = (0..width * height)
                .map(|p| {
                    if init.data[p] > 0.0 && reference.mask.data[p] != 0 {
                        peak(&volume, p, base[p], half as f32, step as f32, min_score as f32, config.peak_min_margin as f32)
                    } else {
                        0.0
                    }
                })
                .collect();
            return Plane { width, height, data };
        }
        let volume: Vec<Vec<f32>> = (-half..=half)
            .map(|n| {
                let offset = n as f32 * step as f32;
                aggregate(&score(level, view, neighbours, search, window, config, &|p| base[p] + offset), width, height, sigma)
            })
            .collect();
        let data = (0..width * height)
            .map(|p| {
                if init.data[p] > 0.0 {
                    peak(&volume, p, base[p], half as f32, step as f32, min_score as f32, config.peak_min_margin as f32)
                } else {
                    0.0
                }
            })
            .collect();
        Plane { width, height, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::build_hull;
    use crate::inputs::Inputs;
    use crate::stereo::depth::initial;
    use crate::stereo::level::build_level;
    use crate::stereo::{options, synthetic};

    const PEAK_CURVES: [[f32; 9]; 8] = [
        [0.1, 0.2, 0.3, 0.7, 0.9, 0.7, 0.3, 0.2, 0.1],
        [0.1, 0.7, 0.9, 0.7, 0.1, 0.7, 0.89, 0.7, 0.1],
        [0.895, 0.896, 0.897, 0.898, 0.9, 0.898, 0.897, 0.896, 0.895],
        [0.99, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1],
        [0.1, 0.2, 0.3, -2.0, 0.9, 0.7, 0.3, 0.2, 0.1],
        [0.1, 0.7, 0.9, 0.7, 0.1, 0.7, 0.8, 0.7, 0.1],
        [0.89, 0.7, 0.1, 0.7, 0.9, 0.7, 0.3, 0.2, 0.1],
        [-2.0, -2.0, -2.0, 0.7, 0.9, 0.7, -2.0, -2.0, -2.0],
    ];

    fn peak_volume() -> Vec<Vec<f32>> {
        (0..9).map(|h| PEAK_CURVES.iter().map(|curve| curve[h]).collect()).collect()
    }

    #[test]
    fn confidence_rejects_distinct_modes_but_keeps_broad_peaks() {
        let volume = peak_volume();
        for (p, keep) in [true, false, true, false, false, true, false, true].into_iter().enumerate() {
            let legacy = reference::peak(&volume, p, 0.5, 0.0, 0.01, 0.55, 0.0);
            let filtered = reference::peak(&volume, p, 0.5, 0.0, 0.01, 0.55, 0.02);
            if keep {
                assert!(legacy > 0.0);
                assert_eq!(filtered, legacy, "curve {p}");
            } else {
                assert_eq!(filtered, 0.0, "curve {p}");
            }
        }
    }

    #[test]
    fn gpu_peak_confidence_matches_scalar_curves() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let gpu = Gpu::new().unwrap();
        let kernel = crate::gpu::block_on(gpu.kernel("peak confidence", include_str!("../shaders/peak.wgsl"), "main")).unwrap();
        let mut volume = peak_volume();
        // Deterministic noisy curves exercise ties, invalid support and arbitrary modes.
        let mut seed = 71u32;
        for _ in 0..256 {
            for row in &mut volume {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                row.push(if seed.is_multiple_of(13) { -2.0 } else { (seed >> 8) as f32 / 16777216.0 });
            }
        }
        let pixels = volume[0].len();
        let cost = gpu.upload("costs", &volume.concat());
        let init = gpu.upload("initial", &vec![2.0f32; pixels]);
        for margin in [0.0, 0.02] {
            for refine in [0, 1] {
                let params = gpu.uniform(
                    "peak params",
                    &PeakParams {
                        size: [pixels as u32, 1, 9, refine],
                        values: [0.01, 0.0, 0.55, 0.5],
                        confidence: [margin, 0.0, 0.0, 0.0],
                    },
                );
                let output = gpu.zeroed("depths", (pixels * 4) as u64);
                crate::gpu::block_on(gpu.run(&kernel, &[&params, &cost, &init, &output], pixels as u64)).unwrap();
                let found = crate::gpu::block_on(gpu.read::<f32>(&output, pixels)).unwrap();
                for (p, actual) in found.into_iter().enumerate() {
                    let expected = reference::peak(&volume, p, 0.5, 0.0, 0.01, 0.55, margin);
                    assert!((actual - expected).abs() < 1e-5, "curve {p}, margin {margin}, refine {refine}: {actual} != {expected}");
                }
            }
        }
    }

    /// Fraction of pixels where two depth maps agree in validity and within `tolerance` (relative).
    fn agreement(a: &Plane<f32>, b: &Plane<f32>, tolerance: f32) -> (f64, usize) {
        let (mut same, mut valid) = (0usize, 0usize);
        for (&x, &y) in a.data.iter().zip(&b.data) {
            valid += (x > 0.0) as usize;
            same += ((x > 0.0) == (y > 0.0) && (x - y).abs() <= tolerance * x.max(y)) as usize;
        }
        (same as f64 / a.data.len() as f64, valid)
    }

    /// GPU sweep and refinement against the scalar implementation (`CRISPMESH_GPU_TESTS=1`).
    #[test]
    fn gpu_matching_matches_the_scalar_implementation() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = synthetic::temporary("matcher", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let gpu = Gpu::new().unwrap();
        let state = crate::gpu::block_on(build_hull(&gpu, &inputs, &config, &mut None)).unwrap();
        let mut matcher = crate::gpu::block_on(Matcher::new(&gpu, &config, &state.search)).unwrap();
        let view = 5;
        let neighbours = inputs.neighbours(view, config.neighbours as usize, &config).unwrap();

        let coarse = build_level(&inputs, 64);
        let buffers = matcher.upload(&coarse);
        let (found, step) =
            crate::gpu::block_on(matcher.sweep(&buffers, &coarse, view, &neighbours, state.bounds[view], 5, 1.0, config.min_score))
                .unwrap();
        let expected = reference::sweep(&coarse, view, &neighbours, state.bounds[view], &state.search, 5, 1.0, config.min_score, &config);
        let (same, valid) = agreement(&expected, &found, 1e-4);
        assert!(valid > 500, "{valid} valid pixels");
        assert!(same > 0.995, "sweep: {same}");

        let fine = build_level(&inputs, 128);
        let buffers = matcher.upload(&fine);
        let init = initial(&found, &fine[view].mask, 1.5);
        let found =
            crate::gpu::block_on(matcher.refine(&buffers, &fine, view, &neighbours, &init, step / 2.0, 8, 7, 1.0, config.min_score))
                .unwrap();
        let expected = reference::refine(&fine, view, &neighbours, &init, step / 2.0, 8, &state.search, 7, 1.0, config.min_score, &config);
        let (same, valid) = agreement(&expected, &found, 1e-4);
        assert!(valid > 1000, "{valid} valid pixels");
        assert!(same > 0.995, "refine: {same}");
        assert!(gpu.peak_binding() <= 128 << 20);
    }
}
