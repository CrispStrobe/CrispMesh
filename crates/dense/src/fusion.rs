//! Depth fusion into a truncated signed distance volume: port of `Stereo.tsdf`
//! from `scripts/turntable_mesh/multiscale_stereo.py`.
//!
//! Every hull voxel is projected into every view on the GPU; votes are added in
//! view order in float32, as the reference does. Voxels are processed as lists
//! in chunks and depth maps in batches that fit one storage binding, so the
//! default WebGPU limits are enough for any grid and view count.

use bytemuck::{Pod, Zeroable};

use crate::config::DenseConfig;
use crate::gpu::Gpu;
use crate::hull::{camera_rows, erode, Hull, VoxelList, CHUNK};
use crate::inputs::{Camera, Plane};
use crate::repair::orbit_down;
use crate::stereo::level::LevelView;

/// Support plane found by fusion: a point on the orbit axis, the direction
/// from the cameras to the object side, and the height of the lowest
/// well-supported measured surface along it (none without enough surface).
#[derive(Debug, Clone, Copy)]
pub struct Support {
    pub point: [f32; 3],
    pub down: [f32; 3],
    pub height: Option<f32>,
}

pub struct Fused {
    /// Linear indices of the hull voxels, in C order.
    pub indices: Vec<u32>,
    pub total: Vec<f32>,
    pub weight: Vec<f32>,
    pub truncation: f64,
    pub support: Support,
    /// The views do not go around the object (`partial_views`).
    pub partial_views: bool,
    /// The run treated the silhouette hull as the surface (`silhouette_first`); the surface stage follows.
    pub silhouette_first: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FuseParams {
    count: u32,
    views: u32,
    nx: u32,
    ny: u32,
    nz: u32,
    interpolate: u32,
    pad: [u32; 2],
    truncation: f32,
    behind: f32,
    behind_weight: f32,
    free_weight: f32,
}

/// Depth noise measured from the depth maps themselves, no reference: every
/// `stride`-th pixel is projected into its `voters`, and the robust spread
/// (1.4826 x median absolute deviation, world units) of the differences to the
/// depth interpolated there estimates that pixel's noise. Differences beyond `outlier`
/// (relative) are another surface or an occlusion, not noise. Returns the
/// median over pixels with at least two such differences, and their count.
pub fn depth_noise(level: &[LevelView], depths: &[Plane<f32>], voters: &[Vec<usize>], stride: usize, outlier: f32) -> Option<(f64, usize)> {
    let stride = stride.max(1);
    let per_view: Vec<Vec<f64>> = crate::inputs::parallel_map(level.len(), |i| {
        let (view, depth) = (&level[i], &depths[i]);
        let mut spreads = Vec::new();
        let mut differences: Vec<f32> = Vec::with_capacity(voters[i].len());
        for y in (0..depth.height).step_by(stride) {
            for x in (0..depth.width).step_by(stride) {
                let d = depth.data[y * depth.width + x];
                if d <= 0.0 {
                    continue;
                }
                let world = view.unproject(x, y, d);
                differences.clear();
                for &j in &voters[i] {
                    let (xy, z) = level[j].project(world);
                    // Interpolated, so slanted surfaces do not read as noise; no sample across gaps or jumps.
                    let other = interpolated_depth(&depths[j], xy, outlier * z, 0.0);
                    if z > 0.0 && other > 0.0 && (other - z).abs() <= outlier * z {
                        differences.push((other - z).abs());
                    }
                }
                if differences.len() >= 2 {
                    differences.sort_by(f32::total_cmp);
                    let n = differences.len();
                    let median = if n % 2 == 1 { differences[n / 2] } else { 0.5 * (differences[n / 2 - 1] + differences[n / 2]) };
                    spreads.push(1.4826 * median as f64);
                }
            }
        }
        spreads
    });
    let mut all: Vec<f64> = per_view.into_iter().flatten().collect();
    if all.len() < 100 {
        return None;
    }
    let count = all.len();
    Some((crate::inputs::median_f64(&mut all), count))
}

/// Depths confirmed across directions: a pixel stays when at least `votes` of up to `targets` views whose
/// direction differs by `minimum..=maximum` degrees (spread evenly over that range) see the same surface
/// point within `tolerance` (relative depth). Views that see another surface there do not count either way.
/// Returns the filtered maps and the kept fraction of the valid pixels.
pub fn cross_checked(
    level: &[LevelView],
    depths: &[Plane<f32>],
    angles: &[Vec<f64>],
    (minimum, maximum): (f64, f64),
    targets: usize,
    tolerance: f32,
    votes: usize,
) -> (Vec<Plane<f32>>, f64) {
    let checked: Vec<(Plane<f32>, usize, usize)> = crate::inputs::parallel_map(level.len(), |i| {
        let mut order: Vec<usize> = (0..level.len()).filter(|&j| j != i && minimum <= angles[i][j] && angles[i][j] <= maximum).collect();
        order.sort_by(|&a, &b| angles[i][a].total_cmp(&angles[i][b]).then(a.cmp(&b)));
        if order.len() > targets && targets > 1 {
            order = (0..targets).map(|k| order[k * (order.len() - 1) / (targets - 1)]).collect();
        }
        let (view, depth) = (&level[i], &depths[i]);
        let mut out = Plane { width: depth.width, height: depth.height, data: depth.data.clone() };
        let (mut valid, mut kept) = (0usize, 0usize);
        for y in 0..depth.height {
            for x in 0..depth.width {
                let d = depth.data[y * depth.width + x];
                if d <= 0.0 {
                    continue;
                }
                valid += 1;
                let world = view.unproject(x, y, d);
                let mut agreeing = 0;
                for &j in &order {
                    let (xy, z) = level[j].project(world);
                    if z <= 0.0 {
                        continue;
                    }
                    let (px, py) = (xy[0].round_ties_even(), xy[1].round_ties_even());
                    if !(px >= 0.0 && py >= 0.0 && px < depths[j].width as f32 && py < depths[j].height as f32) {
                        continue;
                    }
                    let nearest = depths[j].at(px as usize, py as usize);
                    let other = interpolated_depth(&depths[j], xy, tolerance * z, nearest);
                    if other > 0.0 && (other - z).abs() <= tolerance * z {
                        agreeing += 1;
                        if agreeing >= votes {
                            break;
                        }
                    }
                }
                if agreeing >= votes {
                    kept += 1;
                } else {
                    out.data[y * depth.width + x] = 0.0;
                }
            }
        }
        (out, valid, kept)
    });
    let (valid, kept) = checked.iter().fold((0usize, 0usize), |(v, k), c| (v + c.1, k + c.2));
    (checked.into_iter().map(|c| c.0).collect(), kept as f64 / valid.max(1) as f64)
}

/// Masks grown by the depths: every `stride`-th depth pixel (or fewer) of every view is a surface point; where the
/// points land in another view, dilated by the stride and closed by `close` pixels, is object there too.
/// Holes of the union are filled. Returns the masks and the median added fraction of the mask area.
pub fn grown_masks(
    level: &[LevelView],
    depths: &[Plane<f32>],
    cameras: &[Camera],
    masks: &[Plane<u8>],
    stride: usize,
    close: usize,
) -> (Vec<Plane<u8>>, f64) {
    // At most about four million points: every one is projected into every view.
    let valid: usize = depths.iter().map(|d| d.data.iter().filter(|&&v| v > 0.0).count()).sum();
    let stride = stride.max(1).max((valid as f64 / 4e6).sqrt().ceil() as usize);
    let points: Vec<[f32; 3]> = crate::inputs::parallel_map(level.len(), |i| {
        let (view, depth) = (&level[i], &depths[i]);
        let mut points = Vec::new();
        for y in (0..depth.height).step_by(stride) {
            for x in (0..depth.width).step_by(stride) {
                let d = depth.data[y * depth.width + x];
                if d > 0.0 {
                    points.push(view.unproject(x, y, d));
                }
            }
        }
        points
    })
    .into_iter()
    .flatten()
    .collect();
    let grown: Vec<(Plane<u8>, f64)> = crate::inputs::parallel_map(masks.len(), |n| {
        let (mask, camera) = (&masks[n], &cameras[n]);
        let (w, h) = (mask.width, mask.height);
        let mut landed = Plane { width: w, height: h, data: vec![0u8; w * h] };
        // The level may be smaller than the photo: one level pixel covers this many mask pixels.
        let scale = (camera.k[0] / level[n].camera.k[0]).max(1.0);
        for &point in &points {
            let p = camera.to_camera(point);
            if p[2] <= 0.0 {
                continue;
            }
            let (x, y) = ((p[0] / p[2] * camera.k[0] + camera.k[2] - 0.5).round(), (p[1] / p[2] * camera.k[1] + camera.k[3] - 0.5).round());
            if x >= 0.0 && y >= 0.0 && x < w as f32 && y < h as f32 {
                landed.data[y as usize * w + x as usize] = 1;
            }
        }
        // Points are `stride` level pixels apart: closing over that distance joins them. One level pixel is
        // then taken off, so that a fringe of slightly misplaced points along an intact mask adds nothing
        // and only solid regions do.
        let reach = (stride as f32 * scale).ceil() as usize + close;
        let closed = crate::hull::erode(&crate::hull::dilate(&landed, reach), reach + scale.ceil() as usize);
        let mut out = mask.clone();
        for (o, &c) in out.data.iter_mut().zip(&closed.data) {
            *o |= c;
        }
        fill_holes(&mut out);
        let (before, after) = (mask.data.iter().filter(|&&m| m != 0).count(), out.data.iter().filter(|&&m| m != 0).count());
        (out, (after - before) as f64 / before.max(1) as f64)
    });
    let mut added: Vec<f64> = grown.iter().map(|g| g.1).collect();
    let median = if added.is_empty() { 0.0 } else { crate::inputs::median_f64(&mut added) };
    (grown.into_iter().map(|g| g.0).collect(), median)
}

/// Sets every background pixel that is not connected to the image border.
fn fill_holes(mask: &mut Plane<u8>) {
    let (w, h) = (mask.width, mask.height);
    if w == 0 || h == 0 {
        return;
    }
    let mut outside = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let seed = |p: usize, outside: &mut Vec<bool>, stack: &mut Vec<usize>| {
        if mask.data[p] == 0 && !outside[p] {
            outside[p] = true;
            stack.push(p);
        }
    };
    for x in 0..w {
        seed(x, &mut outside, &mut stack);
        seed((h - 1) * w + x, &mut outside, &mut stack);
    }
    for y in 0..h {
        seed(y * w, &mut outside, &mut stack);
        seed(y * w + w - 1, &mut outside, &mut stack);
    }
    while let Some(p) = stack.pop() {
        let (x, y) = (p % w, p / w);
        if x > 0 {
            seed(p - 1, &mut outside, &mut stack);
        }
        if x + 1 < w {
            seed(p + 1, &mut outside, &mut stack);
        }
        if y > 0 {
            seed(p - w, &mut outside, &mut stack);
        }
        if y + 1 < h {
            seed(p + w, &mut outside, &mut stack);
        }
    }
    for (m, &o) in mask.data.iter_mut().zip(&outside) {
        if !o {
            *m = 1;
        }
    }
}

/// One view's vote for a point, as the kernel computes it: reference for tests.
pub fn vote(view: &LevelView, depth: &Plane<f32>, point: [f32; 3], truncation: f32, behind: f32, config: &DenseConfig) -> (f32, f32) {
    let (xy, z) = view.project(point);
    let (x, y) = (xy[0].round_ties_even(), xy[1].round_ties_even());
    let inside = x >= 0.0 && x < depth.width as f32 && y >= 0.0 && y < depth.height as f32;
    let mut measured = if inside { depth.at(x as usize, y as usize) } else { 0.0 };
    if config.fusion_interpolate {
        measured = interpolated_depth(depth, xy, truncation, measured);
    }
    let sdf = measured - z;
    let seen = measured > 0.0 && z > 0.0;
    let near = seen && sdf > -truncation;
    let is_behind = seen && !near && sdf > -behind;
    let vote = if near {
        if sdf >= truncation {
            config.free_weight as f32
        } else {
            1.0
        }
    } else {
        0.0
    };
    let inside_vote = if is_behind { config.behind_weight as f32 } else { 0.0 };
    (vote * (sdf / truncation).min(1.0) - inside_vote, vote + inside_vote)
}

/// Inverse depth is affine on a plane in image coordinates. Do not bridge a
/// missing corner or a depth jump; retain the legacy sample in those cases.
fn interpolated_depth(depth: &Plane<f32>, xy: [f32; 2], maximum_jump: f32, fallback: f32) -> f32 {
    let (x, y) = (xy[0].floor(), xy[1].floor());
    if !(x >= 0.0 && y >= 0.0 && x + 1.0 < depth.width as f32 && y + 1.0 < depth.height as f32) {
        return fallback;
    }
    let (ix, iy) = (x as usize, y as usize);
    let values = [depth.at(ix, iy), depth.at(ix + 1, iy), depth.at(ix, iy + 1), depth.at(ix + 1, iy + 1)];
    let minimum = values.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if minimum <= 0.0 || maximum - minimum > maximum_jump {
        return fallback;
    }
    let (u, v) = (xy[0] - x, xy[1] - y);
    1.0 / ((1.0 - u) * (1.0 - v) / values[0] + u * (1.0 - v) / values[1] + (1.0 - u) * v / values[2] + u * v / values[3])
}

/// Depth with the silhouette rim removed: a window that straddles the
/// silhouette matches the apparent contour, so the rim is left to the hull.
pub fn without_rim(view: &LevelView, depth: &Plane<f32>, rim_pixels: usize) -> Plane<f32> {
    if rim_pixels == 0 {
        return depth.clone();
    }
    let inner = erode(&view.mask, rim_pixels);
    Plane {
        width: depth.width,
        height: depth.height,
        data: depth.data.iter().zip(&inner.data).map(|(&d, &m)| if m != 0 { d } else { 0.0 }).collect(),
    }
}

/// `Stereo.tsdf`: average truncated signed distance per hull voxel, plus the support height.
pub async fn tsdf(
    gpu: &Gpu,
    hull: &Hull,
    cameras: &[Camera],
    level: &[LevelView],
    depths: &[Plane<f32>],
    rim_pixels: usize,
    config: &DenseConfig,
) -> anyhow::Result<Fused> {
    let kernel = gpu.kernel("fuse", include_str!("shaders/fuse.wgsl"), "main").await?;
    let indices = hull.indices();
    let list = VoxelList::new(gpu, hull, &indices);
    let truncation = config.truncation_voxels * hull.voxel;
    let behind = config.behind_voxels * hull.voxel;

    // Depth maps in batches of whole views that fit one binding.
    let budget = (gpu.binding_budget() / 4) as usize;
    let mut batches: Vec<(u32, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer)> = Vec::new();
    let (mut rows, mut frames, mut values) = (Vec::<[f32; 4]>::new(), Vec::<[u32; 4]>::new(), Vec::<f32>::new());
    let mut flush = |rows: &mut Vec<[f32; 4]>, frames: &mut Vec<[u32; 4]>, values: &mut Vec<f32>| {
        if !frames.is_empty() {
            batches.push((
                frames.len() as u32,
                gpu.upload("fuse cameras", rows),
                gpu.upload("fuse frames", frames),
                gpu.upload("fuse depths", values),
            ));
            rows.clear();
            frames.clear();
            values.clear();
        }
    };
    for (view, depth) in level.iter().zip(depths) {
        anyhow::ensure!(
            (depth.width, depth.height) == (view.width, view.height),
            "depth map of {}x{} does not match the level size {}x{}",
            depth.width,
            depth.height,
            view.width,
            view.height
        );
        if !frames.is_empty() && values.len() + depth.data.len() > budget {
            flush(&mut rows, &mut frames, &mut values);
        }
        frames.push([depth.width as u32, depth.height as u32, values.len() as u32, 0]);
        rows.extend_from_slice(&camera_rows(&view.camera));
        values.extend_from_slice(&without_rim(view, depth, rim_pixels).data);
    }
    flush(&mut rows, &mut frames, &mut values);

    let totals = gpu.zeroed("fuse total", 4 * CHUNK as u64);
    let weights = gpu.zeroed("fuse weight", 4 * CHUNK as u64);
    let (mut total, mut weight) = (Vec::with_capacity(indices.len()), Vec::with_capacity(indices.len()));
    for (voxels, count) in &list.chunks {
        gpu.clear(&totals);
        gpu.clear(&weights);
        for (views, cameras, frames, values) in &batches {
            let params = gpu.uniform(
                "fuse params",
                &FuseParams {
                    count: *count as u32,
                    views: *views,
                    nx: hull.shape[0] as u32,
                    ny: hull.shape[1] as u32,
                    nz: hull.shape[2] as u32,
                    interpolate: config.fusion_interpolate as u32,
                    pad: [0; 2],
                    truncation: truncation as f32,
                    behind: behind as f32,
                    behind_weight: config.behind_weight as f32,
                    free_weight: config.free_weight as f32,
                },
            );
            gpu.run(&kernel, &[&params, cameras, frames, values, &list.axes, voxels, &totals, &weights], *count as u64).await?;
        }
        total.extend(gpu.read::<f32>(&totals, *count).await?);
        weight.extend(gpu.read::<f32>(&weights, *count).await?);
    }

    // Support height: silhouettes cannot tell a flat base from a cone under it,
    // but no photo measures surface below the support. Take the lowest level
    // that still has well-supported measured surface.
    let (middle, down) = orbit_down(cameras, hull);
    let axes = [hull.axis(0), hull.axis(1), hull.axis(2)];
    let mut heights: Vec<f32> = Vec::new();
    for (n, &linear) in indices.iter().enumerate() {
        if weight[n] >= 3.0 && (total[n] / weight[n].max(1e-6)).abs() < 0.5 {
            let [i, j, k] = hull.unravel(linear);
            let p = [axes[0][i], axes[1][j], axes[2][k]];
            heights.push((p[0] - middle[0]) * down[0] + (p[1] - middle[1]) * down[1] + (p[2] - middle[2]) * down[2]);
        }
    }
    let mut height = None;
    if heights.len() > 1000 {
        heights.sort_unstable_by(f32::total_cmp);
        height = Some(heights[(0.998 * (heights.len() - 1) as f64) as usize]);
    }
    if config.support_evidence {
        if let Some(found) = height {
            if support_evidence(hull, cameras, &Support { point: middle, down, height: Some(found) }).floating {
                height = None;
            }
        }
    }
    // The object's centre for `partial_views`: the hull's centroid (the cameras' mean lies off-centre
    // whenever the views do not surround the object).
    let mut centroid = [0.0f64; 3];
    for &linear in indices.iter().step_by(7) {
        let [i, j, k] = hull.unravel(linear);
        for (a, value) in [axes[0][i], axes[1][j], axes[2][k]].into_iter().enumerate() {
            centroid[a] += f64::from(value);
        }
    }
    let sampled = indices.len().div_ceil(7).max(1) as f64;
    let partial = partial_views(cameras, &centroid.map(|v| (v / sampled) as f32));
    Ok(Fused {
        indices,
        total,
        weight,
        truncation,
        support: Support { point: middle, down, height },
        partial_views: partial,
        silhouette_first: false,
    })
}

/// Whether the views leave part of the object unseen: the directions from the object to the cameras,
/// as unit vectors, average to a long vector when they all lie on one side (DTU's arc of views from
/// the front: about 0.8) and nearly cancel for a ring around it (a turntable at 10 to 20 degrees of
/// elevation: below 0.35). Partial above 0.6.
pub fn partial_views(cameras: &[Camera], centre: &[f32; 3]) -> bool {
    let mut sum = [0.0f64; 3];
    for camera in cameras {
        let c = camera.centre();
        let d = [c[0] - centre[0], c[1] - centre[1], c[2] - centre[2]].map(f64::from);
        let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
        for a in 0..3 {
            sum[a] += d[a] / n;
        }
    }
    let count = cameras.len().max(1) as f64;
    (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt() / count > 0.6
}

/// Whether the cameras look at the object from several heights all the way round, so that the silhouette
/// hull is close to the object everywhere but underneath. The point the cameras look at is the one nearest
/// to all optical axes; the axis is the normal of the plane of the camera centres (up, for a turntable).
/// Elevations of the cameras seen from that point must reach down to 25 degrees, span
/// at least 40 degrees (5th to 95th percentile) with each third of the span holding a tenth of the views,
/// and leave no 30-degree sector around the axis empty. One turn has no span; five turns from 8 to 85
/// degrees span about 75; views from one side (a cap around its own normal) do not reach down.
pub fn several_heights(cameras: &[Camera]) -> bool {
    if cameras.len() < 12 {
        return false;
    }
    let centres: Vec<[f64; 3]> = cameras.iter().map(|c| c.centre().map(f64::from)).collect();
    // Nearest point to all optical axes: sum (I - d d^T) (p - c) = 0.
    let (mut m, mut rhs) = ([[0.0f64; 3]; 3], [0.0f64; 3]);
    for (camera, c) in cameras.iter().zip(&centres) {
        let d = camera.rotation[2].map(f64::from);
        for i in 0..3 {
            for j in 0..3 {
                let entry = if i == j { 1.0 } else { 0.0 } - d[i] * d[j];
                m[i][j] += entry;
                rhs[i] += entry * c[j];
            }
        }
    }
    let det = |a: &[[f64; 3]; 3]| {
        a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    };
    let whole = det(&m);
    if whole.abs() < 1e-9 {
        return false;
    }
    let mut target = [0.0f64; 3];
    for (k, slot) in target.iter_mut().enumerate() {
        let mut replaced = m;
        for (row, value) in replaced.iter_mut().zip(&rhs) {
            row[k] = *value;
        }
        *slot = det(&replaced) / whole;
    }
    let unit = |v: [f64; 3]| {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        v.map(|x| x / n)
    };
    let directions: Vec<[f64; 3]> = centres.iter().map(|c| unit([c[0] - target[0], c[1] - target[1], c[2] - target[2]])).collect();
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // The axis is the normal of the camera centres' plane (a ring level with the object has no mean
    // direction), pointing to the cameras' side.
    let count = centres.len() as f64;
    let middle = [0, 1, 2].map(|a| centres.iter().map(|c| c[a]).sum::<f64>() / count);
    let mut scatter = [[0.0f64; 3]; 3];
    for c in &centres {
        let d = [0, 1, 2].map(|a| c[a] - middle[a]);
        for i in 0..3 {
            for j in 0..3 {
                scatter[i][j] += d[i] * d[j];
            }
        }
    }
    let mut axis = crate::repair::smallest_eigenvector(scatter);
    if directions.iter().map(|&d| dot(d, axis)).sum::<f64>() < 0.0 {
        axis = axis.map(|v| -v);
    }
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let u = unit(cross(axis, if axis[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
    let v = cross(axis, u);
    let mut sectors = [false; 12];
    for &d in &directions {
        sectors[((dot(d, v).atan2(dot(d, u)).to_degrees() + 180.0) / 30.0).floor().clamp(0.0, 11.0) as usize] = true;
    }
    if sectors.contains(&false) {
        return false;
    }
    let mut elevations: Vec<f64> = directions.iter().map(|&d| dot(d, axis).clamp(-1.0, 1.0).asin().to_degrees()).collect();
    elevations.sort_by(f64::total_cmp);
    let at = |q: f64| elevations[(q * (elevations.len() - 1) as f64).round() as usize];
    let (low, high) = (at(0.05), at(0.95));
    let span = high - low;
    if span < 40.0 || low > 25.0 {
        return false;
    }
    (0..3).all(|third| {
        let (from, to) = (low + span * third as f64 / 3.0, low + span * (third + 1) as f64 / 3.0);
        elevations.iter().filter(|&&e| e >= from - 1e-9 && e <= to + 1e-9).count() * 10 >= elevations.len()
    })
}

/// What the hull below a support height says about whether the object stands on anything.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SupportEvidence {
    /// Deepest hull voxel below the support, voxels.
    pub depth: f32,
    /// Height of the hull above the support, voxels.
    pub height: f32,
    /// Depth of the cone the views leave under the footprint: tan(elevation) times its radius, voxels.
    pub cone: f32,
    /// Median elevation of the cameras over the support point, degrees.
    pub elevation: f32,
    /// No support: the hull goes on below the lowest measured level as the object itself would.
    pub floating: bool,
}

/// Decides whether a support height is a support. Under an object standing on a
/// plane, the silhouette hull reaches below the plane only by the cone the views
/// leave under the footprint: no camera sees through the plane, so every voxel
/// below it is one the masks cannot exclude, and from cameras at elevation `e`
/// that region ends `tan(e)` times the footprint radius below the plane. That
/// is shallow next to the object (at most 5.5 % of its height on the four test
/// objects). An object without a support whose underside no view measures (a
/// sphere seen from a low ring) has its real bottom there instead: the hull
/// goes on by a large part of its height (26 % for the demo sphere from a
/// 10-degree ring and for the asymmetric capture from a 20-degree ring).
/// Floating therefore means deeper than 15 % of the height and deeper than
/// the cone the cameras' elevation explains (by more than 15 %). With one low
/// ring the two kinds of cone have nearly the same slope (the demo sphere 1.75
/// times the cone, the asymmetric test object 1.29); cameras that look down
/// steeply (YCB, about 52 degrees) make the support's own cone deep, more than
/// 15 % of a box's height, but no deeper than tan(elevation) times the
/// footprint radius, so the support is kept. A wide, flat object on a support
/// keeps it for the same reason.
pub fn support_evidence(hull: &Hull, cameras: &[Camera], support: &Support) -> SupportEvidence {
    let none = SupportEvidence { depth: 0.0, height: 0.0, cone: 0.0, elevation: 0.0, floating: false };
    let Some(level) = support.height else { return none };
    let (point, down) = (support.point, support.down);
    let voxel = hull.voxel as f32;
    let axes = [hull.axis(0), hull.axis(1), hull.axis(2)];
    let (mut depth, mut top, mut footprint) = (f32::MIN, f32::MAX, 0usize);
    for linear in hull.indices() {
        let [i, j, k] = hull.unravel(linear);
        let p = [axes[0][i] - point[0], axes[1][j] - point[1], axes[2][k] - point[2]];
        let below = ((p[0] * down[0] + p[1] * down[1] + p[2] * down[2]) - level) / voxel;
        depth = depth.max(below);
        top = top.min(below);
        footprint += (below > -1.5 && below <= -0.5) as usize;
    }
    if depth == f32::MIN {
        return none;
    }
    let base = [point[0] + level * down[0], point[1] + level * down[1], point[2] + level * down[2]];
    let mut elevations: Vec<f32> = cameras
        .iter()
        .map(|camera| {
            let c = camera.centre();
            let r = [c[0] - base[0], c[1] - base[1], c[2] - base[2]];
            let length = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt().max(1e-12);
            (-(r[0] * down[0] + r[1] * down[1] + r[2] * down[2]) / length).clamp(-1.0, 1.0).asin()
        })
        .collect();
    elevations.sort_unstable_by(f32::total_cmp);
    let elevation = elevations.get(elevations.len() / 2).copied().unwrap_or(0.0);
    let cone = elevation.max(0.0).tan() * (footprint as f32 / std::f32::consts::PI).sqrt();
    let height = -top;
    let floating = depth > 0.15 * height && depth > 1.15 * cone;
    SupportEvidence { depth: depth.max(0.0), height, cone, elevation: elevation.to_degrees(), floating }
}

/// Keeps the support from lying below the lowest matched features.
///
/// A mask made by thresholding takes the contact shadow under the object for
/// object, so the silhouettes reach lower than the object does, and dense
/// depth continues the wall of the object down through the featureless shadow:
/// the lowest measured surface then lies below the real base and the flat base
/// is cut too low. The sparse points are features matched across photographs,
/// which a soft shadow does not give; where they stop (but for a few wrong
/// matches) the object or the plate it stands on stops. A support below that level is raised to it; one above it is left
/// alone, so masks that follow the object change nothing. Returns the two
/// levels (measured, sparse) along `down` when there are enough points.
pub fn support_from_sparse(support: &mut Support, sparse: &[[f64; 3]]) -> Option<(f32, f32)> {
    let measured = support.height?;
    if sparse.len() < 500 {
        return None;
    }
    let (point, down) = (support.point, support.down);
    let mut heights: Vec<f32> = sparse
        .iter()
        .map(|p| (p[0] as f32 - point[0]) * down[0] + (p[1] as f32 - point[1]) * down[1] + (p[2] as f32 - point[2]) * down[2])
        .collect();
    heights.sort_unstable_by(f32::total_cmp);
    // Five points, or one in a thousand, may be wrong matches below the support.
    let outliers = (heights.len() / 1000).max(5);
    let lowest = heights[heights.len() - 1 - outliers];
    support.height = Some(measured.min(lowest));
    Some((measured, lowest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::build_hull;
    use crate::inputs::Inputs;
    use crate::stereo::level::build_level;
    use crate::stereo::{options, synthetic};

    /// A camera at `centre` (identity rotation: only the centre matters here).
    fn at(centre: [f32; 3]) -> Camera {
        Camera { rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation: centre.map(|v| -v), k: [1.0; 4] }
    }

    /// A ring at 10 to 50 degrees of elevation surrounds the object; an arc of views from one side
    /// (DTU: about 100 degrees of azimuth) does not.
    #[test]
    fn partial_views_tells_an_arc_from_a_ring() {
        let ring = |elevation: f32, from: f32, to: f32, count: usize| -> Vec<Camera> {
            (0..count)
                .map(|n| {
                    let azimuth = (from + (to - from) * n as f32 / count as f32).to_radians();
                    let e = elevation.to_radians();
                    at([5.0 * e.cos() * azimuth.cos(), 5.0 * e.cos() * azimuth.sin(), 5.0 * e.sin()])
                })
                .collect()
        };
        for elevation in [10.0, 20.0, 35.0] {
            assert!(!partial_views(&ring(elevation, 0.0, 360.0, 48), &[0.0; 3]), "ring at {elevation}");
        }
        let mut arc = ring(15.0, -50.0, 50.0, 25);
        arc.extend(ring(40.0, -50.0, 50.0, 24));
        assert!(partial_views(&arc, &[0.0; 3]));
    }

    /// The noise measured between views is near zero for exact depth and follows added noise:
    /// the spread of differences between two independently noisy maps is about sqrt(2) sigma.
    #[test]
    fn depth_noise_measures_the_disagreement_between_views() {
        let root = synthetic::temporary("noise", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let level = build_level(&inputs, 128);
        let voters: Vec<Vec<usize>> = (0..inputs.count()).map(|i| inputs.neighbours(i, 6, &config).unwrap()).collect();
        let exact: Vec<Plane<f32>> = level.iter().map(synthetic::sphere_depth).collect();
        let (clean, samples) = depth_noise(&level, &exact, &voters, 1, 0.02).unwrap();
        assert!(samples > 1000, "{samples}");
        // Deterministic, roughly normal noise of relative sigma 0.002 (sum of 12 uniforms).
        let sigma = 0.002f32;
        let mut state = 12345u64;
        let mut uniform = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        let noisy: Vec<Plane<f32>> = exact
            .iter()
            .map(|d| {
                let mut d = d.clone();
                for v in d.data.iter_mut().filter(|v| **v > 0.0) {
                    let normal: f32 = (0..12).map(|_| uniform()).sum::<f32>() - 6.0;
                    *v *= 1.0 + sigma * normal;
                }
                d
            })
            .collect();
        let (measured, _) = depth_noise(&level, &noisy, &voters, 1, 0.02).unwrap();
        let depth = 5.0; // the synthetic ring is 6 from the unit sphere: its front is about 5 away
        let expected = std::f64::consts::SQRT_2 * sigma as f64 * depth;
        assert!(clean < 0.15 * expected, "exact depths measured {clean}, expected near 0");
        assert!((0.6 * expected..1.6 * expected).contains(&measured), "measured {measured}, expected about {expected}");
    }

    /// GPU fusion of exact sphere depth against the scalar votes (`CRISPMESH_GPU_TESTS=1`).
    #[test]
    fn gpu_fusion_matches_scalar_votes_and_finds_the_sphere() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = synthetic::temporary("fusion", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let gpu = Gpu::new().unwrap();
        let state = crate::gpu::block_on(build_hull(&gpu, &inputs, &config, &mut None)).unwrap();
        let level = build_level(&inputs, 128);
        let depths: Vec<Plane<f32>> = level.iter().map(synthetic::sphere_depth).collect();
        let rim = 4;
        let fused = crate::gpu::block_on(tsdf(&gpu, &state.hull, &inputs.cameras, &level, &depths, rim, &config)).unwrap();
        assert_eq!(fused.indices.len(), state.hull.count());
        let (truncation, behind) = (fused.truncation as f32, (config.behind_voxels * state.hull.voxel) as f32);
        let trimmed: Vec<Plane<f32>> = level.iter().zip(&depths).map(|(v, d)| without_rim(v, d, rim)).collect();
        let axes = [state.hull.axis(0), state.hull.axis(1), state.hull.axis(2)];
        let (mut checked, mut different, mut wrong_side) = (0, 0, 0);
        for n in (0..fused.indices.len()).step_by(23) {
            let [i, j, k] = state.hull.unravel(fused.indices[n]);
            let point = [axes[0][i], axes[1][j], axes[2][k]];
            let (mut total, mut weight) = (0.0f32, 0.0f32);
            for (view, depth) in level.iter().zip(&trimmed) {
                let (t, w) = vote(view, depth, point, truncation, behind, &config);
                total += t;
                weight += w;
            }
            checked += 1;
            if (total - fused.total[n]).abs() > 1e-3 || (weight - fused.weight[n]).abs() > 1e-3 {
                different += 1;
            }
            // Well-observed voxels clearly inside or outside the sphere carry the right sign.
            let radius = point.iter().map(|v| v * v).sum::<f32>().sqrt();
            if fused.weight[n] >= 3.0 {
                let mean = fused.total[n] / fused.weight[n];
                wrong_side += ((radius < 0.9 && mean > 0.0) || (radius > 1.1 && mean < 0.0)) as usize;
            }
        }
        assert!(checked > 3000, "{checked}");
        assert!(different * 500 <= checked, "{different} of {checked} voxels differ");
        assert_eq!(wrong_side, 0);
        let subpixel = DenseConfig { fusion_interpolate: true, ..config.clone() };
        let interpolated = crate::gpu::block_on(tsdf(&gpu, &state.hull, &inputs.cameras, &level, &depths, rim, &subpixel)).unwrap();
        let (mut sub_checked, mut sub_different) = (0usize, 0usize);
        for n in (0..interpolated.indices.len()).step_by(23) {
            let [i, j, k] = state.hull.unravel(interpolated.indices[n]);
            let point = [axes[0][i], axes[1][j], axes[2][k]];
            let (mut total, mut weight) = (0.0f32, 0.0f32);
            for (view, depth) in level.iter().zip(&trimmed) {
                let (t, w) = vote(view, depth, point, truncation, behind, &subpixel);
                total += t;
                weight += w;
            }
            sub_checked += 1;
            sub_different += ((total - interpolated.total[n]).abs() > 1e-3 || (weight - interpolated.weight[n]).abs() > 1e-3) as usize;
        }
        assert!(sub_different * 500 <= sub_checked, "subpixel GPU/scalar mismatch {sub_different}/{sub_checked}");

        // The ring looks down by 10 degrees. The sphere floats: the hull goes on far below its lowest
        // measured level, so there is no support.
        assert!(fused.support.down[2] < -0.99, "{:?}", fused.support.down);
        assert_eq!(fused.support.height, None, "a floating sphere has no support");
        // Without that test the support is the sphere's lowest measured level.
        let assumed = DenseConfig { support_evidence: false, ..config.clone() };
        let fused = crate::gpu::block_on(tsdf(&gpu, &state.hull, &inputs.cameras, &level, &depths, rim, &assumed)).unwrap();
        let height = fused.support.height.expect("support height");
        let lowest = fused.support.point[2] - height;
        assert!(lowest < -0.5 && lowest > -1.1, "{lowest}");
    }

    /// Sparse points that stop above the measured support raise it; ones that reach lower do not.
    #[test]
    fn the_support_is_not_below_the_lowest_features() {
        let points = |lowest: f64| -> Vec<[f64; 3]> { (0..1000).map(|n| [20.0, 20.0, lowest + f64::from(n) * 0.01]).collect() };
        let support = Support { point: [20.0, 20.0, 40.0], down: [0.0, 0.0, -1.0], height: Some(36.0) };
        let mut raised = Support { ..support };
        assert_eq!(support_from_sparse(&mut raised, &points(9.0)).map(|l| l.0), Some(36.0));
        // Five points are taken for wrong matches: the sixth lowest is at z = 9.05.
        assert!((raised.height.unwrap() - 30.95).abs() < 0.001, "{:?}", raised.height);
        let mut kept = Support { ..support };
        assert!(support_from_sparse(&mut kept, &points(1.0)).is_some());
        assert_eq!(kept.height, Some(36.0));
        assert_eq!(support_from_sparse(&mut kept, &points(9.0)[..100]), None);
        let mut none = Support { height: None, ..support };
        assert_eq!(support_from_sparse(&mut none, &points(9.0)), None);
    }
}

#[cfg(test)]
mod height_tests {
    use super::*;

    /// A camera on a sphere of radius 5 looking at the origin.
    fn looking_in(elevation: f32, azimuth: f32) -> Camera {
        let (e, a) = (elevation.to_radians(), azimuth.to_radians());
        let centre = [5.0 * e.cos() * a.cos(), 5.0 * e.cos() * a.sin(), 5.0 * e.sin()];
        let forward = centre.map(|v| -v / 5.0);
        let right = [-a.sin(), a.cos(), 0.0];
        let up = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        let rotation = [right, up, forward];
        let translation = [0, 1, 2].map(|r| -(rotation[r][0] * centre[0] + rotation[r][1] * centre[1] + rotation[r][2] * centre[2]));
        Camera { rotation, translation, k: [1.0; 4] }
    }

    #[test]
    fn several_heights_needs_a_wide_and_filled_span_of_elevations() {
        let turns = |elevations: &[f32]| -> Vec<Camera> {
            elevations.iter().flat_map(|&e| (0..36).map(move |n| looking_in(e, n as f32 * 10.0))).collect()
        };
        assert!(!several_heights(&turns(&[15.0])));
        assert!(!several_heights(&turns(&[10.0, 30.0])));
        // Two far-apart heights leave the middle of the span empty.
        assert!(!several_heights(&turns(&[10.0, 70.0])));
        assert!(several_heights(&turns(&[10.0, 35.0, 60.0])));
        assert!(several_heights(&turns(&[8.0, 25.0, 45.0, 65.0, 85.0])));
        // Three heights over a third of a turn (views from one side) do not go round the object.
        let arc: Vec<Camera> = [10.0f32, 35.0, 60.0].iter().flat_map(|&e| (0..36).map(move |n| looking_in(e, n as f32 * 3.0))).collect();
        assert!(!several_heights(&arc));
    }
}

#[cfg(test)]
mod cross_tests {
    use super::*;
    use crate::inputs::Inputs;
    use crate::stereo::level::build_level;
    use crate::stereo::{options, synthetic};

    fn scene(name: &str) -> (Inputs, Vec<LevelView>, Vec<Plane<f32>>) {
        let root = synthetic::temporary(name, 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let level = build_level(&inputs, 128);
        let exact = level.iter().map(synthetic::sphere_depth).collect();
        (inputs, level, exact)
    }

    /// Exact depths confirm each other; one view whose depths are 2 % off is confirmed by none and
    /// loses its depths, while the others keep theirs.
    #[test]
    fn cross_check_keeps_agreeing_depths_and_empties_a_view_that_disagrees() {
        let (inputs, level, exact) = scene("cross-check");
        let valid = |d: &Plane<f32>| d.data.iter().filter(|&&v| v > 0.0).count();
        let (checked, kept) = cross_checked(&level, &exact, &inputs.angles, (20.0, 80.0), 16, 0.004, 2);
        assert!(kept > 0.85, "{kept}");
        assert!(checked.iter().zip(&exact).all(|(c, e)| c.data.iter().zip(&e.data).all(|(&c, &e)| c == e || c == 0.0)));
        let mut wrong = exact.clone();
        wrong[5].data.iter_mut().for_each(|v| *v *= 1.02);
        let (checked, _) = cross_checked(&level, &wrong, &inputs.angles, (20.0, 80.0), 16, 0.004, 2);
        assert!(valid(&checked[5]) * 20 < valid(&exact[5]), "{} of {}", valid(&checked[5]), valid(&exact[5]));
        assert!(valid(&checked[12]) * 10 > valid(&exact[12]) * 8);
    }

    /// A mask that lost a third of the object gets it back from the depths of the other views; intact
    /// masks hardly change.
    #[test]
    fn masks_grow_back_where_confirmed_depths_land() {
        let (inputs, level, exact) = scene("cross-grow");
        let area = |m: &Plane<u8>| m.data.iter().filter(|&&v| v != 0).count();
        let mut masks = inputs.masks.clone();
        let (w, whole) = (masks[3].width, area(&masks[3]));
        let columns: Vec<usize> = (0..masks[3].data.len()).filter(|&p| masks[3].data[p] != 0).map(|p| p % w).collect();
        let cut = columns.iter().min().unwrap() + (columns.iter().max().unwrap() - columns.iter().min().unwrap()) / 3;
        for p in 0..masks[3].data.len() {
            if p % w < cut {
                masks[3].data[p] = 0;
            }
        }
        assert!(area(&masks[3]) * 10 < whole * 9);
        let mut depths = exact.clone();
        depths[3].data.fill(0.0);
        let (grown, _) = grown_masks(&level, &depths, &inputs.cameras, &masks, 2, 4);
        let back = area(&grown[3]) as f64 / whole as f64;
        assert!((0.90..1.03).contains(&back), "{back}");
        let other = area(&grown[10]) as f64 / area(&inputs.masks[10]) as f64;
        assert!((1.0..1.01).contains(&other), "{other}");
    }

    #[test]
    fn holes_are_filled_and_the_outside_is_not() {
        let mut mask = Plane { width: 5, height: 5, data: vec![0u8; 25] };
        for p in [6, 7, 8, 11, 13, 16, 17, 18] {
            mask.data[p] = 1;
        }
        fill_holes(&mut mask);
        assert_eq!(mask.data[12], 1);
        assert_eq!(mask.data.iter().filter(|&&v| v != 0).count(), 9);
    }
}

#[cfg(test)]
mod interpolation_tests {
    use super::*;
    #[test]
    fn subpixel_depth_preserves_a_plane_and_does_not_bridge_gaps_or_jumps() {
        let plane = Plane { width: 2, height: 2, data: vec![1.0, 1.0 / 1.1, 1.0 / 1.2, 1.0 / 1.3] };
        let expected = 1.0 / (1.0 + 0.1 * 0.25 + 0.2 * 0.75);
        assert!((interpolated_depth(&plane, [0.25, 0.75], 0.5, 99.0) - expected).abs() < 1e-6);
        let mut gap = plane.clone();
        gap.data[3] = 0.0;
        assert_eq!(interpolated_depth(&gap, [0.25, 0.75], 0.5, 99.0), 99.0);
        let mut jump = plane.clone();
        jump.data[3] = 2.0;
        assert_eq!(interpolated_depth(&jump, [0.25, 0.75], 0.5, 99.0), 99.0);
        assert_eq!(interpolated_depth(&plane, [-0.25, 0.75], 0.5, 99.0), 99.0);
    }
}
