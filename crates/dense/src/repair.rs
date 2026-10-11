//! Multi-view mask repair: port of `Stereo.orbit_down`, `Stereo.cover` and
//! `Stereo.repair_masks` from `scripts/turntable_mesh/multiscale_stereo.py`.
//!
//! The projection of the loose hull into every view runs on the GPU; the photo
//! test (darker than the midpoint between object and backdrop) and the ring
//! median are plain image passes on the CPU.

#![allow(clippy::needless_range_loop)]

use bytemuck::{Pod, Zeroable};
use serde::Serialize;

use crate::config::DenseConfig;
use crate::gpu::{Gpu, Kernel};
use crate::hull::{camera_rows, dilate, Hull, HullState, VoxelList};
use crate::inputs::{median_f64, parallel_map, Camera, Inputs, Plane};

/// Eigenvector of the smallest eigenvalue of a symmetric 3x3 matrix (cyclic Jacobi).
pub(crate) fn smallest_eigenvector(mut a: [[f64; 3]; 3]) -> [f64; 3] {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        if off < 1e-300 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    let column = (0..3).min_by(|&i, &j| a[i][i].total_cmp(&a[j][j])).unwrap();
    [v[0][column], v[1][column], v[2][column]]
}

#[inline]
fn height(point: [f32; 3], middle: [f32; 3], down: [f32; 3]) -> f32 {
    (point[0] - middle[0]) * down[0] + (point[1] - middle[1]) * down[1] + (point[2] - middle[2]) * down[2]
}

/// `Stereo.orbit_down`: centre of the camera orbit and its normal, pointing
/// from the cameras to the object side.
///
/// The reference takes the normal from a single-precision SVD of the centred
/// camera centres; here it is the eigenvector of their scatter matrix computed
/// in double precision and rounded, which agrees to float32 rounding.
pub fn orbit_down(cameras: &[Camera], hull: &Hull) -> ([f32; 3], [f32; 3]) {
    let centres: Vec<[f32; 3]> = cameras.iter().map(Camera::centre).collect();
    let mut middle = [0.0f32; 3];
    for axis in 0..3 {
        middle[axis] = centres.iter().map(|c| c[axis]).sum::<f32>() / centres.len() as f32;
    }
    let mut scatter = [[0.0f64; 3]; 3];
    for c in &centres {
        let d = [0, 1, 2].map(|a| (c[a] - middle[a]) as f64);
        for i in 0..3 {
            for j in 0..3 {
                scatter[i][j] += d[i] * d[j];
            }
        }
    }
    let mut down = smallest_eigenvector(scatter).map(|v| v as f32);
    let axes = [hull.axis(0), hull.axis(1), hull.axis(2)];
    let (mut sum, mut count) = (0.0f64, 0usize);
    for linear in hull.indices().into_iter().step_by(5) {
        let [i, j, k] = hull.unravel(linear);
        sum += height([axes[0][i], axes[1][j], axes[2][k]], middle, down) as f64;
        count += 1;
    }
    if count > 0 && sum / (count as f64) < 0.0 {
        down = down.map(|v| -v);
    }
    (middle, down)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CoverParams {
    count: u32,
    nx: u32,
    ny: u32,
    nz: u32,
    width: u32,
    height: u32,
    stride: u32,
    pad: u32,
}

/// Projects voxel lists into views (`Stereo.cover`).
pub struct Coverer<'a> {
    gpu: &'a Gpu,
    kernel: Kernel,
}

impl<'a> Coverer<'a> {
    pub async fn new(gpu: &'a Gpu) -> anyhow::Result<Self> {
        Ok(Coverer { gpu, kernel: gpu.kernel("cover", include_str!("shaders/cover.wgsl"), "main").await? })
    }

    /// Pixels of a `width` x `height` view that the listed voxel centres project onto (2x2 splat).
    pub async fn cover(&self, list: &VoxelList, camera: &Camera, width: usize, height: usize) -> anyhow::Result<Plane<u8>> {
        let stride = width.div_ceil(32);
        let bits = self.gpu.zeroed("cover bits", (4 * stride * height) as u64);
        let rows = self.gpu.upload("cover camera", &camera_rows(camera));
        for (voxels, count) in &list.chunks {
            let params = self.gpu.uniform(
                "cover params",
                &CoverParams {
                    count: *count as u32,
                    nx: list.shape[0] as u32,
                    ny: list.shape[1] as u32,
                    nz: list.shape[2] as u32,
                    width: width as u32,
                    height: height as u32,
                    stride: stride as u32,
                    pad: 0,
                },
            );
            self.gpu.run(&self.kernel, &[&params, &rows, &list.axes, voxels, &bits], *count as u64).await?;
        }
        let words = self.gpu.read::<u32>(&bits, stride * height).await?;
        let mut out = Plane::<u8>::new(width, height);
        for y in 0..height {
            for x in 0..width {
                out.data[y * width + x] = ((words[y * stride + x / 32] >> (x % 32)) & 1) as u8;
            }
        }
        Ok(out)
    }
}

