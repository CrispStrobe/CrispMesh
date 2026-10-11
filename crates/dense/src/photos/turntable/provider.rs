//! The `turntable` camera provider inside the `photos` command.

use crate::photos::fs::Stored as _;
use anyhow::{anyhow, bail};
use image::imageops::FilterType;
use serde_json::{json, Value};

use crate::photos::cameras::CameraProvider;
use crate::photos::markers::linalg::Camera;
use crate::photos::options::Options;
use crate::photos::providers::{Provider, CAMERAS_TURNTABLE};
use crate::photos::run::Run;
use crate::photos::solution::{Lens, Solution};
use crate::photos::staging::capture_name;
use crate::photos::util;

use super::solver::{self, Matches, PairMatches};
use super::{features, matching};

pub struct Turntable;

/// A feature off the object that moves less than this between two photos is the background.
const STILL_PIXELS: f64 = 2.0;

fn in_mask(mask: &crate::inputs::Plane<u8>, p: [f64; 2]) -> bool {
    let (x, y) = (p[0].floor(), p[1].floor());
    x >= 0.0 && y >= 0.0 && (x as usize) < mask.width && (y as usize) < mask.height && mask.data[y as usize * mask.width + x as usize] > 127
}

/// Search the mask and a surrounding band one bounding-box side wide. Static
/// off-object features are removed by their lack of motion when matching.
fn turntable_region(mask: &crate::inputs::Plane<u8>) -> crate::inputs::Plane<u8> {
    let (w, h) = (mask.width, mask.height);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if mask.data[y * w + x] > 127 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    let mut region = mask.clone();
    if x1 <= x0 {
        return region;
    }
    let reach = (x1 - x0).max(y1 - y0);
    let (left, right, bottom) = (x0.saturating_sub(reach), (x1 + reach).min(w), (y1 + reach).min(h));
    for y in y0.saturating_sub(reach)..bottom {
        for x in left..right {
            region.data[y * w + x] = 255;
        }
    }
    region
}

/// The declared lens at the working resolution: the photo resolution when its longest side is at most
/// `longest` (or `longest` is 0), else scaled to that longest side (centre of the top-left pixel at 0).
pub fn working_lens(lens: &Lens, longest: usize) -> Lens {
    let side = lens.width.max(lens.height) as usize;
    if longest == 0 || side <= longest {
        return *lens;
    }
    let factor = longest as f64 / side as f64;
    let (width, height) = ((lens.width as f64 * factor).round().max(1.0) as u32, (lens.height as f64 * factor).round().max(1.0) as u32);
    let (sx, sy) = (width as f64 / lens.width as f64, height as f64 / lens.height as f64);
    let [fx, fy, cx, cy] = lens.pixels;
    Lens { width, height, pixels: [fx * sx, fy * sy, (cx + 0.5) * sx - 0.5, (cy + 0.5) * sy - 0.5], k: lens.k }
}

/// Features that turn with the object: inside its mask, or matched within their own turn (where matches
/// of the still surroundings were already left out).
fn moving_features(within: &[PairMatches], inside: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let mut moving: Vec<Vec<bool>> = inside.to_vec();
    for (i, j, list) in within {
        for &(a, b) in list {
            moving[*i][a as usize] = true;
            moving[*j][b as usize] = true;
        }
    }
    moving
}

fn resized(plane: crate::inputs::Plane<u8>, size: (usize, usize), filter: FilterType) -> crate::inputs::Plane<u8> {
    if (plane.width, plane.height) == size {
        return plane;
    }
    let image = image::GrayImage::from_raw(plane.width as u32, plane.height as u32, plane.data).expect("plane size");
    let data = image::imageops::resize(&image, size.0 as u32, size.1 as u32, filter).into_raw();
    crate::inputs::Plane { width: size.0, height: size.1, data }
}

