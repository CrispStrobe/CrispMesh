//! The scalar field whose zero level is the surface (port of `tsdf_hull_mesh.field`).
//!
//! Arithmetic follows the reference operation by operation, including where NumPy works in
//! `float32` and where in `float64`, so that both produce the same field up to the rounding
//! of the Gaussian sums.

use std::path::Path;

use anyhow::{bail, ensure, Result};
use rayon::prelude::*;

use super::edt;
use super::gaussian::gaussian_filter;
use super::grid::{count, Dims};
use crate::config::DenseConfig;
use crate::npz::{Array, Data, Npz};

/// Voxels of padding around the hull grid on every side.
pub const PAD: usize = 3;

/// The support plane the object stood on, as `multiscale_stereo` stores it.
pub struct Support {
    pub point: Array,
    pub down: Vec<f64>,
    pub height: f64,
}

/// Content of `volume.npz`: fused evidence at the voxels of the silhouette hull.
pub struct Volume {
    pub shape: Dims,
    /// Voxel indices (x, y, z) of the hull voxels.
    pub index: Vec<[u16; 3]>,
    pub total: Vec<f32>,
    pub weight: Vec<f32>,
    pub origin: Array,
    pub voxel: f64,
    /// Truncation distance in scene units.
    pub truncation: f64,
    pub support: Option<Support>,
    /// The views do not go around the object; see `fusion::partial_views`.
    pub partial_views: bool,
}

/// Preview volumes handed from the stereo stage to the preview mesher of the same run, by the path
/// their `preview_volume` event names; nothing is written to that path.
static HANDED_OVER: std::sync::Mutex<Vec<(std::path::PathBuf, Volume)>> = std::sync::Mutex::new(Vec::new());

impl Volume {
    /// Leaves a volume for [`Volume::take_handed_over`] under `path`.
    pub fn hand_over(path: &Path, volume: Volume) {
        let mut handed = HANDED_OVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        handed.retain(|(p, _)| p != path);
        handed.push((path.to_path_buf(), volume));
    }

    /// The volume left under `path`, once.
    pub fn take_handed_over(path: &Path) -> Option<Volume> {
        let mut handed = HANDED_OVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let at = handed.iter().position(|(p, _)| p == path)?;
        Some(handed.swap_remove(at).1)
    }

    /// The volume on a grid `factor` times coarser: each block of `factor`^3 voxels becomes one, in the
    /// hull when any of its voxels is, with the sums of their signed distances and weights (so the mean
    /// signed distance is the weighted mean of the block). The surface stage needs about `factor`^3
    /// times less memory on it; used for the coarse live previews.
    pub fn coarsened(self, factor: usize) -> Volume {
        if factor <= 1 {
            return self;
        }
        let shape = self.shape.map(|s| s.div_ceil(factor));
        let mut blocks: std::collections::HashMap<[u16; 3], (f32, f32)> = std::collections::HashMap::with_capacity(self.index.len() / 4);
        for ((voxel, &total), &weight) in self.index.iter().zip(&self.total).zip(&self.weight) {
            let block = voxel.map(|v| (usize::from(v) / factor) as u16);
            let entry = blocks.entry(block).or_insert((0.0, 0.0));
            entry.0 += total;
            entry.1 += weight;
        }
        let mut entries: Vec<([u16; 3], (f32, f32))> = blocks.into_iter().collect();
        entries.sort_unstable_by_key(|(block, _)| *block);
        Volume {
            shape,
            index: entries.iter().map(|(block, _)| *block).collect(),
            total: entries.iter().map(|(_, (total, _))| *total).collect(),
            weight: entries.iter().map(|(_, (_, weight))| *weight).collect(),
            origin: self.origin,
            voxel: self.voxel * factor as f64,
            truncation: self.truncation.max(self.voxel * factor as f64),
            support: self.support,
            partial_views: self.partial_views,
        }
    }

