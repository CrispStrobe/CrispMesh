//! Training of 2D Gaussian splats on a scene, as our trainer after gsplat's `examples/simple_trainer_2dgs.py`
//! (the reference run: drill and mustard on a Tesla T4): splats start on our silhouette hull, one photo per step on
//! a random background, Adam per parameter group, gsplat's default densification (duplicate, split, prune).

use std::path::Path;

use anyhow::{ensure, Result};
use bytemuck::{Pod, Zeroable};

use super::*;
use crate::surface::train::{Random, Scene};

/// Settings; the defaults are the reference run's (30 000 steps).
#[derive(Clone, Debug)]
pub struct Settings {
    pub steps: u32,
    /// End early on the schedule of `steps` (0: no).
    pub stop: u32,
    /// Every this many views is held out for evaluation.
    pub hold_every: usize,
    pub log_every: u32,
    pub seed: u64,
    pub sh_degree: u32,
    pub weights: Weights,
    /// Fusion grid of the mesh.
    pub mesh: usize,
    /// Start from these splats (`splats.npz` of a run) instead of the hull.
    pub init: Option<std::path::PathBuf>,
}

impl Settings {
    /// The settings of a quality level (standard: the reference run's 30 000 steps; the densification schedule
    /// scales with the steps).
    pub fn for_quality(quality: crate::surface::options::Quality) -> Self {
        use crate::surface::options::Quality;
        let standard = Settings::default();
        match quality {
            Quality::Fast => Settings { steps: 10000, mesh: 384, ..standard },
            Quality::Standard => standard,
            Quality::Best => Settings { mesh: 768, ..standard },
        }
    }