impl CameraProvider for Turntable {
    fn info(&self) -> &'static Provider {
        &CAMERAS_TURNTABLE
    }

    fn check(&self, options: &Options) -> anyhow::Result<()> {
        if options.turntable.surface && options.open_turn {
            bail!("surface region currently requires one complete turn");
        }
        if options.turntable.rings > 1 && options.open_turn {
            bail!("--turntable-rings: every turn must be complete (not with --open-turn)");
        }
        if let Some(path) = &options.turntable.matches {
            if !path.stored_file() {
                bail!("--turntable-matches {}: not found", path.display());
            }
        }
        Ok(())
    }

    fn recover(&self, run: &mut Run, lens: Option<&Lens>) -> anyhow::Result<Solution> {
        let lens = *lens.ok_or_else(|| anyhow!("the turntable provider needs the declared lens (--calibration)"))?;
        let (options, out) = (run.options, run.out);
        let (count, threads, timeouts) = (options.photo_count, options.threads, &options.timeouts);
        let rings = options.turntable.rings;
        if rings > 1 && (count % rings != 0 || count / rings < 8) {
            bail!("--turntable-rings {rings}: {count} photos do not make {rings} turns of equal length with at least 8 photos each");
        }
        let per = count / rings;
        // Features are found and the cameras solved at a working resolution, the same for every
        // capture whatever its pixel count: the solver's pixel bounds then mean the same everywhere.
        let working = working_lens(&lens, options.turntable.working_size);
        let (sx, sy) = (working.width as f64 / lens.width as f64, working.height as f64 / lens.height as f64);
        let shrink = |p: [f64; 2]| [(p[0] + 0.5) * sx - 0.5, (p[1] + 0.5) * sy - 0.5];
        let camera = Camera::from_lens(&working);
        let names: Vec<String> = (0..count).map(capture_name).collect();
        let ring_of = |photo: usize| photo / per;
        // Per photo and feature: inside the object's mask (only known for features found here).
        let mut object_side: Option<Vec<Vec<bool>>> = None;
        let mut matches = match &options.turntable.matches {
            // Debug input: features and matches made elsewhere (the prototype's `--export-matches`).
            Some(path) => {
                let mut loaded = Matches::from_json(&util::read_json(path)?)?;
                if loaded.keypoints.len() != count {
                    bail!("--turntable-matches lists {} photos, the capture has {count}", loaded.keypoints.len());
                }
                if options.turntable.surface {
                    let inside: anyhow::Result<Vec<Vec<bool>>> = loaded
                        .keypoints
                        .iter()
                        .enumerate()
                        .map(|(i, points)| {
                            let mask = crate::scene::read_gray(&out.join("masks").join(&names[i]))?;
                            Ok(points.iter().map(|&p| in_mask(&mask, p)).collect())
                        })
                        .collect();
                    object_side = Some(inside?);
                }
                loaded.keypoints.iter_mut().flatten().for_each(|p| *p = shrink(*p));
                if let (true, Some(inside)) = (rings > 1, object_side.as_ref()) {
                    let within: Vec<_> = loaded.pairs.iter().filter(|(i, j, _)| ring_of(*i) == ring_of(*j)).cloned().collect();
                    let moving = moving_features(&within, inside);
                    for (i, j, list) in &mut loaded.pairs {
                        if ring_of(*i) != ring_of(*j) {
                            list.retain(|&(a, b)| moving[*i][a as usize] && moving[*j][b as usize]);
                        }
                    }
                }
                loaded
            }
            None => {
                let settings = features::Settings { maximum: options.turntable.features, ..features::Settings::default() };
                let surface = options.turntable.surface;
                let size = (working.width as usize, working.height as usize);
                let found =
                    run.internal("cameras", "turntable-features", timeouts.features, 0.04, 0.4, "Detecting features", count, |watch| {
                        let work = |n: usize| -> anyhow::Result<(features::Features, Vec<bool>)> {
                            let photo = crate::photos::staging::open_photo(&out.join("work/contrast").join(&names[n]))?;
                            let gray = resized(photo.gray(), size, FilterType::Triangle);
                            let mask = resized(crate::scene::read_gray(&out.join("masks").join(&names[n]))?, size, FilterType::Nearest);
                            let region = if surface { turntable_region(&mask) } else { mask.clone() };
                            let found = features::detect(&gray, Some(&region), &settings);
                            let inside = found.points.iter().map(|p| in_mask(&mask, *p)).collect();
                            Ok((found, inside))
                        };
                        util::parallel(count, threads, work, watch)
                    })?;
                let (found, inside): (Vec<features::Features>, Vec<Vec<bool>>) = found.into_iter().unzip();
                object_side = Some(inside);
                let span = options.turntable.span;
                let mut pairs: Vec<(usize, usize)> = Vec::new();
                for r in 0..rings {
                    pairs.extend(matching::ring_pairs(per, span, options.open_turn).into_iter().map(|(i, j)| (r * per + i, r * per + j)));
                }
                let match_list = |pairs: &[(usize, usize)],
                                  moving: Option<&Vec<Vec<bool>>>,
                                  run: &mut Run,
                                  name: &str,
                                  low: f64,
                                  high: f64,
                                  message: &str| {
                    run.internal("cameras", name, timeouts.matching, low, high, message, pairs.len(), |watch| {
                        let inside = object_side.as_ref().expect("set above");
                        let work = |k: usize| -> anyhow::Result<Vec<(u32, u32)>> {
                            let (i, j) = pairs[k];
                            let mut list = matching::match_pair(&found[i], &found[j], 0.8);
                            if surface && ring_of(i) == ring_of(j) {
                                // Off the object only what turns with it: the camera stands still, so a
                                // feature of the background is found at the same place in every photo.
                                list.retain(|&(a, b)| {
                                    let (p, q) = (found[i].points[a as usize], found[j].points[b as usize]);
                                    inside[i][a as usize] || inside[j][b as usize] || (p[0] - q[0]).hypot(p[1] - q[1]) >= STILL_PIXELS
                                });
                            }
                            if let Some(moving) = moving {
                                list.retain(|&(a, b)| moving[i][a as usize] && moving[j][b as usize]);
                            }
                            Ok(list)
                        };
                        util::parallel(pairs.len(), threads, work, watch)
                    })
                };
                let high = if rings > 1 { 0.5 } else { 0.6 };
                let matched = match_list(&pairs, None, run, "turntable-matching", 0.4, high, "Matching features")?;
                let mut all: Vec<PairMatches> = pairs.iter().zip(matched).map(|(&(i, j), list)| (i, j, list)).collect();
                if rings > 1 {
                    // Across turns only features known to turn with the object: different cameras see the
                    // static surroundings at every angle alike, and such matches would join anything.
                    let moving = moving_features(&all, object_side.as_ref().expect("set above"));
                    let moving = Some(&moving);
                    // Neighbouring turns: the photo of the next turn at the same turntable angle is the one
                    // that shares the most with it. Probe two photos half a turn apart against every
                    // photo of the next turn, then match each photo with that one and its two neighbours.
                    let mut probes = Vec::new();
                    for r in 0..rings - 1 {
                        for j in 0..per {
                            probes.push((r * per, (r + 1) * per + j));
                            probes.push((r * per + per / 2, (r + 1) * per + j));
                        }
                    }
                    let counts = match_list(
                        &probes,
                        moving,
                        run,
                        "turntable-ring-phase",
                        0.5,
                        0.53,
                        "Finding the same angle in neighbouring turns",
                    )?;
                    let mut phases = Vec::new();
                    let mut across = Vec::new();
                    for r in 0..rings - 1 {
                        let block = &counts[r * 2 * per..(r + 1) * 2 * per];
                        let phase = (0..per)
                            .max_by_key(|&shift| block[2 * shift].len() + block[2 * ((shift + per / 2) % per) + 1].len())
                            .unwrap_or(0);
                        phases.push(phase);
                        for k in 0..per {
                            for d in [per - 1, 0, 1] {
                                across.push((r * per + k, (r + 1) * per + (k + phase + d) % per));
                            }
                        }
                    }
                    run.note("turntable_ring_phases", json!(phases));
                    let matched = match_list(&across, moving, run, "turntable-ring-matching", 0.53, 0.6, "Matching features across turns")?;
                    all.extend(across.iter().zip(matched).map(|(&(i, j), list)| (i, j, list)));
                }
                Matches { keypoints: found.iter().map(|f| f.points.clone()).collect(), pairs: all }
            }
        };
        if options.keep_intermediates {
            crate::photos::fs::create_dir_all(out.join("sfm"))?;
            // Saved at the photo resolution, as the debug input reads them.
            let unshrink = |p: &[f64; 2]| [(p[0] + 0.5) / sx - 0.5, (p[1] + 0.5) / sy - 0.5];
            let keypoints: Vec<Vec<[f64; 2]>> = matches.keypoints.iter().map(|list| list.iter().map(unshrink).collect()).collect();
            util::write_json(
                &out.join("sfm/photo-matches.json"),
                &json!({"schema":"crisp3ds_turntable_matches_v1","keypoints":keypoints,"pairs":matches.pairs.iter().map(|(i,j,list)|json!({"first":i,"second":j,"matches":list})).collect::<Vec<_>>()}),
                1,
            )?;
        }
        // One turn's matches with photos numbered from 0.
        let ring_matches = |r: usize, matches: &Matches| Matches {
            keypoints: matches.keypoints[r * per..(r + 1) * per].to_vec(),
            pairs: matches
                .pairs
                .iter()
                .filter(|(i, j, _)| ring_of(*i) == r && ring_of(*j) == r)
                .map(|(i, j, list)| (i - r * per, j - r * per, list.clone()))
                .collect(),
        };
        let mut solved_rings = Vec::new();
        for r in 0..rings {
            let own = if rings == 1 { std::mem::take(&mut matches) } else { ring_matches(r, &matches) };
            let counted: Vec<(usize, usize, usize)> = own.pairs.iter().map(|p| (p.0, p.1, p.2.len())).collect();
            let by_distance = matching::matches_by_distance(per, &counted);
            run.note(
                if rings == 1 { "turntable_order".to_string() } else { format!("turntable_order_ring_{}", r + 1) }.as_str(),
                json!({"median_matches_by_distance": by_distance}),
            );
            matching::check_order(&by_distance)
                .map_err(|reason| if rings == 1 { anyhow!(reason) } else { anyhow!("turn {}: {reason}", r + 1) })?;
            let planar = if options.turntable.surface {
                let inside = &object_side.as_ref().expect("photo features")[r * per..(r + 1) * per];
                match super::planar::estimate(&own, inside, &camera) {
                    Ok(seed) => Some(seed),
                    Err(e) if rings == 1 => return Err(e),
                    // A turn seen from the side shows the turntable edge-on: no plane to fit. Its motion
                    // is searched from the matches like without the surface region.
                    Err(e) => {
                        run.warn(format!("turn {}: {e}; its axis is searched from the matches instead", r + 1));
                        None
                    }
                }
            } else {
                None
            };
            let settings = solver::Settings { open_turn: options.open_turn, wide_axis: options.turntable.surface, planar };
            let (low, high) = (0.6 + 0.22 * r as f64 / rings as f64, 0.6 + 0.22 * (r + 1) as f64 / rings as f64);
            let solved =
                run.internal("cameras", "turntable-solve", timeouts.sfm, low, high, "Recovering cameras (turntable solver)", 1, |_| {
                    solver::solve(&own, &camera, &settings).map_err(|e| if rings == 1 { e } else { anyhow!("turn {}: {e}", r + 1) })
                })?;
            if rings == 1 {
                matches = own;
            } else if options.keep_intermediates {
                // Every turn on its own, in its own frame, for diagnosis of the join.
                let folder = out.join("sfm/turntable");
                crate::photos::fs::create_dir_all(&folder)?;
                util::write_json(
                    &folder.join(format!("turn-{}.json", r + 1)),
                    &json!({"rotations": solved.rotations, "translations": solved.translations, "points": solved.points,
                            "observations": solved.observations, "report": solved.report}),
                    0,
                )?;
            }
            solved_rings.push(solved);
        }
        let solved = if rings == 1 {
            solved_rings.pop().expect("one turn")
        } else {
            run.internal("cameras", "turntable-join", timeouts.sfm, 0.8, 0.82, "Joining the turns", 1, |_| {
                solver::join(&matches, &camera, &solved_rings)
            })?
        };
        let folder = out.join("sfm/turntable");
        crate::photos::fs::create_dir_all(&folder)?;
        let mut report = solved.report.clone();
        let mut counts: Vec<f64> = matches.keypoints.iter().map(|k| k.len() as f64).collect();
        let total: usize = matches.pairs.iter().map(|p| p.2.len()).sum();
        report["features_per_photo"] =
            json!({"min": counts.iter().cloned().fold(f64::INFINITY, f64::min), "median": crate::inputs::median_f64(&mut counts)});
        report["matches"] = json!({"pairs": matches.pairs.len(), "total": total});
        report["working_size"] = json!([working.width, working.height]);
        util::write_json(&folder.join("report.json"), &report, 1)?;
        if let Some(object) = report.as_object_mut() {
            object.remove("steps_deg");
            object.remove("adjustments");
            if let Some(Value::Array(list)) = object.get_mut("rings") {
                for ring in list.iter_mut().filter_map(Value::as_object_mut) {
                    ring.remove("steps_deg");
                    ring.remove("adjustments");
                }
            }
        }
        let mut solution = solver::solution(&working, &names, &matches, &solved);
        if options.turntable.surface {
            // The scene is located by points of the object, not of the turntable around it: a point
            // counts when most of the photos that see it see it inside the mask.
            let inside = object_side.as_ref().expect("surface features are found here");
            let mut votes = vec![(0usize, 0usize); solved.points.len()];
            for &(view, point, feature) in &solved.observations {
                votes[point].1 += 1;
                votes[point].0 += inside[view][feature as usize] as usize;
            }
            let object: Vec<_> = solved.points.iter().zip(&votes).filter(|(_, (a, n))| 2 * a > *n).map(|(p, _)| *p).collect();
            report["surface_points"] = json!({"object": object.len(), "turntable": solved.points.len() - object.len()});
            solution.object_points = Some(object);
        }
        // Back to the photo resolution: the poses do not change, the lens and the observations scale.
        solution.lens = lens;
        for landmark in &mut solution.landmarks {
            for (_, p) in &mut landmark.observations {
                *p = [(p[0] + 0.5) / sx - 0.5, (p[1] + 0.5) / sy - 0.5];
            }
        }
        run.note("turntable", report);
        Ok(solution)
    }

    fn intermediates(&self) -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_lens_keeps_small_photos_and_scales_pixel_centres() {
        let lens = Lens { width: 4272, height: 2848, pixels: [3562.8, 3562.8, 2142.1, 1407.6], k: [0.1, 0.0, 0.0] };
        assert_eq!(working_lens(&lens, 0), lens);
        assert_eq!(working_lens(&lens, 4272), lens);
        let small = working_lens(&lens, 2136);
        assert_eq!((small.width, small.height), (2136, 1424));
        // The same ray through the principal point and through a corner pixel's centre.
        assert!((small.pixels[0] - 1781.4).abs() < 1e-9 && (small.pixels[2] - 1070.8).abs() < 1e-9);
        let full = Camera::from_lens(&lens).undistort([0.0, 0.0]);
        let half = Camera::from_lens(&small).undistort([-0.25, -0.25]);
        assert!((full[0] - half[0]).abs() < 1e-12 && (full[1] - half[1]).abs() < 1e-12);
        assert_eq!(small.k, lens.k);
    }
}