    /// Drops every volume left and not taken (a run that ends early).
    pub fn drop_handed_over() {
        HANDED_OVER.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    }

    /// A volume as `write_volume` followed by [`Volume::read`] would give it.
    pub fn from_parts(
        hull: &crate::hull::Hull,
        indices: &[u32],
        total: Vec<f32>,
        weight: Vec<f32>,
        truncation: f64,
        support: Option<&crate::fusion::Support>,
    ) -> Self {
        let support = support.map(|s| Support {
            point: Array::new(&[3], Data::F32(s.point.to_vec())),
            down: s.down.iter().map(|&v| f64::from(v)).collect(),
            height: f64::from(s.height.unwrap_or(f32::NAN)),
        });
        Volume {
            shape: hull.shape,
            index: indices.iter().map(|&linear| hull.unravel(linear).map(|v| v as u16)).collect(),
            total,
            weight,
            origin: Array::new(&[3], Data::F32(hull.origin.to_vec())),
            voxel: f64::from(hull.voxel as f32),
            truncation: f64::from(truncation as f32),
            support,
            partial_views: false,
        }
    }

    /// The volume of a fusion result, exactly as writing it to `volume.npz` and reading it back would
    /// give it (the same single-precision origin, voxel size, truncation and support).
    pub fn from_fused(hull: &crate::hull::Hull, fused: crate::fusion::Fused) -> Self {
        let index = fused.indices.iter().map(|&linear| hull.unravel(linear).map(|v| v as u16)).collect();
        let support = &fused.support;
        let support = Some(Support {
            point: Array::new(&[3], Data::F32(support.point.to_vec())),
            down: support.down.iter().map(|&v| f64::from(v)).collect(),
            height: f64::from(support.height.unwrap_or(f32::NAN)),
        });
        Volume {
            shape: hull.shape,
            index,
            total: fused.total,
            weight: fused.weight,
            origin: Array::new(&[3], Data::F32(hull.origin.to_vec())),
            voxel: f64::from(hull.voxel as f32),
            truncation: f64::from(fused.truncation as f32),
            support,
            partial_views: fused.partial_views,
        }
    }

    pub fn read(path: &Path) -> Result<Self> {
        let mut npz = Npz::read(path)?;
        let shape = npz.get("shape")?.to_f64();
        ensure!(
            shape.len() == 3 && shape.iter().all(|&s| (1.0..=60000.0).contains(&s) && s.fract() == 0.0),
            "volume shape must be three positive integers"
        );
        let shape = [shape[0] as usize, shape[1] as usize, shape[2] as usize];
        // The large arrays are taken out of the archive one at a time, to hold each only once.
        let index = {
            let rows = npz.take("index")?;
            ensure!(rows.shape.len() == 2 && rows.shape[1] == 3, "volume index must be N x 3, found {:?}", rows.shape);
            let inside = |voxel: [i64; 3]| -> Result<[u16; 3]> {
                ensure!(
                    (0..3).all(|a| voxel[a] >= 0 && voxel[a] < shape[a] as i64),
                    "hull voxel {voxel:?} lies outside the volume shape {shape:?}"
                );
                Ok(voxel.map(|v| v as u16))
            };
            match &rows.data {
                Data::I32(values) => values.as_chunks::<3>().0.iter().map(|v| inside(v.map(i64::from))).collect::<Result<Vec<_>>>()?,
                Data::I64(values) => values.as_chunks::<3>().0.iter().map(|v| inside(*v)).collect::<Result<Vec<_>>>()?,
                _ => bail!("volume index must be an integer array, found {}", rows.descr()),
            }
        };
        let total = npz.take("total")?.into_f32();
        let weight = npz.take("weight")?.into_f32();
        ensure!(total.len() == index.len() && weight.len() == index.len(), "total and weight must have one entry per index row");
        let origin = npz.get("origin")?.clone();
        ensure!(origin.len() == 3, "volume origin must have three entries");
        let support = if npz.contains("support_height") {
            let (point, down) = (npz.get("support_point")?.clone(), npz.get("support_down")?.to_f64());
            ensure!(point.len() == 3 && down.len() == 3, "support point and direction must have three entries");
            Some(Support { point, down, height: npz.get("support_height")?.scalar()? })
        } else {
            None
        };
        Ok(Volume {
            shape,
            index,
            total,
            weight,
            origin,
            voxel: npz.get("voxel")?.scalar()?,
            truncation: npz.get("truncation")?.scalar()?,
            support,
            partial_views: npz.contains("partial_views") && npz.get("partial_views")?.scalar()? > 0.5,
        })
    }
}