    /// Applies one `NAME=VALUE`; false if the name is not a setting of the splats.
    pub fn set(&mut self, name: &str, value: &str) -> Result<bool> {
        use crate::surface::options::{number, whole};
        let int = || whole(name, value);
        match name {
            "steps" => self.steps = int()?,
            "stop" => self.stop = int()?,
            "log" => self.log_every = int()?,
            "seed" => self.seed = int()? as u64,
            "mesh" => self.mesh = int()? as usize,
            "sh" => self.sh_degree = int()?.min(3),
            "hold" => self.hold_every = int()? as usize,
            "init" => self.init = Some(std::path::PathBuf::from(value)),
            "l1" => self.weights.l1 = number(name, value)?,
            "ssim" => self.weights.ssim = number(name, value)?,
            "mask" => self.weights.mask = number(name, value)?,
            "normal" => self.weights.normal = number(name, value)?,
            "distortion" => self.weights.distortion = number(name, value)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            steps: 30000,
            stop: 0,
            hold_every: 10,
            log_every: 500,
            seed: 42,
            sh_degree: 3,
            weights: Weights::default(),
            mesh: 512,
            init: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AdamParams {
    count: u32,
    pad: u32,
    correction1: f32,
    correction2: f32,
    rates: [f32; 8],
}

/// gsplat's DefaultStrategy settings (with the reference run's schedule scaled to `steps / 30000`).
struct Strategy {
    grow_grad2d: f32,
    grow_scale3d: f32,
    prune_opacity: f32,
    prune_scale3d: f32,
    refine_start: u32,
    refine_stop: u32,
    refine_every: u32,
    reset_every: u32,
}

impl Strategy {
    fn new(steps: u32) -> Self {
        let scale = steps as f64 / 30000.0;
        Strategy {
            grow_grad2d: 0.0002,
            grow_scale3d: 0.01,
            prune_opacity: 0.005,
            prune_scale3d: 0.1,
            refine_start: (500.0 * scale) as u32 + 1,
            refine_stop: (15000.0 * scale) as u32,
            refine_every: ((100.0 * scale) as u32).max(20),
            reset_every: ((3000.0 * scale) as u32).max(100),
        }
    }
}

/// A view of the scene as a splat camera, in the scene's unit frame.
pub fn camera(view: &crate::surface::train::View) -> Camera {
    let r = view.rotation;
    let o = view.origin;
    let rotation: [[f32; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| r[i][j] as f32));
    let translation = std::array::from_fn(|i| -(0..3).map(|j| r[i][j] * o[j]).sum::<f64>() as f32);
    let k = view.intrinsics;
    Camera {
        rotation,
        translation,
        intrinsics: [k[0] as f32, k[1] as f32, k[2] as f32, k[3] as f32],
        width: view.width as u32,
        height: view.height as u32,
    }
}

/// Start splats: every second hull point, scale the root mean squared distance to the three nearest others,
/// random quaternions in [0, 1)^4, opacity 0.1, grey (all SH coefficients zero).
pub fn initial(scene: &Scene, random: &mut Random) -> Vec<f32> {
    let points: Vec<[f64; 3]> = scene.hull_points.iter().step_by(2).copied().collect();
    let tree = crate::surface::train::Tree::new(&points);
    let mut rows = vec![0f32; points.len() * PARAMS as usize];
    let opacity = (0.1f64 / 0.9).ln() as f32;
    for (g, p) in points.iter().enumerate() {
        let nearest = tree.nearest_k(*p, 4);
        let mean = nearest[1..].iter().sum::<f64>() / (nearest.len() - 1).max(1) as f64;
        let scale = (mean.sqrt().max(1e-7)).ln() as f32;
        let row = &mut rows[g * PARAMS as usize..(g + 1) * PARAMS as usize];
        for i in 0..3 {
            row[MEAN + i] = p[i] as f32;
            row[SCALE + i] = scale;
        }
        for i in 0..4 {
            row[QUAT + i] = random.uniform() as f32;
        }
        row[OPACITY] = opacity;
    }
    rows
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn rotation(q: [f32; 4]) -> [[f32; 3]; 3] {
    let n = (q.iter().map(|v| v * v).sum::<f32>()).sqrt();
    let [w, x, y, z] = q.map(|v| v / n);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/// gsplat's grow (duplicate, then split) and prune on the rows and their Adam moments; returns the new rows and
/// moments and the counts (duplicated, split, pruned).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn refine(
    strategy: &Strategy,
    step: u32,
    extent: f32,
    rows: Vec<f32>,
    first: Vec<f32>,
    second: Vec<f32>,
    stats: &[f32],
    random: &mut Random,
) -> (Vec<f32>, Vec<f32>, Vec<f32>, [usize; 3]) {
    let w = PARAMS as usize;
    let n = rows.len() / w;
    let max_scale = |row: &[f32]| row[SCALE..SCALE + 3].iter().fold(f32::MIN, |m, v| m.max(*v)).exp();
    let high: Vec<bool> = (0..n).map(|g| stats[g * 2] / stats[g * 2 + 1].max(1.0) > strategy.grow_grad2d).collect();
    let small: Vec<bool> = (0..n).map(|g| max_scale(&rows[g * w..(g + 1) * w]) <= strategy.grow_scale3d * extent).collect();
    let (mut rows, mut first, mut second) = (rows, first, second);
    // Duplicate: copies appended, their moments zero.
    let dupli: Vec<usize> = (0..n).filter(|g| high[*g] && small[*g]).collect();
    for g in &dupli {
        rows.extend_from_within(g * w..(g + 1) * w);
    }
    first.resize(rows.len(), 0.0);
    second.resize(rows.len(), 0.0);
    // Split: the selected removed, two samples of each appended (all first samples, then all second ones), scales
    // divided by 1.6, moments zero.
    let total = rows.len() / w;
    let split: Vec<bool> = (0..total).map(|g| g < n && high[g] && !small[g]).collect();
    let chosen: Vec<usize> = (0..total).filter(|g| split[*g]).collect();
    let mut kept_rows = Vec::with_capacity(rows.len() + chosen.len() * w);
    let mut kept_first = Vec::with_capacity(kept_rows.capacity());
    let mut kept_second = Vec::with_capacity(kept_rows.capacity());
    for g in (0..total).filter(|g| !split[*g]) {
        kept_rows.extend_from_slice(&rows[g * w..(g + 1) * w]);
        kept_first.extend_from_slice(&first[g * w..(g + 1) * w]);
        kept_second.extend_from_slice(&second[g * w..(g + 1) * w]);
    }
    let samples: Vec<[[f32; 3]; 2]> =
        chosen.iter().map(|_| std::array::from_fn(|_| std::array::from_fn(|_| random.normal() as f32))).collect();
    #[allow(clippy::needless_range_loop)]
    for copy in 0..2 {
        for (i, g) in chosen.iter().enumerate() {
            let mut row = rows[g * w..(g + 1) * w].to_vec();
            let s: [f32; 3] = std::array::from_fn(|k| row[SCALE + k].exp());
            let r = rotation([row[QUAT], row[QUAT + 1], row[QUAT + 2], row[QUAT + 3]]);
            for a in 0..3 {
                row[MEAN + a] += (0..3).map(|b| r[a][b] * s[b] * samples[i][copy][b]).sum::<f32>();
                row[SCALE + a] = (s[a] / 1.6).ln();
            }
            kept_rows.extend_from_slice(&row);
        }
    }
    kept_first.resize(kept_rows.len(), 0.0);
    kept_second.resize(kept_rows.len(), 0.0);
    // Prune: transparent splats, and from the first reset step on, too large ones.
    let total = kept_rows.len() / w;
    let prune: Vec<bool> = (0..total)
        .map(|g| {
            let row = &kept_rows[g * w..(g + 1) * w];
            sigmoid(row[OPACITY]) < strategy.prune_opacity
                || (step > strategy.reset_every && max_scale(row) > strategy.prune_scale3d * extent)
        })
        .collect();
    let pruned = prune.iter().filter(|p| **p).count();
    let keep = |v: &[f32]| -> Vec<f32> { (0..total).filter(|g| !prune[*g]).flat_map(|g| v[g * w..(g + 1) * w].iter().copied()).collect() };
    (keep(&kept_rows), keep(&kept_first), keep(&kept_second), [dupli.len(), chosen.len(), pruned])
}

/// Mean colour PSNR inside the mask of each held-out view.
async fn evaluate(
    gpu: &Gpu,
    kernels: &Kernels,
    splats: &Splats,
    scene: &Scene,
    views: &[usize],
    degree: u32,
    out: Option<&Path>,
) -> Result<Vec<f64>> {
    let mut psnr = Vec::new();
    for (n, &v) in views.iter().enumerate() {
        let view = &scene.views[v];
        let cam = camera(view);
        let mut frame = Frame::new(gpu, splats.count, &cam);
        render(gpu, kernels, splats, &cam, degree, &mut frame).await?;
        let pixels: Vec<f32> = gpu.read(&frame.pixels, view.width * view.height * PIXEL as usize).await?;
        let (mut error, mut inside) = (0f64, 0usize);
        for (i, m) in view.mask.iter().enumerate() {
            if *m {
                for c in 0..3 {
                    let d = pixels[i * PIXEL as usize + c].clamp(0.0, 1.0) as f64 - view.image[i * 3 + c] as f64 / 255.0;
                    error += d * d;
                }
                inside += 3;
            }
        }
        psnr.push(-10.0 * (error / inside.max(1) as f64).log10());
        if let (Some(dir), true) = (out, n < 3) {
            let (w, h) = (view.width, view.height);
            let mut sheet = image::RgbImage::new(2 * w as u32, h as u32);
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let photo = [view.image[i * 3], view.image[i * 3 + 1], view.image[i * 3 + 2]];
                    let ours: [u8; 3] = std::array::from_fn(|c| (pixels[i * PIXEL as usize + c].clamp(0.0, 1.0) * 255.0).round() as u8);
                    sheet.put_pixel(x as u32, y as u32, image::Rgb(photo));
                    sheet.put_pixel((w + x) as u32, y as u32, image::Rgb(ours));
                }
            }
            sheet.save(dir.join(format!("heldout-{v:03}.png")))?;
        }
    }
    Ok(psnr)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FuseParams {
    count: u32,
    resolution: u32,
    width: u32,
    height: u32,
    rotation: [[f32; 4]; 3],
    intrinsics: [f32; 4],
    truncation: f32,
    pad: [f32; 3],
}

/// The mesh of the splats: every view's median depth fused (truncation 4 voxels) at `resolution` cubed over the
/// unit sphere's cube, within `band` of our hull (outside it the hull's side decides), then marching cubes; written
/// as STL in world coordinates. Returns the triangles.
#[allow(clippy::too_many_arguments)]
pub async fn mesh(
    gpu: &Gpu,
    kernels: &Kernels,
    splats: &Splats,
    scene: &Scene,
    degree: u32,
    resolution: usize,
    band: f32,
    path: &Path,
) -> Result<usize> {
    use rayon::prelude::*;
    let hull_resolution = 160;
    let hull = crate::surface::train::hull_grid(scene, hull_resolution);
    let size = 2.0 / resolution as f32;
    let centre = |i: usize| (i as f32 + 0.5) * size - 1.0;
    let r = resolution;
    let hull_value: Vec<f32> = (0..r * r * r)
        .into_par_iter()
        .map(|n| crate::surface::train::trilinear(&hull, hull_resolution, [centre(n / (r * r)), centre(n / r % r), centre(n % r)]))
        .collect();
    let voxels: Vec<u32> = (0..r * r * r).filter(|n| hull_value[*n].abs() < band).map(|n| n as u32).collect();
    let sums = gpu.zeroed("splat fuse sums", voxels.len() as u64 * 3 * 4);
    let list = gpu.upload("splat fuse voxels", &voxels);
    let fuse = gpu.kernel("splat_fuse", include_str!("../shaders/splat_fuse.wgsl"), "main").await?;
    for view in &scene.views {
        let cam = camera(view);
        let mut frame = Frame::new(gpu, splats.count, &cam);
        render(gpu, kernels, splats, &cam, degree, &mut frame).await?;
        let params = FuseParams {
            count: voxels.len() as u32,
            resolution: r as u32,
            width: cam.width,
            height: cam.height,
            rotation: std::array::from_fn(|i| [cam.rotation[i][0], cam.rotation[i][1], cam.rotation[i][2], cam.translation[i]]),
            intrinsics: cam.intrinsics,
            truncation: 4.0 * size,
            pad: [0.0; 3],
        };
        let uniform = gpu.uniform("splat fuse", &params);
        gpu.run(&fuse, &[&uniform, &list, &frame.pixels, &sums], voxels.len() as u64).await?;
    }
    let sums: Vec<f32> = gpu.read(&sums, voxels.len() * 3).await?;
    let mut field: Vec<f32> = hull_value.iter().map(|h| if *h < 0.0 { -1.0 } else { 1.0 }).collect();
    drop(hull_value);
    for (i, n) in voxels.iter().enumerate() {
        let (total, weight, behind) = (sums[i * 3], sums[i * 3 + 1], sums[i * 3 + 2]);
        field[*n as usize] = if weight >= 3.0 {
            total / weight
        } else if behind > weight {
            -1.0
        } else {
            1.0
        };
    }
    let mesh = crate::mesh::cubes::extract(&field, [r; 3], 1);
    if mesh.faces.is_empty() {
        return Ok(0);
    }
    let vertices: Vec<[f64; 3]> = mesh
        .vertices
        .iter()
        .map(|v| std::array::from_fn(|i| ((v[i] as f64 + 0.5) * size as f64 - 1.0) / scene.unit + scene.centre[i]))
        .collect();
    crate::stl::write_binary(path, "crispmesh 2d splats (webgpu)", &vertices, &mesh.faces)?;
    Ok(mesh.faces.len())
}

/// Trains splats on a scene and writes `splats.npz`, held-out renders and `summary.json` into `out`. Progress to stdout.
pub async fn train(gpu: &Gpu, scene: &Scene, out: &Path, settings: &Settings) -> Result<Splats> {
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
) -> Result<Splats> {
    let start = web_time::Instant::now();
    let log = |start: &web_time::Instant, text: &str| say(&format!("[{:6.0}s] {text}", start.elapsed().as_secs_f64()));
    std::fs::create_dir_all(out)?;
    let kernels = Kernels::new(gpu).await?;
    let adam = gpu.kernel("splat_adam", include_str!("../shaders/splat_adam.wgsl"), "main").await?;
    let densify = gpu.kernel("splat_densify_stats", include_str!("../shaders/splat_densify_stats.wgsl"), "main").await?;
    gpu.set_batching(true);
    let mut random = Random::new(settings.seed);
    let held: Vec<usize> = (0..scene.views.len()).step_by(settings.hold_every.max(1)).collect();
    let training: Vec<usize> = (0..scene.views.len()).filter(|v| v % settings.hold_every.max(1) != 0).collect();
    ensure!(!training.is_empty(), "no training views");
    let extent = scene.views.iter().map(|v| (v.origin.iter().map(|o| o * o).sum::<f64>()).sqrt()).fold(0.0, f64::max) as f32;
    let strategy = Strategy::new(settings.steps);
    let rows = match &settings.init {
        Some(path) => crate::npz::Npz::read(path)?.get("params")?.to_f32(),
        None => initial(scene, &mut random),
    };
    let mut splats = Splats::new(gpu, &rows)?;
    let mut first = gpu.zeroed("splat adam m", rows.len() as u64 * 4);
    let mut second = gpu.zeroed("splat adam v", rows.len() as u64 * 4);
    let mut grads = gpu.zeroed("splat grads", rows.len() as u64 * 4);
    let mut stats = gpu.zeroed("splat densify stats", splats.count as u64 * 2 * 4);
    let mut dsplat = gpu.zeroed("splat dsplat", splats.count as u64 * DSPLAT as u64 * 4);
    drop(rows);
    log(&start, &format!("{} views ({} held out), {} splats, extent {extent:.3}", scene.views.len(), held.len(), splats.count));
    let (w, h) = (scene.views[0].width, scene.views[0].height);
    ensure!(scene.views.iter().all(|v| v.width == w && v.height == h), "the views differ in size");
    let loss = Loss::new(gpu, w as u32, h as u32);
    let mut frame = Frame::new(gpu, splats.count, &camera(&scene.views[training[0]]));
    let mut adam_steps = 0;
    let end = if settings.stop > 0 { settings.stop } else { settings.steps };
    let mean_rate = |step: u32| 1.6e-4 * extent * 0.01f32.powf(step as f32 / settings.steps as f32);
    let mut history = Vec::new();
    for step in 0..end {
        check()?;
        let v = training[random.below(training.len())];
        let view = &scene.views[v];
        let cam = camera(view);
        let degree = (step / ((1000.0 * settings.steps as f64 / 30000.0) as u32).max(1)).min(settings.sh_degree);
        let background = [random.uniform() as f32, random.uniform() as f32, random.uniform() as f32];
        gpu.write(&loss.photo, &pack_photo(&view.image, &view.mask));
        render(gpu, &kernels, &splats, &cam, degree, &mut frame).await?;
        let scale = settings.steps as f64 / 30000.0;
        let weights = Weights {
            normal: if step as f64 > 7000.0 * scale { settings.weights.normal } else { 0.0 },
            distortion: if step as f64 > 3000.0 * scale { settings.weights.distortion } else { 0.0 },
            ..settings.weights
        };
        loss_backward(gpu, &kernels, &cam, &frame, &loss, &weights, background).await?;
        gpu.clear(&dsplat);
        gpu.clear(&grads);
        raster_backward(gpu, &kernels, &cam, &frame, &loss.dpixels, &dsplat).await?;
        project_backward(gpu, &kernels, &splats, &cam, degree, &frame, &dsplat, &grads).await?;
        if step < strategy.refine_stop {
            let uniform = gpu.uniform("splat densify", &Quad { a: splats.count, b: cam.width, c: cam.height, d: 0 });
            gpu.run(&densify, &[&uniform, &frame.projected, &dsplat, &stats], splats.count as u64).await?;
        }
        let refining = step < strategy.refine_stop && step > strategy.refine_start && step % strategy.refine_every == 0;
        if step % settings.log_every == 0 || step + 1 == end {
            let terms: Vec<f32> = gpu.read(&loss.terms, w * h * 5).await?;
            let ssim: Vec<f32> = gpu.read(&loss.ssim, w * h).await?;
            let p = (w * h) as f64;
            let mean = |k: usize| terms.iter().skip(k).step_by(5).map(|v| *v as f64).sum::<f64>() / p;
            let (l1, s) = (mean(0) / 3.0, ssim.iter().map(|v| *v as f64).sum::<f64>() / (3.0 * p));
            let total = 0.8 * l1 + 0.2 * (1.0 - s) + 0.1 * mean(2) + weights.normal as f64 * mean(3) + weights.distortion as f64 * mean(4);
            history.push(serde_json::json!([step, total, l1, s, mean(2), mean(3), mean(4), splats.count]));
            if let Some(report) = gpu.profile_report() {
                log(&start, &format!("kernels since the last report: {report}"));
            }
            log(&start, &format!("step {step}: loss {total:.4} l1 {l1:.4} ssim {s:.4} mask {:.4} normal {:.4} distortion {:.5}, {} splats, {} intersections", mean(2), mean(3), mean(4), splats.count, frame.intersections));
        }
        if refining {
            // gsplat replaces the parameters here, so the optimiser does not step on this iteration.
            let n = splats.count as usize * PARAMS as usize;
            let rows: Vec<f32> = gpu.read(&splats.params, n).await?;
            let m: Vec<f32> = gpu.read(&first, n).await?;
            let s: Vec<f32> = gpu.read(&second, n).await?;
            let st: Vec<f32> = gpu.read(&stats, splats.count as usize * 2).await?;
            let (rows, m, s, counts) = refine(&strategy, step, extent, rows, m, s, &st, &mut random);
            splats = Splats::new(gpu, &rows)?;
            first = gpu.upload("splat adam m", &m);
            second = gpu.upload("splat adam v", &s);
            grads = gpu.zeroed("splat grads", rows.len() as u64 * 4);
            stats = gpu.zeroed("splat densify stats", splats.count as u64 * 2 * 4);
            dsplat = gpu.zeroed("splat dsplat", splats.count as u64 * DSPLAT as u64 * 4);
            frame = Frame::new(gpu, splats.count, &cam);
            if step % (settings.log_every.max(strategy.refine_every)) == 0 {
                log(
                    &start,
                    &format!("refine {step}: {} duplicated, {} split, {} pruned, {} splats", counts[0], counts[1], counts[2], splats.count),
                );
            }
        } else {
            adam_steps += 1;
            let rates = [mean_rate(step), 1e-3, 5e-3, 5e-2, 2.5e-3, 2.5e-3 / 20.0, 0.0, 0.0];
            let params = AdamParams {
                count: splats.count,
                pad: 0,
                correction1: 1.0 - 0.9f32.powi(adam_steps),
                correction2: 1.0 - 0.999f32.powi(adam_steps),
                rates,
            };
            let uniform = gpu.uniform("splat adam", &params);
            gpu.run(&adam, &[&uniform, &splats.params, &grads, &first, &second], splats.count as u64 * PARAMS as u64).await?;
        }
    }
    let train_seconds = start.elapsed().as_secs_f64();
    let psnr = evaluate(gpu, &kernels, &splats, scene, &held, settings.sh_degree, Some(out)).await?;
    let mut sorted = psnr.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(f64::NAN);
    let triangles = mesh(gpu, &kernels, &splats, scene, settings.sh_degree, settings.mesh, 0.05, &out.join("mesh.stl")).await?;
    log(&start, &format!("mesh: {triangles} triangles"));
    let rows: Vec<f32> = gpu.read(&splats.params, splats.count as usize * PARAMS as usize).await?;
    let array = crate::npz::Array::new(&[splats.count as usize, PARAMS as usize], crate::npz::Data::F32(rows));
    crate::npz::write(&out.join("splats.npz"), &[("params", &array)], false)?;
    let summary = serde_json::json!({"steps": settings.steps, "views": scene.views.len(), "held_out": held.len(), "splats": splats.count,
        "train_seconds": train_seconds, "seconds": start.elapsed().as_secs_f64(), "triangles": triangles, "heldout_psnr_in_mask_median": median, "heldout_psnr_in_mask": psnr, "adapter": gpu.describe(), "history": history});
    std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary)?)?;
    log(&start, &format!("done: {} splats, training {train_seconds:.0}s, held-out PSNR in the mask median {median:.2} dB", splats.count));
    Ok(splats)
}

/// `crisp3ds-dense splats SOURCE OUT [quality=fast|standard|best] [NAME=VALUE ...]`
#[cfg(not(target_arch = "wasm32"))]
pub fn main(arguments: &[String]) -> Result<()> {
    use anyhow::{anyhow, bail};
    const USAGE: &str =
        "usage: crisp3ds-dense splats SOURCE OUT [quality=fast|standard|best] [steps=N stop=N log=N seed=N mesh=N sh=DEGREE \
hold=N init=SPLATS.npz l1=X ssim=X mask=X normal=X distortion=X] [hull_mesh=STL photos=DIR masks=DIR div=N crop=PIXELS]
SOURCE as for `crisp3ds-dense surface` (crop: 480 fast, 720 standard, 1080 best). The quality level sets the
defaults; single settings after it override them.";
    ensure!(arguments.len() >= 2, "{USAGE}");
    let (source, out) = (std::path::PathBuf::from(&arguments[0]), std::path::PathBuf::from(&arguments[1]));
    let (quality, items) = crate::surface::options::split(&arguments[2..]).map_err(|e| anyhow!("{e}\n{USAGE}"))?;
    let mut s = Settings::for_quality(quality);
    let mut from = crate::surface::options::Source::for_quality(quality, 1080);
    for (name, value) in &items {
        if !s.set(name, value)? && !from.set(name, value)? {
            bail!("unknown setting {name}\n{USAGE}");
        }
    }
    let scene = from.scene(&source, s.seed)?;
    let gpu = Gpu::new()?;
    crate::gpu::block_on(train(&gpu, &scene, &out, &s)).map(|_| ())
}
