//! The coarse-to-fine level loop of `run` in `multiscale_stereo.py`: a full
//! sweep at the coarsest level, band refinements around the upsampled depth at
//! finer ones, the hull-front candidate for thin parts, cross-view agreement
//! after every pass, and the fallback merge at the end.

use std::path::Path;
use web_time::Instant;

use serde_json::{json, Value};

use crate::config::DenseConfig;
use crate::control::Control;
use crate::events::EventLog;
use crate::fusion::tsdf;
use crate::gpu::Gpu;
use crate::hull::HullState;
use crate::inputs::{parallel_map, round_half_even, Inputs, Plane};

use super::depth::{consistent, hull_front, initial, merge_fallback};
use super::level::{build_level_sharpened, coverage, LevelView};
use super::matcher::Matcher;
use super::planes::{consistent_planes, depth_of, initial_planes, pack_normal, Planes, Settings as PlaneSettings};
use super::previews::depth_sheet;
use super::run::Previews;

/// Everything the loop needs from the driver.
#[derive(Clone, Copy)]
pub struct LevelContext<'a> {
    pub gpu: &'a Gpu,
    pub inputs: &'a Inputs,
    pub state: &'a HullState,
    pub config: &'a DenseConfig,
    pub output: &'a Path,
    pub events: &'a EventLog,
    pub previews: &'a Previews<'a>,
    pub picks: &'a [usize],
    pub control: &'a Control,
}