/// What `field` reports besides the field itself.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldReport {
    pub flat_base_applied: bool,
    pub hull_fraction_below_support: Option<f64>,
    pub observed_hull_fraction: f64,
    pub extrapolated_hull_fraction: f64,
    /// How far unmeasured surface was moved inside the silhouette hull (`mesh_hull_overshoot`), voxels.
    pub hull_overshoot_voxels: f64,
}

pub struct Field {
    /// Padded grid, negative inside.
    pub value: Vec<f32>,
    /// With views that do not go around the object (`mesh_open_unseen`): the padded voxels with
    /// measured or extrapolated evidence; surface elsewhere is left out of the mesh.
    pub known: Option<Vec<bool>>,
    pub dims: Dims,
    pub report: FieldReport,
}

pub fn field(volume: &Volume, config: &DenseConfig, check: &dyn Fn() -> Result<()>) -> Result<Field> {
    let dims = volume.shape.map(|s| s + 2 * PAD);
    let total_voxels = count(dims);
    ensure!(total_voxels < u32::MAX as usize, "volume grid is too large");
    let truncation = volume.truncation / volume.voxel;
    ensure!(truncation.is_finite() && truncation > 0.0, "volume truncation and voxel size must be positive");
    let hull_count = volume.index.len();
    ensure!(hull_count > 0, "the volume has no hull voxels");
    ensure!(volume.total.len() == hull_count && volume.weight.len() == hull_count, "total and weight must have one entry per hull voxel");
    let mut place = Vec::with_capacity(hull_count);
    for voxel in &volume.index {
        let [x, y, z] = voxel.map(|v| usize::from(v) + PAD);
        ensure!(
            x < dims[0] - PAD && y < dims[1] - PAD && z < dims[2] - PAD,
            "hull voxel {voxel:?} lies outside the volume shape {:?}",
            volume.shape
        );
        place.push(((x * dims[1] + y) * dims[2] + z) as u32);
    }

    // Silhouette prior: signed distance to the hull boundary in truncation units, positive
    // outside. A voxel centre is half a voxel from the boundary between it and its neighbour.
    // The two distance transforms run one after the other so that only one is in memory.
    let mut hull = vec![false; total_voxels];
    for &p in &place {
        hull[p as usize] = true;
    }
    let prior = |signed: f64| ((signed + if signed > 0.0 { -0.5 } else { 0.5 }) / truncation).clamp(-1.0, 1.0) as f32;
    let mut value = vec![0f32; total_voxels];
    for inside in [false, true] {
        check()?;
        let squared = edt::squared_distance(|i| hull[i] != inside, dims);
        value.par_iter_mut().zip(&squared).zip(&hull).for_each(|((v, &d), &h)| {
            if h == inside {
                let distance = f64::from(d).sqrt();
                *v = prior(if inside { -distance } else { distance });
            }
        });
    }
    drop(hull);

    // How far the silhouette hull stands outside the measured surface: the median depth below the
    // hull boundary of the measured zero-crossings near it. The hull is wider than the object by the mask
    // tolerance and by what the ring of views cannot carve; where nothing is measured, the surface
    // is placed that far inside the hull instead of on it.
    let overshoot = if config.mesh_hull_overshoot {
        let reach = (truncation - 0.5) as f32;
        let mut depths: Vec<f32> = place
            .iter()
            .enumerate()
            .filter(|&(i, _)| volume.weight[i] >= 3.0 && (volume.total[i] / volume.weight[i]).abs() < 0.5)
            // Depth of the zero-crossing itself: the voxel's depth plus its signed distance to the surface.
            .map(|(i, &p)| -value[p as usize] * truncation as f32 - 0.5 + volume.total[i] / volume.weight[i] * truncation as f32)
            .filter(|&d| d >= 0.0 && d < reach)
            .collect();
        if depths.len() >= 100 {
            let middle = depths.len() / 2;
            *depths.select_nth_unstable_by(middle, f32::total_cmp).1
        } else {
            0.0
        }
    } else {
        0.0
    };

    // The silhouette prior of every hull voxel, for the overshoot correction below.
    let priors: Vec<f32> = if overshoot > 0.0 { place.iter().map(|&p| value[p as usize]).collect() } else { Vec::new() };

    // Averaged signed distance times a confidence that saturates at a few views, and that
    // confidence, at the observed hull voxels (zero elsewhere).
    let minimum = config.mesh_minimum_weight as f32;
    let cap = config.mesh_confidence_cap as f32;
    let observed: Vec<bool> = volume.weight.iter().map(|&w| w >= minimum).collect();
    let confidence: Vec<f32> = volume.weight.iter().zip(&observed).map(|(&w, &o)| if o { w.min(cap) / cap } else { 0.0 }).collect();
    let weighted = |i: usize| if observed[i] { volume.total[i] / volume.weight[i].max(1e-6) * confidence[i] } else { 0.0 };

    // Confidence-weighted smoothing where observed, then bounded extrapolation into nearby
    // unobserved hull voxels; what stays unfilled keeps the prior. A filled voxel never drops
    // below the prior (`max(value, prior)` in the reference; each voxel is filled at most
    // once, so `value` still holds its prior when it is filled).
    // Silhouette first (`mesh_hull_correction`): the hull is the surface, and depth only moves it inward, by the
    // confidence-weighted offset between the measured distance and the prior averaged over a neighbourhood,
    // less `mesh_hull_correction_gate` times the local scatter of those offsets. Averaging removes the depth
    // noise that the maximum below would turn into pits; the scatter term leaves the hull alone where the
    // depths disagree among themselves. Offsets count where both distances are inside the truncation band.
    let mut corrected: Option<Vec<bool>> = None;
    if config.mesh_hull_correction > 0.0 {
        let (band, gate) = (0.9f32, config.mesh_hull_correction_gate as f32);
        let mut offset = vec![0f32; total_voxels];
        let mut support = vec![0f32; total_voxels];
        let mut square = vec![0f32; if gate > 0.0 { total_voxels } else { 0 }];
        for (i, &p) in place.iter().enumerate() {
            if observed[i] {
                let measured = volume.total[i] / volume.weight[i].max(1e-6);
                let prior = value[p as usize];
                if measured.abs() < band && prior.abs() < band {
                    let difference = measured - prior;
                    offset[p as usize] = difference * confidence[i];
                    support[p as usize] = confidence[i];
                    if gate > 0.0 {
                        square[p as usize] = difference * difference * confidence[i];
                    }
                }
            }
        }
        check()?;
        gaussian_filter(&mut offset, dims, config.mesh_hull_correction);
        gaussian_filter(&mut support, dims, config.mesh_hull_correction);
        if gate > 0.0 {
            check()?;
            gaussian_filter(&mut square, dims, config.mesh_hull_correction);
        }
        let mut reached = vec![false; hull_count];
        for (i, &p) in place.iter().enumerate() {
            let p = p as usize;
            if support[p] > 0.02 {
                let mean = offset[p] / support[p];
                let scatter = if gate > 0.0 { (square[p] / support[p] - mean * mean).max(0.0).sqrt() } else { 0.0 };
                value[p] = (value[p] + (mean - gate * scatter).max(0.0)).clamp(-1.0, 1.0);
                reached[i] = true;
            }
        }
        corrected = Some(reached);
    }
    let mut filled = corrected.clone().unwrap_or_else(|| vec![false; hull_count]);
    let mut work = vec![0f32; total_voxels];
    let mut numerator = vec![0f32; hull_count];
    for &sigma in std::iter::once(&config.mesh_smooth).chain(&config.mesh_fill_sigmas) {
        if corrected.is_some() {
            break;
        }
        work.fill(0.0);
        for (i, &p) in place.iter().enumerate() {
            work[p as usize] = weighted(i);
        }
        check()?;
        gaussian_filter(&mut work, dims, sigma);
        for (slot, &p) in numerator.iter_mut().zip(&place) {
            *slot = work[p as usize];
        }
        work.fill(0.0);
        for (&c, &p) in confidence.iter().zip(&place) {
            work[p as usize] = c;
        }
        check()?;
        gaussian_filter(&mut work, dims, sigma);
        for (i, &p) in place.iter().enumerate() {
            let denominator = work[p as usize];
            if !filled[i] && denominator > 0.02 {
                let prior = value[p as usize];
                value[p as usize] = (numerator[i] / denominator).max(prior);
                filled[i] = true;
            }
        }
    }
    let mut known = (volume.partial_views && config.mesh_open_unseen).then(|| {
        let mut known = vec![false; total_voxels];
        for (i, &p) in place.iter().enumerate() {
            known[p as usize] = observed[i] || filled[i];
        }
        known
    });
    drop((confidence, work, numerator, place));
    if overshoot > 0.0 {
        // Unmeasured voxels (extrapolated or not) take at least the prior moved inward by the overshoot.
        let shift = overshoot / truncation as f32;
        for (i, voxel) in volume.index.iter().enumerate() {
            if !observed[i] {
                let [x, y, z] = voxel.map(|v| usize::from(v) + PAD);
                let at = (x * dims[1] + y) * dims[2] + z;
                value[at] = value[at].max((priors[i] + shift).clamp(-1.0, 1.0));
            }
        }
    }

    let mut report = FieldReport {
        flat_base_applied: false,
        hull_fraction_below_support: None,
        observed_hull_fraction: observed.iter().filter(|&&o| o).count() as f64 / hull_count as f64,
        extrapolated_hull_fraction: filled.iter().zip(&observed).filter(|(&f, &o)| f && !o).count() as f64 / hull_count as f64,
        hull_overshoot_voxels: f64::from(overshoot),
    };

    if let Some(support) = volume.support.as_ref().filter(|s| config.mesh_flat_base && s.height.is_finite()) {
        // Signed distance to the support plane in truncation units, positive below it.
        let voxel = volume.voxel;
        let down = &support.down;
        let difference: Vec<f64> = if volume.origin.is_f32() && support.point.is_f32() {
            volume.origin.to_f32().iter().zip(support.point.to_f32()).map(|(&o, p)| f64::from(o - p)).collect()
        } else {
            volume.origin.to_f64().iter().zip(support.point.to_f64()).map(|(&o, p)| o - p).collect()
        };
        let offset = (difference[0] * down[0] + difference[1] * down[1] + difference[2] * down[2] - support.height) as f32;
        let axes: Vec<Vec<f32>> =
            (0..3).map(|a| (0..dims[a]).map(|i| ((i as f64 - PAD as f64 + 0.5) * voxel * down[a]) as f32).collect()).collect();
        let (voxel, margin, truncation) = (voxel as f32, config.mesh_base_margin as f32, truncation as f32);
        let plane = |x: usize, y: usize, z: usize| ((axes[0][x] + axes[1][y] + axes[2][z] + offset) / voxel - margin) / truncation;
        let plane = |x: usize, y: usize, z: usize| plane(x, y, z).clamp(-1.0, 1.0);
        let below =
            volume.index.iter().filter(|v| plane(usize::from(v[0]) + PAD, usize::from(v[1]) + PAD, usize::from(v[2]) + PAD) > 0.0).count();
        report.flat_base_applied = true;
        report.hull_fraction_below_support = Some(below as f64 / hull_count as f64);
        value.par_chunks_mut(dims[1] * dims[2]).enumerate().for_each(|(x, slab)| {
            for (y, row) in slab.chunks_exact_mut(dims[2]).enumerate() {
                for (z, v) in row.iter_mut().enumerate() {
                    *v = v.max(plane(x, y, z));
                }
            }
        });
        if let Some(known) = known.as_mut() {
            // The flat base is inferred from the support, not unseen hull: it stays in the mesh.
            known.par_chunks_mut(dims[1] * dims[2]).enumerate().for_each(|(x, slab)| {
                for (y, row) in slab.chunks_exact_mut(dims[2]).enumerate() {
                    for (z, k) in row.iter_mut().enumerate() {
                        *k |= plane(x, y, z) > -1.0;
                    }
                }
            });
        }
    }

    // The maximum leaves creases that would give ambiguous cells.
    if config.mesh_final_smooth != 0.0 {
        check()?;
        gaussian_filter(&mut value, dims, config.mesh_final_smooth);
    }
    Ok(Field { value, known, dims, report })
}