/// Scalar reference of [`Coverer::cover`] for tests.
pub fn cover_reference(points: &[[f32; 3]], camera: &Camera, width: usize, height: usize) -> Plane<u8> {
    let mut out = Plane::<u8>::new(width, height);
    for &point in points {
        let p = camera.to_camera(point);
        let u = p[0] / p[2] * camera.k[0] + camera.k[2] - 0.5;
        let v = p[1] / p[2] * camera.k[1] + camera.k[3] - 0.5;
        if u >= 0.0 && u < (width - 1) as f32 && v >= 0.0 && v < (height - 1) as f32 && p[2] > 0.0 {
            let (x, y) = (u.floor() as usize, v.floor() as usize);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                out.data[(y + dy) * width + x + dx] = 1;
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairRound {
    pub added_fraction_median: f64,
    pub added_fraction_maximum: f64,
}

/// `Stereo.repair_masks`: adds object pixels that single-view segmentation dropped.
///
/// A pixel is added only when it lies in the projection of the loose hull
/// (voxels that all but `repair_loose` views agree on, kept away from the
/// support) and is darker than the midpoint between this photo's object range
/// and its backdrop. Rewrites `inputs.masks`.
pub async fn repair_masks(gpu: &Gpu, inputs: &mut Inputs, state: &HullState, config: &DenseConfig) -> anyhow::Result<RepairRound> {
    repair_masks_with(gpu, inputs, state, config, &|| Ok(())).await
}

/// [`repair_masks`] that can be stopped: `check` is called before every view's coverage pass.
pub async fn repair_masks_with(
    gpu: &Gpu,
    inputs: &mut Inputs,
    state: &HullState,
    config: &DenseConfig,
    check: &dyn Fn() -> anyhow::Result<()>,
) -> anyhow::Result<RepairRound> {
    let (near_ring, far_ring) = (5usize, 15usize);
    let hull = &state.hull;
    let axes = [hull.axis(0), hull.axis(1), hull.axis(2)];
    let centre = |linear: u32| {
        let [i, j, k] = hull.unravel(linear);
        [axes[0][i], axes[1][j], axes[2][k]]
    };
    // Contact shadows on the support are dark and view-consistent too. Keep the
    // repair away from the base: the orbit normal gives "down", and the strict
    // hull's lowest extent gives the support height.
    let (middle, down) = orbit_down(&inputs.cameras, hull);
    let mut heights: Vec<f32> = hull.indices().into_iter().step_by(5).map(|linear| height(centre(linear), middle, down)).collect();
    heights.sort_unstable_by(f32::total_cmp);
    let base = heights[(0.995 * (heights.len() - 1) as f64) as usize] as f64;
    let limit = (base - config.repair_base_margin * hull.voxel) as f32;
    let loose = inputs.repair_loose.clamp(0, u16::MAX as i64) as u16;
    let indices: Vec<u32> = state
        .violations
        .iter()
        .enumerate()
        .filter(|(linear, &count)| count <= loose && height(centre(*linear as u32), middle, down) < limit)
        .map(|(linear, _)| linear as u32)
        .collect();
    let list = VoxelList::new(gpu, hull, &indices);
    let coverer = Coverer::new(gpu).await?;
    let mut covers = Vec::with_capacity(inputs.count());
    for n in 0..inputs.count() {
        check()?;
        covers.push(coverer.cover(&list, &inputs.cameras[n], inputs.masks[n].width, inputs.masks[n].height).await?);
    }
    let repaired: Vec<(Plane<u8>, f64)> = parallel_map(inputs.count(), |n| {
        let (mask, cover) = (&inputs.masks[n], &covers[n]);
        // Outside the grey plane (photos loaded as canvas crops) a pixel counts as not dark.
        let gray = |p: usize| inputs.gray_at(n, p % mask.width, p / mask.width);
        let (near, far) = (dilate(mask, near_ring), dilate(mask, far_ring));
        let mut ring: Vec<f32> = (0..mask.data.len()).filter(|&p| far.data[p] != 0 && near.data[p] == 0).filter_map(gray).collect();
        // torch.median: the lower of the two middle values.
        let backdrop = if ring.is_empty() {
            f32::NAN
        } else {
            let middle = (ring.len() - 1) / 2;
            *ring.select_nth_unstable_by(middle, f32::total_cmp).1
        };
        let threshold = (1.0 + 0.5 * (backdrop as f64 - 1.0)) as f32;
        let mut out = mask.clone();
        let (mut added, mut before) = (0usize, 0usize);
        for p in 0..mask.data.len() {
            before += mask.data[p] as usize;
            if cover.data[p] != 0 && mask.data[p] == 0 && gray(p).is_some_and(|g| g < threshold) {
                out.data[p] = 1;
                added += 1;
            }
        }
        (out, (added as f32 / before as f32) as f64)
    });
    let mut fractions = Vec::with_capacity(repaired.len());
    for (n, (mask, fraction)) in repaired.into_iter().enumerate() {
        inputs.masks[n] = mask;
        fractions.push(fraction);
    }
    let maximum = fractions.iter().cloned().fold(f64::MIN, f64::max);
    Ok(RepairRound { added_fraction_median: median_f64(&mut fractions), added_fraction_maximum: maximum })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smallest_eigenvector_of_a_tilted_ring() {
        // Points on a circle in the plane with normal n: the scatter's smallest eigenvector is n.
        let n = [0.2f64, -0.3, 0.9327379053088815];
        let u = [0.0, n[2], -n[1]];
        let ul = (u[1] * u[1] + u[2] * u[2]).sqrt();
        let u = u.map(|v| v / ul);
        let w = [n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0]];
        let mut scatter = [[0.0; 3]; 3];
        for step in 0..24 {
            let a = step as f64 * std::f64::consts::TAU / 24.0;
            let p = [0, 1, 2].map(|k| 3.0 * (a.cos() * u[k] + a.sin() * w[k]));
            for i in 0..3 {
                for j in 0..3 {
                    scatter[i][j] += p[i] * p[j];
                }
            }
        }
        let found = smallest_eigenvector(scatter);
        let dot = found[0] * n[0] + found[1] * n[1] + found[2] * n[2];
        assert!((dot.abs() - 1.0).abs() < 1e-12, "{found:?}");
    }

    #[test]
    fn reference_cover_splats_two_by_two() {
        let camera =
            Camera { rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation: [0.0; 3], k: [10.0, 10.0, 4.0, 4.0] };
        // (0.12, -0.2, 1) projects to u = 4.7, v = 1.5.
        let cover = cover_reference(&[[0.12, -0.2, 1.0], [0.0, 0.0, -1.0], [5.0, 0.0, 1.0]], &camera, 8, 8);
        let set: Vec<usize> = cover.data.iter().enumerate().filter(|(_, &v)| v != 0).map(|(n, _)| n).collect();
        assert_eq!(set, vec![8 + 4, 8 + 5, 16 + 4, 16 + 5]);
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::hull::build_hull;
    use crate::stereo::{options, synthetic};

    /// GPU cover against the scalar splat; repair leaves a clean scene alone (`CRISP3DS_GPU_TESTS=1`).
    #[test]
    fn gpu_cover_matches_the_scalar_splat() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = synthetic::temporary("repair", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let mut inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        crate::storage::remove_dir_all(&root).unwrap();
        let gpu = Gpu::new().unwrap();
        let state = crate::gpu::block_on(build_hull(&gpu, &inputs, &config, &mut None)).unwrap();
        let indices = state.hull.indices();
        let axes = [state.hull.axis(0), state.hull.axis(1), state.hull.axis(2)];
        let points: Vec<[f32; 3]> = indices
            .iter()
            .map(|&linear| {
                let [i, j, k] = state.hull.unravel(linear);
                [axes[0][i], axes[1][j], axes[2][k]]
            })
            .collect();
        let list = VoxelList::new(&gpu, &state.hull, &indices);
        let coverer = crate::gpu::block_on(Coverer::new(&gpu)).unwrap();
        for n in [0, 7, 19] {
            let (w, h) = (inputs.masks[n].width, inputs.masks[n].height);
            let found = crate::gpu::block_on(coverer.cover(&list, &inputs.cameras[n], w, h)).unwrap();
            let expected = cover_reference(&points, &inputs.cameras[n], w, h);
            let different = found.data.iter().zip(&expected.data).filter(|(a, b)| a != b).count();
            let covered = expected.data.iter().filter(|&&v| v != 0).count();
            assert!(covered > 1000 && different <= 2, "view {n}: {different} of {covered} pixels differ");
            // The hull of a clean scene covers its mask.
            let missed = inputs.masks[n].data.iter().zip(&found.data).filter(|(&m, &c)| m != 0 && c == 0).count();
            assert_eq!(missed, 0);
        }
        // The orbit normal of the ring is the z axis, pointing from the cameras (z > 0) to the object.
        let (middle, down) = orbit_down(&inputs.cameras, &state.hull);
        assert!(down[2] < -0.999 && middle[2] > 1.0, "{middle:?} {down:?}");
        // Nothing is darker than the object's own range outside the masks, so nothing is added.
        let before = inputs.masks.clone();
        let round = crate::gpu::block_on(repair_masks(&gpu, &mut inputs, &state, &config)).unwrap();
        assert_eq!(round.added_fraction_maximum, 0.0);
        assert_eq!(inputs.masks, before);
    }
}