/// Runs every level in `sizes`. Returns the finest level and its final depth
/// maps; level rows and the fallback coverage are added to `report`.
pub async fn match_levels(
    context: &LevelContext<'_>,
    sizes: &[i64],
    report: &mut Value,
) -> anyhow::Result<(Vec<LevelView>, Vec<Plane<f32>>)> {
    let LevelContext { gpu, inputs, state, config, output, events, previews, picks, control } = *context;
    let count = inputs.count();
    let neighbours: Vec<Vec<usize>> =
        (0..count).map(|i| inputs.neighbours(i, config.neighbours as usize, config)).collect::<anyhow::Result<_>>()?;
    let voters: Vec<Vec<usize>> =
        (0..count).map(|i| inputs.neighbours(i, config.vote_neighbours as usize, config)).collect::<anyhow::Result<_>>()?;
    let mut matcher = Matcher::new(gpu, config, &state.search).await?;
    let mut plane_refiner: Option<Planes> = None;
    let mut level: Vec<LevelView> = Vec::new();
    let mut depths: Vec<Plane<f32>> = Vec::new();
    let mut coarser: Option<Vec<Plane<f32>>> = None;
    // Step of inverse depth per view. As in the reference, the step of the last
    // view swept at the coarsest level serves every view of the finer levels (its
    // loop variable is overwritten). CRISPMESH_OWN_STEP=1 gives each view the step
    // of its own depth range instead; under evaluation.
    let shared_step = !std::env::var("CRISPMESH_OWN_STEP").is_ok_and(|v| v == "1");
    let mut steps = vec![0.0f64; count];
    let mut rows: Vec<Value> = Vec::new();
    for (li, &size) in sizes.iter().enumerate() {
        let t = Instant::now();
        level = build_level_sharpened(inputs, size, config.match_sharpening);
        let buffers = matcher.upload(&level);
        let window = DenseConfig::level(&config.windows, li);
        let aggregate = DenseConfig::level(&config.aggregates, li);
        let passes = if li == 0 { 1 } else { DenseConfig::level(&config.passes, li) };
        let scale = 2f64.powi(li as i32);
        for p in 0..passes {
            let t_pass = Instant::now();
            let mut initial_seconds = 0.0;
            let mut raw: Vec<Plane<f32>> = Vec::with_capacity(count);
            let mut front_wins = 0usize;
            // Initial surfaces of a group of views at a time: CPU work in parallel, bounded memory.
            let mut inits: Vec<Plane<f32>> = Vec::new();
            let mut inits_from = 0usize;
            // One view in flight: view i's work is queued and its readback started before view i-1's
            // depth is collected and finished on the CPU, so the GPU computes while the CPU waits
            // for a readback, builds initial surfaces or tests the hull's front. Every view's work
            // depends only on the previous pass, so the results do not depend on this order.
            let mut in_flight: Option<(usize, super::matcher::Pending)> = None;
            for i in 0..=count {
                if i < count {
                    control.check()?;
                }
                let next = if i == count {
                    None
                } else if li == 0 {
                    let (pending, view_step) = matcher
                        .begin_sweep(&buffers, &level, i, &neighbours[i], state.bounds[i], window, aggregate, config.min_score)
                        .await?;
                    steps[i] = view_step;
                    if shared_step {
                        steps = vec![view_step; count];
                    }
                    Some(pending)
                } else {
                    if i >= inits_from + inits.len() {
                        let t_init = Instant::now();
                        inits_from = i;
                        let sigma = if p == 0 { 1.5 } else { 1.0 };
                        inits = parallel_map((count - i).min(init_group()), |n| initial(&depths[i + n], &level[i + n].mask, sigma));
                        initial_seconds += t_init.elapsed().as_secs_f64();
                    }
                    let fine = steps[i] / scale / if p == 0 { 1.0 } else { 2.0 };
                    let half = if p == 0 { DenseConfig::level(&config.band_first, li) } else { config.band_later };
                    Some(
                        matcher
                            .begin_refine(
                                &buffers,
                                &level,
                                i,
                                &neighbours[i],
                                &inits[i - inits_from],
                                fine,
                                half,
                                window,
                                aggregate,
                                config.min_score,
                            )
                            .await?,
                    )
                };
                let previous = in_flight.take();
                if let Some(pending) = next {
                    in_flight = Some((i, pending));
                }
                let Some((v, pending)) = previous else { continue };
                let mut d = matcher.finish(pending).await?;
                if config.hull_front && li == (config.hull_front_level as usize).min(sizes.len() - 1) && p == 0 {
                    // Thin parts are narrower than the coarse window, so they inherit the
                    // depth of whatever lies behind them. Test the hull's front surface
                    // too: a photo-consistent nearer surface occludes anything matched behind it.
                    let front = hull_front(&level[v], &state.hull, state.bounds[v], config.hull_front_stride as usize);
                    let half = DenseConfig::level(&config.band_first, li);
                    let minimum = config.min_score.max(config.hull_front_min_score);
                    let d2 = matcher
                        .refine(&buffers, &level, v, &neighbours[v], &front, steps[v] / scale, half, window, aggregate, minimum)
                        .await?;
                    let margin = (1.0 - config.hull_front_margin) as f32;
                    for (value, &candidate) in d.data.iter_mut().zip(&d2.data) {
                        if candidate > 0.0 && (*value == 0.0 || candidate < *value * margin) {
                            *value = candidate;
                            front_wins += 1;
                        }
                    }
                }
                raw.push(d);
                if v % 10 == 9 {
                    let done = (li as f64 + (p as f64 + (v + 1) as f64 / count as f64) / passes as f64) / sizes.len() as f64;
                    events.progress(
                        0.08 + 0.84 * done,
                        &format!("Matching level {} of {}, view {} of {}", li + 1, sizes.len(), v + 1, count),
                    )?;
                }
            }
            let mut refinement_seconds = 0.0;
            let mut normals: Vec<Vec<u32>> = Vec::new();
            if (config.slanted_refine || config.patchmatch) && li + 1 == sizes.len() && p + 1 == passes && li > 0 {
                // Slanted planes per pixel, from the band refinement where it found a depth and from
                // the smooth surface of the previous pass or level elsewhere.
                let t_pm = Instant::now();
                if plane_refiner.is_none() {
                    let refiner = if config.patchmatch {
                        #[cfg(any(test, feature = "research-patchmatch"))]
                        {
                            Planes::new(gpu, &state.search).await?
                        }
                        #[cfg(not(any(test, feature = "research-patchmatch")))]
                        {
                            anyhow::bail!("patchmatch is excluded from this build pending patent review");
                        }
                    } else {
                        Planes::independent(gpu, &state.search).await?
                    };
                    plane_refiner = Some(refiner);
                }
                let pm = plane_refiner.as_ref().expect("created");
                let half = if p == 0 { DenseConfig::level(&config.band_first, li) } else { config.band_later };
                for v in 0..count {
                    control.check()?;
                    let smooth = initial(&depths[v], &level[v].mask, 1.0);
                    let planes = initial_planes(&level[v], &raw[v], &smooth);
                    let fine = steps[v] / scale / if p == 0 { 1.0 } else { 2.0 };
                    let (near, far) = state.bounds[v];
                    let settings = PlaneSettings {
                        radius: (window / 2) as u32,
                        stride: 2,
                        best_of: config.best_of as u32,
                        min_variance: config.min_variance as f32,
                        window_fill: config.window_fill as f32,
                        iterations: if config.patchmatch { config.patchmatch_iterations } else { config.slanted_iterations } as u32,
                        inverse_step: (half as f64 * fine) as f32,
                        normal_step: 0.5,
                        inverse_range: ((1.0 / far) as f32, (1.0 / near) as f32),
                        seed: v as u32,
                    };
                    let (planes, scores) = pm.run(gpu, &level, v, &neighbours[v], &state.search, &planes, &settings).await?;
                    raw[v] = depth_of(&planes, &scores, level[v].width, level[v].height, config.min_score as f32);
                    normals.push(
                        planes.iter().zip(&raw[v].data).map(|(q, &d)| if d > 0.0 { pack_normal([q[1], q[2], q[3]]) } else { 0 }).collect(),
                    );
                    if v % 10 == 9 || v + 1 == count {
                        control.log(format!(
                            "{}: {} of {count} views, {:.1}s",
                            if config.patchmatch { "patchmatch" } else { "independent planes" },
                            v + 1,
                            t_pm.elapsed().as_secs_f64()
                        ));
                    }
                }
                refinement_seconds = t_pm.elapsed().as_secs_f64();
            }
            let t_c = Instant::now();
            let (tolerance, votes) = (DenseConfig::level(&config.tolerances, li), DenseConfig::level(&config.min_votes, li));
            let agreed = if !normals.is_empty() {
                consistent_planes(
                    &level,
                    &raw,
                    &normals,
                    &voters,
                    tolerance,
                    votes,
                    if config.patchmatch { config.patchmatch_normal_agreement as f32 } else { 0.8 },
                )
            } else {
                consistent(&level, &raw, &voters, tolerance, votes)
            };
            drop(normals);
            if li + 1 == sizes.len() && std::env::var_os("CRISPMESH_STAGE_DEPTHS").is_some() {
                // Opt-in diagnostics distinguish matching from cross-view rejection.
                // These are the pipeline's own depths, before any coarse fallback.
                super::run::write_depths(&output.join(format!("depths-pass-{p}-raw.npz")), &raw)?;
                super::run::write_depths(&output.join(format!("depths-pass-{p}-consistent.npz")), &agreed)?;
                let cameras: Vec<serde_json::Value> =
                    level.iter().map(|v| json!({"width": v.width, "height": v.height, "k": v.camera.k})).collect();
                crate::storage::write(
                    output.join("depths-stage-cameras.json"),
                    serde_json::to_string(&json!({"views": cameras, "boxes": inputs.boxes}))?,
                )?;
            }
            let row = json!({
                "size": size, "pass": p, "window": window, "working_size": [level[0].width, level[0].height],
                "raw_coverage": coverage(&level, &raw), "consistent_coverage": coverage(&level, &agreed),
                "hull_front_pixels": front_wins, "consistency_seconds": t_c.elapsed().as_secs_f64(),
                "initial_seconds": initial_seconds, "patchmatch_seconds": if config.patchmatch { refinement_seconds } else { 0.0 },
                "slanted_seconds": if config.slanted_refine { refinement_seconds } else { 0.0 }, "pass_seconds": t_pass.elapsed().as_secs_f64(),
                "seconds": t.elapsed().as_secs_f64(),
            });
            control.log(format!(
                "level {li} pass {p}: size {size}, coverage {:.3}, {:.1}s",
                row["consistent_coverage"]["median"].as_f64().unwrap_or(0.0),
                row["seconds"].as_f64().unwrap_or(0.0)
            ));
            rows.push(row);
            depths = agreed;
        }
        if std::env::var("CRISPMESH_LEVEL_DEPTHS").is_ok() {
            // Diagnostic: every level's consistent depth, with the level's canvas geometry.
            super::run::write_depths(&output.join(format!("depths-level-{li}.npz")), &depths)?;
            let cameras: Vec<serde_json::Value> =
                level.iter().map(|v| json!({"width": v.width, "height": v.height, "k": v.camera.k})).collect();
            crate::storage::write(
                output.join(format!("depths-level-{li}.json")),
                serde_json::to_string(&json!({"views": cameras, "boxes": inputs.boxes}))?,
            )?;
        }
        let sheet = output.join(format!("depth-level-{li}.png"));
        depth_sheet(&sheet, &level, &depths, picks)?;
        events.artifact("depth_sheet", &sheet, &format!("Depth at level {} (photo, depth, shading)", li + 1), json!({"level": li}))?;
        events.metric(&format!("coverage_level_{li}"), rows.last().unwrap()["consistent_coverage"]["median"].as_f64().unwrap_or(0.0))?;
        if previews.enabled() && li + 1 < sizes.len() {
            let rim = round_half_even(config.rim_fraction * window as f64) as usize;
            let fused = tsdf(gpu, &state.hull, &inputs.cameras, &level, &depths, rim, config).await?;
            previews.write(
                &format!("1{li}-level-{li}"),
                &format!("Surface after level {}", li + 1),
                &state.hull,
                &fused.indices,
                &fused.total,
                &fused.weight,
                Some(&fused.support),
                json!({"level": li}),
            )?;
        }
        if config.fallback_level && li + 1 == sizes.len() {
            if let Some(coarse) = &coarser {
                // Keep the finest depth; where it failed its checks, fall back to the
                // previous level's consistent depth instead of leaving a hole.
                if config.fallback_supported {
                    // Native-resolution pyramids may stop before the configured
                    // finest level. Gap fills still require its final configured
                    // policy, rather than the earlier level's acceptance threshold.
                    let tolerance = *config.tolerances.last().expect("validated nonempty tolerances");
                    let minimum = *config.min_votes.last().expect("validated nonempty min_votes");
                    let results = parallel_map(count, |i| {
                        let merged = merge_fallback(&level[i], &depths[i], &coarse[i]);
                        let candidates = merged.data.iter().zip(&depths[i].data).filter(|(d, f)| **d > 0.0 && **f <= 0.0).count();
                        let supported = super::depth::supported_fallback(i, &level, &depths, merged, &voters[i], tolerance, minimum);
                        let accepted = supported.data.iter().zip(&depths[i].data).filter(|(d, f)| **d > 0.0 && **f <= 0.0).count();
                        (supported, candidates, accepted)
                    });
                    report["fallback_support"] = json!({
                        "relative_tolerance": tolerance, "minimum_votes": minimum,
                        "candidates": results.iter().map(|r| r.1).sum::<usize>(),
                        "accepted": results.iter().map(|r| r.2).sum::<usize>(),
                        "fine_pixels_unchanged": true,
                    });
                    depths = results.into_iter().map(|r| r.0).collect();
                } else {
                    depths = parallel_map(count, |i| merge_fallback(&level[i], &depths[i], &coarse[i]));
                }
                report["fallback_coverage"] = coverage(&level, &depths);
                let sheet = output.join("depth-merged.png");
                depth_sheet(&sheet, &level, &depths, picks)?;
                events.artifact("depth_sheet", &sheet, "Final depth with level fallback", json!({"level": li}))?;
            }
        }
        coarser = Some(depths.clone());
    }
    report["levels"] = json!(rows);
    report["gpu_peak_binding_bytes"] = json!(gpu.peak_binding());
    Ok((level, depths))
}

/// Views whose initial surfaces are computed together. With threads, a group is spread over them;
/// without (a single-threaded browser) one view at a time, so that each initial surface is
/// computed while the GPU works on the previous view.
fn init_group() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        #[cfg(target_feature = "atomics")]
        if rayon::current_num_threads() > 1 {
            return 16;
        }
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    16
}