#[cfg(test)]
mod correction_tests {
    use super::*;

    /// A hull box whose +x face stands two voxels outside the measured surface.
    fn volume(noise: f32) -> Volume {
        let mut index = Vec::new();
        let (mut total, mut weight) = (Vec::new(), Vec::new());
        for x in 6u16..18 {
            for y in 4u16..20 {
                for z in 4u16..20 {
                    index.push([x, y, z]);
                    // Measured near the +x face only, away from the other faces.
                    let seen = x >= 12 && (8..16).contains(&y) && (8..16).contains(&z);
                    let sign = if (x + y + z) % 2 == 0 { 1.0 } else { -1.0 };
                    let distance = ((f32::from(x) + 0.5 - 16.0) / 3.0 + noise * sign).clamp(-1.0, 1.0);
                    weight.push(if seen { 4.0 } else { 0.0 });
                    total.push(if seen { 4.0 * distance } else { 0.0 });
                }
            }
        }
        Volume {
            shape: [24, 24, 24],
            index,
            total,
            weight,
            origin: Array::new(&[3], Data::F32(vec![0.0; 3])),
            voxel: 1.0,
            truncation: 3.0,
            support: None,
            partial_views: false,
        }
    }

    fn at(field: &Field, x: usize) -> f32 {
        field.value[((x + PAD) * field.dims[1] + 12 + PAD) * field.dims[2] + 12 + PAD]
    }

    /// The correction moves the hull face inward to the measured surface when the depths agree, and
    /// leaves the hull where they scatter by more than they shift it.
    #[test]
    fn hull_correction_follows_agreeing_depths_and_ignores_scattered_ones() {
        let mut config = DenseConfig { mesh_hull_overshoot: false, mesh_final_smooth: 0.0, ..DenseConfig::default() };
        config.mesh_hull_correction = 2.0;
        let clean = field(&volume(0.0), &config, &|| Ok(())).unwrap();
        // Hull boundary at x = 18, surface at x = 16: the voxel centred at 16.5 is outside, the one at 15.5 inside.
        assert!(at(&clean, 16) > 0.0 && at(&clean, 15) < 0.0, "{} {}", at(&clean, 16), at(&clean, 15));
        let scattered = field(&volume(1.0), &config, &|| Ok(())).unwrap();
        assert!(at(&scattered, 16) < 0.0 && at(&scattered, 17) < 0.0, "{} {}", at(&scattered, 16), at(&scattered, 17));
    }
}
