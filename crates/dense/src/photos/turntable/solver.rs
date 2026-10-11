//! Cameras of an ordered turntable capture from feature matches
//! (`docs/TURNTABLE-SOLVER.md`): one repeated rotation about a fixed axis as
//! the starting point, then tracks and a bundle adjustment with free poses.
//! Port of `crates/dense/tools/turntable_prototype.py`.

use std::collections::HashMap;

use anyhow::bail;
use serde_json::{json, Value};

use crate::inputs::median_f64;
use crate::photos::markers::linalg::{add, apply, cross, dot, mul, norm, rodrigues, scale, sub, transpose, unit, Camera, M3, V3};
use crate::photos::solution::{Landmark, Lens, Solution, View};

use super::adjust::{adjust, adjust_turns, residuals, Observation, Turn};
use super::geometry::{essential_of, sampson, triangulate, P2};

/// `(first photo, second photo, [(feature in first, feature in second)])`.
pub type PairMatches = (usize, usize, Vec<(u32, u32)>);

/// Features of every photo (pixels, centre of the top-left pixel at (0, 0)) and matches between photo pairs.
#[derive(Debug, Clone, Default)]
pub struct Matches {
    pub keypoints: Vec<Vec<P2>>,
    /// `(first photo, second photo, [(feature in first, feature in second)])`.
    pub pairs: Vec<PairMatches>,
}

impl Matches {
    /// The file the prototype writes with `--export-matches` (`crisp3ds_turntable_matches_v1`).
    pub fn from_json(value: &Value) -> anyhow::Result<Matches> {
        if value["schema"] != "crisp3ds_turntable_matches_v1" {
            bail!("not a turntable matches file");
        }
        let keypoints: Vec<Vec<P2>> = serde_json::from_value(value["keypoints"].clone())?;
        let mut pairs = Vec::new();
        for pair in value["pairs"].as_array().ok_or_else(|| anyhow::anyhow!("matches pairs must be an array"))? {
            let list: Vec<(u32, u32)> = serde_json::from_value(pair["matches"].clone())?;
            let first =
                pair["first"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| anyhow::anyhow!("invalid first view index"))?;
            let second = pair["second"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| anyhow::anyhow!("invalid second view index"))?;
            pairs.push((first, second, list));
        }
        if keypoints.iter().flatten().flatten().any(|v| !v.is_finite())
            || pairs.iter().any(|(i, j, list)| {
                *i >= keypoints.len()
                    || *j >= keypoints.len()
                    || list.iter().any(|&(a, b)| a as usize >= keypoints[*i].len() || b as usize >= keypoints[*j].len())
            })
        {
            bail!("matches contain invalid coordinates or feature indices");
        }
        Ok(Matches { keypoints, pairs })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The photos do not close a full turn: do not scale the steps to 360 degrees.
    pub open_turn: bool,
    /// Expanded search for downward views, used only with experimental surface features.
    pub wide_axis: bool,
    pub planar: Option<super::planar::Seed>,
}

/// Turns that the steps of a closed capture may add up to before they are scaled to one. Measured on
/// correct solutions: 0.78 (the Happy Buddha, rotation and translation hard to tell apart) to 1.07 (25
/// photos at 15 degrees); a wrong axis gave 1.19 (a smooth bottle seen from a steep camera).
const CLOSING_TURNS: (f64, f64) = (0.75, 1.15);

/// The step motion of the turntable model: rotation by `angle` about the axis through `centre`.
fn step_motion(axis: V3, centre: V3, angle: f64) -> (M3, V3) {
    let rotation = rodrigues(scale(axis, angle));
    (rotation, sub(centre, apply(&rotation, centre)))
}

/// Minimum of a function on an interval by golden-section search.
fn minimise(f: &dyn Fn(f64) -> f64, mut low: f64, mut high: f64) -> f64 {
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (high - ratio * (high - low), low + ratio * (high - low));
    let (mut fa, mut fb) = (f(a), f(b));
    for _ in 0..40 {
        if fa < fb {
            (high, b, fb) = (b, a, fa);
            a = high - ratio * (high - low);
            fa = f(a);
        } else {
            (low, a, fa) = (a, b, fb);
            b = low + ratio * (high - low);
            fb = f(b);
        }
    }
    (low + high) / 2.0
}

struct Tracks {
    points: Vec<V3>,
    /// `(photo, point, feature)`.
    observations: Vec<(usize, usize, u32)>,
    verified: usize,
}

/// Matches that agree with the poses (Sampson distance below `bound` pixels), joined into tracks of
/// at least three photos and triangulated.
fn build(matches: &Matches, normalised: &[Vec<P2>], rotations: &[M3], translations: &[V3], focal: f64, bound: f64) -> Tracks {
    let mut ids: HashMap<(usize, u32), usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    let mut nodes: Vec<(usize, u32)> = Vec::new();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut verified = 0;
    for (i, j, list) in &matches.pairs {
        let relative = mul(&rotations[*j], &transpose(&rotations[*i]));
        let translation = sub(translations[*j], apply(&relative, translations[*i]));
        if norm(translation) < 1e-6 {
            continue;
        }
        let e = essential_of(&relative, translation);
        for &(a, b) in list {
            if sampson(&e, normalised[*i][a as usize], normalised[*j][b as usize]) * focal >= bound {
                continue;
            }
            verified += 1;
            let mut node = |key: (usize, u32)| {
                *ids.entry(key).or_insert_with(|| {
                    parent.push(parent.len());
                    nodes.push(key);
                    parent.len() - 1
                })
            };
            let (x, y) = (node((*i, a)), node((*j, b)));
            let (rx, ry) = (find(&mut parent, x), find(&mut parent, y));
            parent[rx] = ry;
        }
    }
    let mut groups: HashMap<usize, Vec<(usize, u32)>> = HashMap::new();
    for (index, key) in nodes.iter().enumerate() {
        groups.entry(find(&mut parent, index)).or_default().push(*key);
    }
    let mut tracks: Vec<Vec<(usize, u32)>> = groups.into_values().collect();
    for track in &mut tracks {
        track.sort_unstable();
    }
    // A fixed order, whatever the hash map did.
    tracks.sort_unstable();
    let (mut points, mut observations) = (Vec::new(), Vec::new());
    for track in tracks {
        let distinct = track.windows(2).all(|w| w[0].0 != w[1].0);
        if track.len() < 3 || !distinct {
            continue;
        }
        let seen: Vec<(M3, V3, P2)> = track.iter().map(|&(v, f)| (rotations[v], translations[v], normalised[v][f as usize])).collect();
        let Some(x) = triangulate(&seen) else { continue };
        let mut worst = 0f64;
        let mut front = true;
        for (r, t, p) in &seen {
            let q = add(apply(r, x), *t);
            front &= q[2] > 0.0;
            worst = worst.max((q[0] / q[2] - p[0]).hypot(q[1] / q[2] - p[1]) * focal);
        }
        if front && worst < 2.0 * bound {
            observations.extend(track.iter().map(|&(v, f)| (v, points.len(), f)));
            points.push(x);
        }
    }
    Tracks { points, observations, verified }
}

/// What the solver found, with the numbers for the report.
pub struct Solved {
    pub rotations: Vec<M3>,
    pub translations: Vec<V3>,
    pub points: Vec<V3>,
    /// `(photo, point, feature)`.
    pub observations: Vec<(usize, usize, u32)>,
    pub report: Value,
}

impl Solved {
    /// A turn without its own solution: `per` cameras at one point, so no plausible ring; `join` registers
    /// its photos one by one.
    pub fn unsolved(per: usize, reason: &str) -> Self {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        Solved {
            rotations: vec![identity; per],
            translations: vec![[0.0; 3]; per],
            points: Vec::new(),
            observations: Vec::new(),
            report: json!({"unsolved": reason}),
        }
    }
}

/// Solves the cameras of `matches` for a lens `camera`. The frame is the first camera's; the
/// distance from the camera to the turntable axis is 1 before the adjustment.
pub fn solve(matches: &Matches, camera: &Camera, settings: &Settings) -> anyhow::Result<Solved> {
    let n = matches.keypoints.len();
    if n < 8 {
        bail!("the turntable solver needs at least 8 photos, found {n}");
    }
    let focal = (camera.fx + camera.fy) / 2.0;
    let normalised: Vec<Vec<P2>> = matches.keypoints.iter().map(|list| list.iter().map(|p| camera.undistort(*p)).collect()).collect();
    let pair = |i: usize, j: usize| matches.pairs.iter().find(|p| p.0 == i && p.1 == j).map(|p| p.2.as_slice()).unwrap_or(&[]);
    let consecutive: Vec<(Vec<P2>, Vec<P2>)> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            pair(i, j).iter().map(|&(a, b)| (normalised[i][a as usize], normalised[j][b as usize])).unzip()
        })
        .collect();
    let flow = |a: &[P2], b: &[P2]| -> f64 {
        if a.is_empty() {
            return 0.0;
        }
        median_f64(&mut a.iter().zip(b).map(|(p, q)| (p[0] - q[0]).hypot(p[1] - q[1])).collect::<Vec<_>>()) * focal
    };

    // One motion for every step: a rotation about a fixed axis. Its direction, the direction from the
    // camera to it (at distance 1, which fixes the scale) and the angle of every step are fitted to the
    // matches of all consecutive pairs together. A single pair of a small object seen through a narrow
    // field barely tells rotation from translation; all pairs sharing one axis do.
    let moving: Vec<usize> = (0..n).filter(|&i| consecutive[i].0.len() >= 30 && flow(&consecutive[i].0, &consecutive[i].1) > 2.0).collect();
    if moving.len() < 3 {
        bail!("only {} consecutive photo pairs show motion; too few matches between neighbours", moving.len());
    }
    let mut moved = vec![false; n];
    for &i in &moving {
        moved[i] = true;
    }
    // At most 400 matches per pair for the fit; all of them for the angles afterwards.
    let sample: Vec<Vec<(P2, P2)>> = moving
        .iter()
        .map(|&i| {
            let (a, b) = &consecutive[i];
            let stride = a.len().div_ceil(400).max(1);
            (0..a.len()).step_by(stride).map(|k| (a[k], b[k])).collect()
        })
        .collect();
    let pair_cost = |pairs: &[(P2, P2)], axis: V3, centre: V3, angle: f64| -> f64 {
        let (rotation, t) = step_motion(axis, centre, angle);
        let e = essential_of(&rotation, t);
        pairs.iter().map(|(p, q)| (sampson(&e, *p, *q) * focal).min(3.0)).sum()
    };
    let nominal = 2.0 * std::f64::consts::PI / n as f64;
    // The best angle of one pair for a given axis: a coarse scan, then a fine search.
    let best_angle = |pairs: &[(P2, P2)], axis: V3, centre: V3, typical: f64, fine: bool| -> (f64, f64) {
        let mut best = (f64::INFINITY, typical);
        for k in 0..24 {
            let angle = typical * (0.2 + 2.3 * k as f64 / 23.0);
            let value = pair_cost(pairs, axis, centre, angle);
            if value < best.0 {
                best = (value, angle);
            }
        }
        if fine {
            let reach = 0.1 * typical.abs();
            let angle = minimise(&|x| pair_cost(pairs, axis, centre, x), best.1 - reach, best.1 + reach);
            best = (pair_cost(pairs, axis, centre, angle), angle);
        }
        best
    };
    let total_cost =
        |axis: V3, centre: V3, typical: f64| -> f64 { sample.iter().map(|pairs| best_angle(pairs, axis, centre, typical, false).0).sum() };
    // The axis is roughly upright in the photos and the object in front of the camera: start from the
    // best of a few tilts and both turning directions, towards where the features are.
    let mean = {
        let all: Vec<P2> = moving.iter().flat_map(|&i| consecutive[i].0.iter().copied()).collect();
        let count = all.len().max(1) as f64;
        [all.iter().map(|p| p[0]).sum::<f64>() / count, all.iter().map(|p| p[1]).sum::<f64>() / count, 1.0]
    };
    let across = |axis: V3, v: V3| unit(sub(v, scale(axis, dot(axis, v))));
    let mut start = (f64::INFINITY, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], nominal);
    for tilt in [-80.0f64, -70.0, -60.0, -50.0, -40.0, -30.0, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0] {
        if !settings.wide_axis && tilt.abs() > 50.0 {
            continue;
        }
        let axis = [0.0, tilt.to_radians().cos(), tilt.to_radians().sin()];
        let centre = across(axis, mean);
        for typical in [nominal, -nominal] {
            let value = total_cost(axis, centre, typical);
            if value < start.0 {
                start = (value, axis, centre, typical);
            }
        }
    }
    if let Some(seed) = &settings.planar {
        start = (0.0, seed.axis, seed.centre, median_f64(&mut seed.steps.clone()));
    }
    let (_, mut axis, mut centre, signed) = start;
    // Refinement: the axis direction (two angles) and the direction to the axis (one angle about it),
    // one after the other over shrinking ranges, the step angles re-fitted inside every evaluation.
    for range in [12.0f64, 6.0, 3.0, 1.5, 0.7, 0.3, 0.1] {
        if settings.planar.is_some() {
            break;
        }
        let range = range.to_radians();
        for parameter in 0..3 {
            let moved_to = |x: f64| -> (V3, V3) {
                let turn = match parameter {
                    0 => rodrigues(scale(centre, x)),
                    1 => rodrigues(scale(crate::photos::markers::linalg::cross(axis, centre), x)),
                    _ => rodrigues(scale(axis, x)),
                };
                let new_axis = if parameter < 2 { unit(apply(&turn, axis)) } else { axis };
                (new_axis, across(new_axis, apply(&turn, centre)))
            };
            let evaluate = |x: f64| {
                let (a, c) = moved_to(x);
                total_cost(a, c, signed)
            };
            let candidate = moved_to(minimise(&evaluate, -range, range));
            if total_cost(candidate.0, candidate.1, signed) < total_cost(axis, centre, signed) {
                (axis, centre) = candidate;
            }
        }
    }
    let mut angles: Vec<f64> = sample.iter().map(|pairs| best_angle(pairs, axis, centre, signed, true).1).collect();
    let typical = if let Some(seed) = &settings.planar { median_f64(&mut seed.steps.clone()) } else { median_f64(&mut angles) };
    let axes = &moving;

    // The angle of every step with the axis fixed, from all its matches.
    let mut steps = Vec::with_capacity(n);
    for (i, (a, b)) in consecutive.iter().enumerate() {
        if a.len() < 12 {
            steps.push(typical);
            continue;
        }
        // A step of zero has no epipolar geometry; it shows as no image motion.
        if !moved[i] && flow(a, b) <= 2.0 {
            steps.push(0.0);
            continue;
        }
        let pairs: Vec<(P2, P2)> = a.iter().copied().zip(b.iter().copied()).collect();
        steps.push(best_angle(&pairs, axis, centre, typical, true).1);
    }
    if let Some(seed) = &settings.planar {
        steps = seed.steps.clone();
    }
    // Work with positive steps: an axis pointing the other way turns the same way.
    if typical < 0.0 {
        axis = scale(axis, -1.0);
        steps.iter_mut().for_each(|s| *s = -*s);
    }
    let typical = typical.abs();
    // A step far from the others is a failed search, not a jump of the turntable.
    let usual = median_f64(&mut steps.iter().copied().filter(|s| *s > 0.0).collect::<Vec<_>>());
    for step in &mut steps {
        if *step > 1.6 * typical || (*step < 0.4 * typical && *step > 0.0) {
            *step = usual;
        }
    }
    let raw_sum: f64 = steps.iter().sum();
    let turns = raw_sum / (2.0 * std::f64::consts::PI);
    // A full turn closes: all steps, the one back to the first photo included, add up to 360 degrees.
    // Steps found from the matches add up to close to one turn when the axis is right; far from it
    // the motion was not found, and scaling the steps to 360 degrees would only hide that (the
    // adjustment then settles on wrong cameras that pass every later gate).
    if !settings.open_turn && !(CLOSING_TURNS.0..=CLOSING_TURNS.1).contains(&turns) {
        bail!(
            "the steps found add up to {:.0} degrees, not about one turn ({:.0} to {:.0}): the turntable's motion was not found \
             (an axis tilted further than the search reaches, or too few features on the object); pass --open-turn if the \
             photos are not one closed turn",
            raw_sum.to_degrees(),
            360.0 * CLOSING_TURNS.0,
            360.0 * CLOSING_TURNS.1
        );
    }
    let closed = !settings.open_turn;
    if closed {
        for step in &mut steps {
            *step /= turns;
        }
    }
    let (mut rotations, mut translations) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut total = 0.0;
    for step in &steps {
        let (rotation, t) = step_motion(axis, centre, total);
        rotations.push(rotation);
        translations.push(t);
        total += step;
    }

    let (tracks, mut errors, rounds, first_tracks) =
        refine(matches, &normalised, &mut rotations, &mut translations, focal, &mut Vec::new())?;
    let mut sorted = errors.clone();
    sorted.sort_by(f64::total_cmp);
    let report = json!({
        "planar_seed_pairs": settings.planar.as_ref().map(|s|s.pairs), "planar_seed_inlier_fraction": settings.planar.as_ref().map(|s|s.inlier_fraction), "photos": n, "pairs_with_motion": axes.len(), "axis": axis, "direction_to_axis": centre,
        "step_median_deg": typical.to_degrees(), "turn_sum_deg": raw_sum.to_degrees(), "closed_to_full_turn": closed,
        "steps_deg": steps.iter().map(|s| (s.to_degrees() * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
        "first_tracks": {"verified_matches": first_tracks.0, "points": first_tracks.1, "observations": first_tracks.2},
        "adjustments": rounds, "points": tracks.points.len(), "observations": tracks.observations.len(),
        "reprojection_px": {"median": median_f64(&mut errors), "p95": crate::inputs::percentile_sorted_f64(&sorted, 95.0)},
    });
    Ok(Solved { rotations, translations, points: tracks.points, observations: tracks.observations, report })
}

/// Tracks from the matches under the poses, then adjustment rounds: once more tracks with the adjusted
/// poses, then twice without the far observations. Returns the tracks, the final pixel errors of their
/// observations, the rounds for the report and the first tracks' counts.
#[allow(clippy::type_complexity)]
fn refine(
    matches: &Matches,
    normalised: &[Vec<P2>],
    rotations: &mut Vec<M3>,
    translations: &mut Vec<V3>,
    focal: f64,
    turns: &mut Vec<Turn>,
) -> anyhow::Result<(Tracks, Vec<f64>, Vec<Value>, (usize, usize, usize))> {
    // Tracks, then adjustment rounds: once more tracks with the adjusted poses, then twice without the far observations.
    let mut tracks = build(matches, normalised, rotations, translations, focal, 4.0);
    let first_tracks = (tracks.verified, tracks.points.len(), tracks.observations.len());
    let mut rounds = Vec::new();
    let mut errors: Vec<f64> = Vec::new();
    for round in 0..4 {
        if tracks.points.len() < 20 {
            bail!("only {} points could be triangulated from the matches", tracks.points.len());
        }
        let observations: Vec<Observation> =
            tracks.observations.iter().map(|&(v, p, f)| Observation { camera: v, point: p, target: normalised[v][f as usize] }).collect();
        let (before, after) = if turns.is_empty() {
            adjust(rotations, translations, &mut tracks.points, &observations, focal, 40)
        } else {
            adjust_turns(rotations, translations, &mut tracks.points, &observations, focal, 40, turns)
        };
        let (r, _) = residuals(rotations, translations, &tracks.points, &observations, focal);
        errors = r.iter().map(|d| d[0].hypot(d[1])).collect();
        let median = median_f64(&mut errors.clone());
        rounds.push(json!({"observations": observations.len(), "points": tracks.points.len(), "cost_before": before, "cost_after": after, "median_px": median}));
        if round == 3 {
            break;
        }
        if round == 0 {
            tracks = build(matches, normalised, rotations, translations, focal, 3.0);
            continue;
        }
        let bound = (3.0 * median).max(2.0);
        let kept: Vec<(usize, usize, u32)> =
            tracks.observations.iter().zip(&errors).filter(|(_, e)| **e < bound).map(|(o, _)| *o).collect();
        let mut count = vec![0usize; tracks.points.len()];
        for o in &kept {
            count[o.1] += 1;
        }
        let mut renumber = vec![usize::MAX; tracks.points.len()];
        let mut points = Vec::new();
        for (p, x) in tracks.points.iter().enumerate() {
            if count[p] >= 2 {
                renumber[p] = points.len();
                points.push(*x);
            }
        }
        tracks.observations = kept.into_iter().filter(|o| renumber[o.1] != usize::MAX).map(|(v, p, f)| (v, renumber[p], f)).collect();
        tracks.points = points;
    }
    Ok((tracks, errors, rounds, first_tracks))
}

/// Similarity `y = s R x + t` closest to the pairs in least squares (Umeyama), `None` if degenerate.
fn similarity(x: &[V3], y: &[V3]) -> Option<(f64, M3, V3)> {
    let n = x.len() as f64;
    let mean = |v: &[V3]| v.iter().fold([0.0; 3], |a, b| add(a, scale(*b, 1.0 / n)));
    let (mx, my) = (mean(x), mean(y));
    let mut covariance = [[0.0; 3]; 3];
    let mut spread = 0.0;
    for (a, b) in x.iter().zip(y) {
        let (a, b) = (sub(*a, mx), sub(*b, my));
        spread += dot(a, a) / n;
        for (i, row) in covariance.iter_mut().enumerate() {
            for (j, value) in row.iter_mut().enumerate() {
                *value += b[i] * a[j] / n;
            }
        }
    }
    if spread <= 1e-300 {
        return None;
    }
    let (u, d, v) = super::geometry::svd3(&covariance);
    let flip = if crate::photos::markers::linalg::determinant(&mul(&u, &transpose(&v))) < 0.0 { -1.0 } else { 1.0 };
    let signs = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, flip]];
    let rotation = mul(&u, &mul(&signs, &transpose(&v)));
    let factor = (d[0] + d[1] + flip * d[2]) / spread;
    (factor.is_finite() && factor > 0.0).then(|| (factor, rotation, sub(my, scale(apply(&rotation, mx), factor))))
}

/// Whether a turn's own solution is a plausible ring: its camera centres lie on one circle (radius
/// spread and distance from the plane within 5 % of the radius, the ring gates' default limits). A
/// camera looking almost along the turntable axis sees mostly in-plane rotation, and a turn of such
/// photos can settle on a wrong ring; it is then registered photo by photo against the other turns.
fn sound_ring(solved: &Solved) -> bool {
    let cameras: Vec<crate::photos::ring::SceneCamera> = solved
        .rotations
        .iter()
        .zip(&solved.translations)
        .enumerate()
        .map(|(index, (r, t))| crate::photos::ring::SceneCamera {
            index,
            name: String::new(),
            rotation: *r,
            centre: scale(apply(&transpose(r), *t), -1.0),
        })
        .collect();
    let ring = crate::photos::ring::ring_statistics(&cameras, 0.5);
    let value = |name: &str| ring[name].as_f64().unwrap_or(f64::INFINITY);
    ring["degenerate"] == false && value("radius_spread_percent") <= 5.0 && value("out_of_plane_percent") <= 5.0
}

/// Pose of one camera from points it sees (normalised image position, world point). Candidates: the
/// direct linear transform on six points inside RANSAC (not for points on one plane, such as a turntable
/// seen from above) and the `starts`, poses of cameras near this one. Each is refined by Gauss-Newton on
/// the six pose numbers with Huber weights while the inlier bound shrinks; the candidate with the most
/// observations within 3 pixels wins. Returns the pose and that number.
fn resect(seen: &[(P2, V3)], starts: &[(M3, V3)], focal: f64, random: &mut super::geometry::Random) -> Option<(M3, V3, usize)> {
    if seen.len() < 12 {
        return None;
    }
    let n = seen.len() as f64;
    let middle = seen.iter().fold([0.0; 3], |a, s| add(a, scale(s.1, 1.0 / n)));
    let spread = seen.iter().map(|s| norm(sub(s.1, middle))).sum::<f64>() / n;
    let error = |r: &M3, t: V3, (p, x): &(P2, V3)| -> f64 {
        let q = add(apply(r, *x), t);
        if q[2] <= 0.0 {
            return f64::INFINITY;
        }
        (q[0] / q[2] - p[0]).hypot(q[1] / q[2] - p[1]) * focal
    };
    let dlt = |sample: &[usize]| -> Option<(M3, V3)> {
        let mut ata = vec![vec![0.0; 12]; 12];
        for &k in sample {
            let (p, x) = seen[k];
            let x = scale(sub(x, middle), 1.0 / spread);
            let h = [x[0], x[1], x[2], 1.0];
            let mut rows = [[0.0; 12]; 2];
            for c in 0..4 {
                rows[0][c] = h[c];
                rows[0][8 + c] = -p[0] * h[c];
                rows[1][4 + c] = h[c];
                rows[1][8 + c] = -p[1] * h[c];
            }
            for row in &rows {
                for i in 0..12 {
                    for j in 0..12 {
                        ata[i][j] += row[i] * row[j];
                    }
                }
            }
        }
        let (_, vectors) = crate::photos::markers::linalg::symmetric_eigen(&ata);
        let v = &vectors[0];
        let mut m: M3 = [[v[0], v[1], v[2]], [v[4], v[5], v[6]], [v[8], v[9], v[10]]];
        let mut t = [v[3], v[7], v[11]];
        if crate::photos::markers::linalg::determinant(&m) < 0.0 {
            m = m.map(|row| row.map(|e| -e));
            t = scale(t, -1.0);
        }
        let (u, d, w) = super::geometry::svd3(&m);
        let lambda = (d[0] + d[1] + d[2]) / 3.0;
        if lambda <= 1e-12 {
            return None;
        }
        // Back from the normalised points: x' = (x - middle) / spread.
        let r = mul(&u, &transpose(&w));
        Some((r, sub(scale(t, spread / lambda), apply(&r, middle))))
    };
    let mut best: Option<(usize, M3, V3)> = None;
    for _ in 0..500 {
        let mut sample = Vec::with_capacity(6);
        while sample.len() < 6 {
            let k = random.below(seen.len());
            if !sample.contains(&k) {
                sample.push(k);
            }
        }
        let Some((r, t)) = dlt(&sample) else { continue };
        let count = seen.iter().filter(|s| error(&r, t, s) < 4.0).count();
        if best.as_ref().is_none_or(|b| count > b.0) {
            best = Some((count, r, t));
        }
    }
    let polish = |mut r: M3, mut t: V3, bounds: &[f64]| -> Option<(M3, V3, usize)> {
        for &bound in bounds {
            let inliers: Vec<&(P2, V3)> = seen.iter().filter(|s| error(&r, t, s) < bound).collect();
            if inliers.len() < 12 {
                return None;
            }
            // Gauss-Newton on a rotation increment and the translation, numeric derivatives.
            for _ in 0..10 {
                let residual = |r: &M3, t: V3| -> Vec<f64> {
                    inliers
                        .iter()
                        .flat_map(|(p, x)| {
                            let q = add(apply(r, *x), t);
                            [(q[0] / q[2] - p[0]) * focal, (q[1] / q[2] - p[1]) * focal]
                        })
                        .collect()
                };
                let moved = |delta: &[f64]| -> (M3, V3) {
                    (mul(&rodrigues([delta[0], delta[1], delta[2]]), &r), add(t, [delta[3], delta[4], delta[5]]))
                };
                let base = residual(&r, t);
                if base.iter().any(|v| !v.is_finite()) {
                    return None;
                }
                let step = 1e-6;
                let columns: Vec<Vec<f64>> = (0..6)
                    .map(|a| {
                        let mut delta = [0.0; 6];
                        delta[a] = step;
                        let (rr, tt) = moved(&delta);
                        residual(&rr, tt).iter().zip(&base).map(|(x, y)| (x - y) / step).collect()
                    })
                    .collect();
                let mut a = vec![vec![0.0; 6]; 6];
                let mut b = vec![0.0; 6];
                for k in 0..base.len() {
                    let weight = if base[k].abs() <= 1.0 { 1.0 } else { 1.0 / base[k].abs() };
                    for i in 0..6 {
                        b[i] -= weight * columns[i][k] * base[k];
                        for j in 0..6 {
                            a[i][j] += weight * columns[i][k] * columns[j][k];
                        }
                    }
                }
                for (i, row) in a.iter_mut().enumerate() {
                    row[i] *= 1.0 + 1e-6;
                }
                let Some(delta) = crate::photos::markers::linalg::solve(a, b) else { break };
                (r, t) = moved(&delta);
                if delta.iter().map(|d| d.abs()).fold(0.0, f64::max) < 1e-10 {
                    break;
                }
            }
        }
        let count = seen.iter().filter(|s| error(&r, t, s) < 3.0).count();
        Some((r, t, count))
    };
    let mut candidates = Vec::new();
    if let Some((_, r, t)) = best {
        candidates.extend(polish(r, t, &[4.0, 3.0, 3.0]));
    }
    for &(r, t) in starts {
        candidates.extend(polish(r, t, &[f64::INFINITY, 100.0, 30.0, 10.0, 4.0, 3.0, 3.0]));
    }
    candidates.into_iter().max_by_key(|c| c.2)
}

/// Several turns of one turntable at different camera heights, each solved on its own
/// (`rings[r]` holds the photos `r * per ..(r + 1) * per` of `matches`, with their own frames), joined
/// into one frame and adjusted together. A turn whose own solution is a plausible ring is placed by a
/// similarity fitted with RANSAC to the points of its own solution that matches across rings link to points
/// already placed (the first such turn sets the frame). The photos of the other turns are then registered
/// one by one from their matches to the tracks of the placed photos. Last, all matches, those across turns
/// included, are made into tracks and adjusted with every pose free.
pub fn join(matches: &Matches, camera: &Camera, rings: &[Solved]) -> anyhow::Result<Solved> {
    let per = rings.first().map(|r| r.rotations.len()).unwrap_or(0);
    let count = per * rings.len();
    if rings.len() < 2 || matches.keypoints.len() != count || rings.iter().any(|r| r.rotations.len() != per) {
        bail!("rings of unequal length or the wrong number of photos");
    }
    let focal = (camera.fx + camera.fy) / 2.0;
    // (photo, feature) -> (ring, point of that ring's solution)
    let mut point_of: HashMap<(usize, u32), (usize, usize)> = HashMap::new();
    for (r, ring) in rings.iter().enumerate() {
        for &(view, point, feature) in &ring.observations {
            point_of.insert((r * per + view, feature), (r, point));
        }
    }
    let mut rotations = vec![[[0.0; 3]; 3]; count];
    let mut translations = vec![[0.0; 3]; count];
    let mut world: Vec<Option<Vec<V3>>> = vec![None; rings.len()];
    let place = |r: usize, s: f64, rotation: &M3, t: V3, rotations: &mut [M3], translations: &mut [V3], world: &mut [Option<Vec<V3>>]| {
        let back = transpose(rotation);
        for v in 0..per {
            let (rc, tc) = (rings[r].rotations[v], rings[r].translations[v]);
            let turned = mul(&rc, &back);
            rotations[r * per + v] = turned;
            translations[r * per + v] = sub(scale(tc, s), apply(&turned, t));
        }
        world[r] = Some(rings[r].points.iter().map(|p| add(scale(apply(rotation, *p), s), t)).collect());
    };
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let sound: Vec<bool> = rings.iter().map(sound_ring).collect();
    let Some(first) = sound.iter().position(|&s| s) else { bail!("no turn was solved as a plausible ring") };
    place(first, 1.0, &identity, [0.0; 3], &mut rotations, &mut translations, &mut world);
    let mut placements = Vec::new();
    let mut random = super::geometry::Random(0x005e_ed0f_71e5);
    while (0..rings.len()).any(|r| sound[r] && world[r].is_none()) {
        // The unplaced ring with the most distinct point links to the placed ones.
        let mut best: Option<(usize, Vec<(V3, V3)>)> = None;
        for r in (0..rings.len()).filter(|&r| sound[r] && world[r].is_none()) {
            let mut links: std::collections::BTreeMap<(usize, usize, usize), usize> = std::collections::BTreeMap::new();
            for (i, j, list) in &matches.pairs {
                for &(a, b) in list {
                    let (Some(&(ra, pa)), Some(&(rb, pb))) = (point_of.get(&(*i, a)), point_of.get(&(*j, b))) else { continue };
                    let (placed, own) = if rb == r && world[ra].is_some() {
                        ((ra, pa), pb)
                    } else if ra == r && world[rb].is_some() {
                        ((rb, pb), pa)
                    } else {
                        continue;
                    };
                    *links.entry((placed.0, placed.1, own)).or_default() += 1;
                }
            }
            let pairs: Vec<(V3, V3)> =
                links.keys().map(|&(rp, p, own)| (rings[r].points[own], world[rp].as_ref().expect("placed")[p])).collect();
            if best.as_ref().is_none_or(|b| pairs.len() > b.1.len()) {
                best = Some((r, pairs));
            }
        }
        let (r, pairs) = best.expect("an unplaced ring");
        if pairs.len() < 12 {
            bail!("ring {} shares only {} matched points with the rings already joined; too few to join it", r + 1, pairs.len());
        }
        // Inlier bound: a small share of the placed points' spread around their centre.
        let target: Vec<V3> = pairs.iter().map(|p| p.1).collect();
        let centre = target.iter().fold([0.0; 3], |a, b| add(a, scale(*b, 1.0 / target.len() as f64)));
        let mut radii: Vec<f64> = target.iter().map(|p| norm(sub(*p, centre))).collect();
        let bound = 0.02 * median_f64(&mut radii);
        let inliers_of = |(s, rotation, t): &(f64, M3, V3)| -> Vec<bool> {
            pairs.iter().map(|(x, y)| norm(sub(add(scale(apply(rotation, *x), *s), *t), *y)) < bound).collect()
        };
        let mut chosen: Option<(usize, Vec<bool>)> = None;
        for _ in 0..2000 {
            let sample: Vec<usize> = (0..3).map(|_| random.below(pairs.len())).collect();
            if sample[0] == sample[1] || sample[1] == sample[2] || sample[0] == sample[2] {
                continue;
            }
            let (x, y): (Vec<V3>, Vec<V3>) = sample.iter().map(|&k| pairs[k]).unzip();
            let Some(model) = similarity(&x, &y) else { continue };
            let inliers = inliers_of(&model);
            let n = inliers.iter().filter(|&&k| k).count();
            if chosen.as_ref().is_none_or(|c| n > c.0) {
                chosen = Some((n, inliers));
            }
        }
        let (mut n, mut inliers) = chosen.ok_or_else(|| anyhow::anyhow!("ring {}: no similarity to the joined rings", r + 1))?;
        let mut model = None;
        for _ in 0..3 {
            let (x, y): (Vec<V3>, Vec<V3>) = pairs.iter().zip(&inliers).filter(|(_, &k)| k).map(|(p, _)| *p).unzip();
            if x.len() < 12 {
                break;
            }
            model = similarity(&x, &y);
            let Some(m) = &model else { break };
            inliers = inliers_of(m);
            n = inliers.iter().filter(|&&k| k).count();
        }
        let Some((s, rotation, t)) = model.filter(|_| n >= 12 && n * 5 >= pairs.len()) else {
            bail!("ring {}: only {n} of {} matched points agree on one placement; the rings could not be joined", r + 1, pairs.len());
        };
        place(r, s, &rotation, t, &mut rotations, &mut translations, &mut world);
        placements.push(json!({"ring": r + 1, "linked_points": pairs.len(), "agreeing": n, "scale": s}));
    }
    let normalised: Vec<Vec<P2>> = matches.keypoints.iter().map(|list| list.iter().map(|p| camera.undistort(*p)).collect()).collect();
    let mut resected = Vec::new();
    // Photos with a pose in the joined frame; the others are registered in rounds, each against the tracks
    // of all photos registered so far (so also against neighbours of their own turn registered before).
    let mut known: Vec<bool> = (0..count).map(|v| world[v / per].is_some()).collect();
    let mut round = 0;
    while known.iter().any(|k| !k) {
        round += 1;
        let placed: Vec<usize> = (0..count).filter(|&v| known[v]).collect();
        let mut index = vec![usize::MAX; count];
        for (k, &v) in placed.iter().enumerate() {
            index[v] = k;
        }
        let subset = Matches {
            keypoints: placed.iter().map(|&v| matches.keypoints[v].clone()).collect(),
            pairs: matches
                .pairs
                .iter()
                .filter(|(i, j, _)| index[*i] != usize::MAX && index[*j] != usize::MAX)
                .map(|(i, j, l)| (index[*i], index[*j], l.clone()))
                .collect(),
        };
        let subset_normalised: Vec<Vec<P2>> = placed.iter().map(|&v| normalised[v].clone()).collect();
        let mut subset_rotations: Vec<M3> = placed.iter().map(|&v| rotations[v]).collect();
        let mut subset_translations: Vec<V3> = placed.iter().map(|&v| translations[v]).collect();
        let (tracks, ..) = refine(&subset, &subset_normalised, &mut subset_rotations, &mut subset_translations, focal, &mut Vec::new())?;
        for (k, &v) in placed.iter().enumerate() {
            (rotations[v], translations[v]) = (subset_rotations[k], subset_translations[k]);
        }
        let mut point_at: HashMap<(usize, u32), usize> = HashMap::new();
        for &(view, point, feature) in &tracks.observations {
            point_at.insert((placed[view], feature), point);
        }
        let mut found = Vec::new();
        let mut failed = Vec::new();
        for u in (0..count).filter(|&v| !known[v]) {
            let mut seen: HashMap<u32, V3> = HashMap::new();
            let mut partners: HashMap<usize, usize> = HashMap::new();
            for (i, j, list) in &matches.pairs {
                for &(a, b) in list {
                    let (own, other) = if *i == u {
                        (a, (*j, b))
                    } else if *j == u {
                        (b, (*i, a))
                    } else {
                        continue;
                    };
                    if let Some(&point) = point_at.get(&other) {
                        seen.entry(own).or_insert(tracks.points[point]);
                        *partners.entry(other.0).or_default() += 1;
                    }
                }
            }
            // Start also from the registered cameras that share the most points with this photo.
            let mut partners: Vec<(usize, usize)> = partners.into_iter().collect();
            partners.sort_unstable_by_key(|&(v, n)| (std::cmp::Reverse(n), v));
            let starts: Vec<(M3, V3)> = partners.iter().take(3).map(|&(v, _)| (rotations[v], translations[v])).collect();
            // A fixed order, whatever the hash map did.
            let mut seen: Vec<(u32, V3)> = seen.into_iter().collect();
            seen.sort_unstable_by_key(|s| s.0);
            let seen: Vec<(P2, V3)> = seen.into_iter().map(|(f, x)| (normalised[u][f as usize], x)).collect();
            match resect(&seen, &starts, focal, &mut random).filter(|p| p.2 >= 20) {
                Some((r_u, t_u, agreeing)) => found.push((u, r_u, t_u, agreeing)),
                None => failed.push((u, seen.len())),
            }
        }
        if found.is_empty() {
            let (u, links) = failed[0];
            bail!(
                "turn {}: photo {} (and {} more) could not be registered against the other turns ({links} linked points)",
                u / per + 1,
                u % per + 1,
                failed.len() - 1
            );
        }
        let fewest = found.iter().map(|f| f.3).min();
        for (u, r_u, t_u, _) in &found {
            (rotations[*u], translations[*u]) = (*r_u, *t_u);
            known[*u] = true;
        }
        resected.push(json!({"round": round, "registered": found.len(), "fewest_agreeing_points": fewest, "left": failed.len()}));
    }
    // A resected turn rests on few points per photo, and a few of them land far off; a turn seen almost
    // along the axis also drifts in a free adjustment, its photos barely telling its radius from its height.
    // The turntable fixes what they may be: one camera, turned about the axis the plausible turns share.
    // Such a turn is put onto that motion and adjusted as it: its fixed camera and one angle per photo.
    let mut models = Vec::new();
    let mut turned = Vec::new();
    let placed: Vec<usize> = (0..rings.len()).filter(|&r| sound[r]).collect();
    if resected.iter().any(|r| r["registered"].as_u64().unwrap_or(0) > 0) && sound[0] {
        if let Some((axis, through)) = shared_axis(&placed, per, &rotations, &translations) {
            for r in (0..rings.len()).filter(|&r| !sound[r]) {
                let (rs, ts) = (&mut rotations[r * per..(r + 1) * per], &mut translations[r * per..(r + 1) * per]);
                if let Some((mut model, off)) = onto_turntable(axis, through, rs, ts) {
                    model.cameras = (r * per..(r + 1) * per).collect();
                    models.push(model);
                    turned.push(json!({"ring": r + 1, "photos_off_the_turntable_motion": off}));
                }
            }
        }
    }
    let (tracks, mut errors, rounds, first_tracks) = refine(matches, &normalised, &mut rotations, &mut translations, focal, &mut models)?;
    let mut sorted = errors.clone();
    sorted.sort_by(f64::total_cmp);
    let report = json!({
        "rings": rings.iter().map(|r| r.report.clone()).collect::<Vec<_>>(), "photos": count, "placements": placements,
        "sound_rings": sound, "resected": resected, "turned_onto_axis": turned,
        "first_tracks": {"verified_matches": first_tracks.0, "points": first_tracks.1, "observations": first_tracks.2},
        "adjustments": rounds, "points": tracks.points.len(), "observations": tracks.observations.len(),
        "reprojection_px": {"median": median_f64(&mut errors), "p95": crate::inputs::percentile_sorted_f64(&sorted, 95.0)},
    });
    Ok(Solved { rotations, translations, points: tracks.points, observations: tracks.observations, report })
}

/// The angle of `turn` about `axis`, assuming it is (close to) a rotation about it.
fn angle_about(axis: V3, turn: &M3) -> f64 {
    let across = unit(cross(axis, if axis[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
    let moved = apply(turn, across);
    dot(moved, cross(axis, across)).atan2(dot(moved, across))
}

/// The turntable axis in the joined frame (direction, a point on it), from the placed turns: the
/// object turns about it between any two photos of one turn, and their camera centres circle it.
fn shared_axis(placed: &[usize], per: usize, rotations: &[M3], translations: &[V3]) -> Option<(V3, V3)> {
    let mut sum = [0.0; 3];
    for &r in placed {
        let first = &rotations[r * per];
        for k in 1..per {
            // World to camera: R_k = R_fixed Turn_k, so first^T R_k turns about the axis.
            let w = super::geometry::rotation_vector(&mul(&transpose(first), &rotations[r * per + k]));
            if norm(w) < 20f64.to_radians() || norm(w) > 160f64.to_radians() {
                continue;
            }
            let w = unit(w);
            sum = if dot(w, sum) < 0.0 { sub(sum, w) } else { add(sum, w) };
        }
    }
    if norm(sum) < 1e-9 {
        return None;
    }
    let axis = unit(sum);
    // A point p on the axis: two photos j, k of one turn have centres with c_k - p = Q (c_j - p), Q the
    // turn between them (R_k^T R_j), so (I - Q) p = c_k - Q c_j; least squares over all pairs with the first photo,
    // pinned along the axis at the mean centre.
    let centre = |v: usize| scale(apply(&transpose(&rotations[v]), translations[v]), -1.0);
    let (mut normal, mut right) = (vec![vec![0.0; 3]; 3], vec![0.0; 3]);
    let mut mean = [0.0; 3];
    for &r in placed {
        let (j, first) = (r * per, &rotations[r * per]);
        for k in r * per..(r + 1) * per {
            mean = add(mean, scale(centre(k), 1.0 / (placed.len() * per) as f64));
            let q = mul(&transpose(&rotations[k]), first);
            let rows: [V3; 3] = std::array::from_fn(|a| std::array::from_fn(|b| if a == b { 1.0 } else { 0.0 } - q[a][b]));
            let value = sub(centre(k), apply(&q, centre(j)));
            for a in 0..3 {
                for b in 0..3 {
                    normal[a][b] += (0..3).map(|i| rows[i][a] * rows[i][b]).sum::<f64>();
                }
                right[a] += (0..3).map(|i| rows[i][a] * value[i]).sum::<f64>();
            }
        }
    }
    let along = dot(axis, mean) * per as f64;
    for a in 0..3 {
        for b in 0..3 {
            normal[a][b] += per as f64 * axis[a] * axis[b];
        }
        right[a] += along * axis[a];
    }
    let p = crate::photos::markers::linalg::solve(normal, right)?;
    Some((axis, [p[0], p[1], p[2]]))
}

/// Replaces the cameras of one turn by one fixed camera turned about `axis` through `through`: the
/// reference is the photo whose motion to the others is most nearly a turn about the axis; every photo's
/// angle comes from its own rotation, or for photos far from a turn about the axis (over 3 degrees), from
/// its neighbours. Returns the turn's motion (cameras numbered from 0) and how many photos were that far
/// off, or `None` when the turn was left as it was.
fn onto_turntable(axis: V3, through: V3, rotations: &mut [M3], translations: &mut [V3]) -> Option<(Turn, usize)> {
    let per = rotations.len();
    let off = |j: usize, k: usize| -> (f64, f64) {
        let turn = mul(&transpose(&rotations[j]), &rotations[k]);
        let angle = angle_about(axis, &turn);
        (angle, crate::photos::markers::linalg::rotation_angle_deg(&turn, &rodrigues(scale(axis, angle))))
    };
    let reference =
        (0..per).map(|j| (median_f64(&mut (0..per).map(|k| off(j, k).1).collect::<Vec<_>>()), j)).min_by(|a, b| a.0.total_cmp(&b.0))?.1;
    let found: Vec<(f64, f64)> = (0..per).map(|k| off(reference, k)).collect();
    let good: Vec<usize> = (0..per).filter(|&k| found[k].1 <= 3.0).collect();
    if good.len() * 2 < per {
        return None;
    }
    // Angles unwrapped along the capture order from the reference, the far-off ones interpolated.
    let tau = 2.0 * std::f64::consts::PI;
    let mut angles = vec![f64::NAN; per];
    for &k in &good {
        angles[k] = found[k].0;
    }
    let order: Vec<usize> = (0..per).map(|i| (reference + i) % per).collect();
    let mut last = 0.0;
    for &k in &order {
        if angles[k].is_finite() {
            angles[k] += tau * ((last - angles[k]) / tau).round();
            last = angles[k];
        }
    }
    for i in 0..per {
        let k = order[i];
        if angles[k].is_finite() {
            continue;
        }
        let before = (0..i).rev().map(|b| order[b]).find(|&b| angles[b].is_finite());
        let after = (i + 1..per).map(|a| order[a]).find(|&a| angles[a].is_finite());
        let position = |k: usize| (k + per - reference) % per;
        angles[k] = match (before, after) {
            (Some(b), Some(a)) => {
                let f = (position(k) - position(b)) as f64 / (position(a) - position(b)) as f64;
                angles[b] + f * (angles[a] - angles[b])
            }
            (Some(b), None) => angles[b] + (angles[b] - angles[reference]) / position(b).max(1) as f64 * (position(k) - position(b)) as f64,
            _ => return None,
        };
    }
    // The fixed camera: the reference's rotation, and the centre every photo puts it at once turned back.
    let fixed = rotations[reference];
    let mut parts: [Vec<f64>; 3] = Default::default();
    for &k in &good {
        let centre = scale(apply(&transpose(&rotations[k]), translations[k]), -1.0);
        let back = add(apply(&rodrigues(scale(axis, angles[k])), sub(centre, through)), through);
        for (a, part) in parts.iter_mut().enumerate() {
            part.push(back[a]);
        }
    }
    let centre = [median_f64(&mut parts[0]), median_f64(&mut parts[1]), median_f64(&mut parts[2])];
    let turn =
        Turn { cameras: (0..per).collect(), axis, through, rotation: fixed, translation: scale(apply(&fixed, centre), -1.0), angles };
    for k in 0..per {
        (rotations[k], translations[k]) = turn.pose(k);
    }
    Some((turn, per - good.len()))
}

/// The neutral solution of a solved capture; `names[i]` is the photo of camera `i`.
pub fn solution(lens: &Lens, names: &[String], matches: &Matches, solved: &Solved) -> Solution {
    let views = names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let (rotation, translation) = (solved.rotations[n], solved.translations[n]);
            let centre = scale(apply(&transpose(&rotation), translation), -1.0);
            View { id: n.to_string(), source: name.clone(), rotation, centre, translation }
        })
        .collect();
    let mut landmarks: Vec<Landmark> = solved.points.iter().map(|p| Landmark { position: *p, observations: Vec::new() }).collect();
    for &(view, point, feature) in &solved.observations {
        landmarks[point].observations.push((view, matches.keypoints[view][feature as usize]));
    }
    Solution { lens: *lens, views, unregistered: Vec::new(), landmarks, lens_locked: Some(true), scale: None, object_points: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::markers::linalg::rotation_angle_deg;

    #[test]
    fn malformed_match_indices_are_refused_before_indexing() {
        let valid = json!({"schema":"crisp3ds_turntable_matches_v1", "keypoints":[[[1.,2.]],[[3.,4.]]], "pairs":[{"first":0,"second":1,"matches":[[0,0]]}]});
        assert!(Matches::from_json(&valid).is_ok());
        for (field, value) in [("first", json!(-1)), ("second", json!(9)), ("matches", json!([[0, 8]]))] {
            let mut bad = valid.clone();
            bad["pairs"][0][field] = value;
            assert!(Matches::from_json(&bad).is_err());
        }
        let mut bad = valid;
        bad["pairs"][0].as_object_mut().unwrap().remove("first");
        assert!(Matches::from_json(&bad).is_err());
    }

    /// A turntable capture as matches: a blob of points about a tilted axis, uneven steps (one of them
    /// zero), a distorting lens, one wrong match in ten.
    fn capture(photos: usize, turn: f64) -> (Matches, Camera, Vec<M3>, Vec<V3>) {
        capture_tilt(photos, turn, 20.0)
    }

    fn capture_tilt(photos: usize, turn: f64, tilt: f64) -> (Matches, Camera, Vec<M3>, Vec<V3>) {
        let camera = Camera { fx: 2300.0, fy: 2302.0, cx: 870.0, cy: 560.0, k: [-0.1, 0.2, -0.5] };
        let axis = unit([0.02, tilt.to_radians().cos(), tilt.to_radians().sin()]);
        let centre = [0.01, 0.05, 1.0];
        let nominal = turn * 2.0 * std::f64::consts::PI / photos as f64;
        let mut steps: Vec<f64> = (0..photos).map(|i| nominal * (1.0 + 0.1 * ((i * 7) as f64).sin())).collect();
        steps[5] = 0.0;
        let (mut rotations, mut translations, mut total) = (Vec::new(), Vec::new(), 0.0);
        for step in &steps {
            let (r, t) = step_motion(axis, centre, total);
            rotations.push(r);
            translations.push(t);
            total += step;
        }
        let points: Vec<V3> = (0..400)
            .map(|k| {
                let f = k as f64;
                [0.01 + 0.07 * (f * 1.7).sin(), 0.05 + 0.08 * (f * 2.3).cos(), 1.0 + 0.07 * (f * 3.1).sin() * (f * 0.37).cos()]
            })
            .collect();
        let keypoints: Vec<Vec<P2>> = (0..photos)
            .map(|v| points.iter().map(|x| camera.project(add(apply(&rotations[v], *x), translations[v])).unwrap()).collect())
            .collect();
        let mut pairs = Vec::new();
        for (i, j) in crate::photos::turntable::matching::ring_pairs(photos, 3, turn < 0.99) {
            let list = (0..400u32).map(|k| if (k as usize + i).is_multiple_of(10) { (k, (k * 37 + 11) % 400) } else { (k, k) }).collect();
            pairs.push((i, j, list));
        }
        (Matches { keypoints, pairs }, camera, rotations, translations)
    }

    impl Solved {
        fn clone_parts(&self) -> Solved {
            Solved {
                rotations: self.rotations.clone(),
                translations: self.translations.clone(),
                points: self.points.clone(),
                observations: self.observations.clone(),
                report: self.report.clone(),
            }
        }
    }

    #[test]
    fn a_camera_is_resected_from_points_with_wrong_matches() {
        let rotation = rodrigues([0.3, -0.5, 0.2]);
        let translation = [0.1, -0.05, 2.0];
        let mut random = crate::photos::turntable::geometry::Random(7);
        let seen: Vec<(P2, V3)> = (0..200)
            .map(|k| {
                let f = k as f64;
                let x = [0.3 * (f * 1.3).sin(), 0.3 * (f * 2.1).cos(), 0.2 * (f * 0.7).sin()];
                let q = add(apply(&rotation, x), translation);
                let noise = [1e-4 * (f * 5.1).sin(), 1e-4 * (f * 3.7).cos()];
                // One in four is a wrong match.
                let p = if k % 4 == 0 {
                    [0.1 * (f * 9.9).cos(), 0.1 * (f * 4.4).sin()]
                } else {
                    [q[0] / q[2] + noise[0], q[1] / q[2] + noise[1]]
                };
                (p, x)
            })
            .collect();
        let (r, t, agreeing) = resect(&seen, &[], 2000.0, &mut random).unwrap();
        assert!((150..=152).contains(&agreeing), "{agreeing}");
        assert!(rotation_angle_deg(&r, &rotation) < 0.01);
        assert!(norm(sub(t, translation)) < 1e-3);
        assert!(resect(&seen[..10], &[], 2000.0, &mut random).is_none());
        // Points on one plane: the linear transform cannot tell, a nearby start can.
        let flat: Vec<(P2, V3)> = seen
            .iter()
            .map(|&(_, x)| {
                let x = [x[0], x[1], 0.0];
                let q = add(apply(&rotation, x), translation);
                ([q[0] / q[2], q[1] / q[2]], x)
            })
            .collect();
        let near = (mul(&rodrigues([0.15, 0.1, -0.1]), &rotation), add(translation, [0.1, 0.05, 0.2]));
        let (r, t, agreeing) = resect(&flat, &[near], 2000.0, &mut random).unwrap();
        assert_eq!(agreeing, 200);
        assert!(rotation_angle_deg(&r, &rotation) < 0.01 && norm(sub(t, translation)) < 1e-3);
    }

    #[test]
    fn a_turn_is_put_back_onto_the_turntable_motion() {
        let (_, _, rotations, translations) = capture_tilt(24, 1.0, 20.0);
        let centre = |r: &M3, t: V3| scale(apply(&transpose(r), t), -1.0);
        let (axis, through) = shared_axis(&[0], 24, &rotations, &translations).unwrap();
        // True cameras stay where they are.
        let (mut r, mut t) = (rotations.clone(), translations.clone());
        assert_eq!(onto_turntable(axis, through, &mut r, &mut t).map(|m| m.1), Some(0));
        for k in 0..24 {
            assert!(
                rotation_angle_deg(&r[k], &rotations[k]) < 1e-4
                    && norm(sub(centre(&r[k], t[k]), centre(&rotations[k], translations[k]))) < 1e-7
            );
        }
        // Three photos far off, the others slightly: all of them come back close to the truth.
        for k in 0..24 {
            let (wrong, shift) = if [3, 4, 17].contains(&k) { (12.0f64, 0.3) } else { (0.2, 0.004) };
            let w = [(k as f64 * 1.3).sin(), (k as f64 * 0.7).cos(), 0.5];
            r[k] = mul(&rodrigues(scale(unit(w), wrong.to_radians())), &rotations[k]);
            t[k] = add(translations[k], scale(unit([w[2], w[0], w[1]]), shift));
        }
        assert_eq!(onto_turntable(axis, through, &mut r, &mut t).map(|m| m.1), Some(3));
        for k in 0..24 {
            assert!(rotation_angle_deg(&r[k], &rotations[k]) < 1.0, "photo {k}");
            assert!(norm(sub(centre(&r[k], t[k]), centre(&rotations[k], translations[k]))) < 0.03, "photo {k}");
        }
    }

    #[test]
    fn two_turns_at_different_heights_are_joined_into_one_frame() {
        // Ring B: the same turntable seen by a second camera raised by 25 degrees about the object.
        let (a, camera, rotations, translations) = capture_tilt(24, 1.0, 20.0);
        let raise = rodrigues([25f64.to_radians(), 0.0, 0.0]);
        let centre = [0.01, 0.05, 1.0];
        let (rb, tb): (Vec<M3>, Vec<V3>) =
            rotations.iter().zip(&translations).map(|(r, t)| (mul(&raise, r), add(apply(&raise, sub(*t, centre)), centre))).unzip();
        let points: Vec<V3> = (0..400)
            .map(|k| {
                let f = k as f64;
                [0.01 + 0.07 * (f * 1.7).sin(), 0.05 + 0.08 * (f * 2.3).cos(), 1.0 + 0.07 * (f * 3.1).sin() * (f * 0.37).cos()]
            })
            .collect();
        let b_points: Vec<Vec<P2>> =
            (0..24).map(|v| points.iter().map(|x| camera.project(add(apply(&rb[v], *x), tb[v])).unwrap()).collect()).collect();
        let mut keypoints = a.keypoints.clone();
        keypoints.extend(b_points);
        let mut pairs = a.pairs.clone();
        pairs.extend(a.pairs.iter().map(|(i, j, list)| (i + 24, j + 24, list.clone())));
        for k in 0..24 {
            for d in [23, 0, 1] {
                // One wrong match in ten across the turns too.
                let list =
                    (0..400u32).map(|q| if (q as usize + k).is_multiple_of(10) { (q, (q * 53 + 7) % 400) } else { (q, q) }).collect();
                pairs.push((k, 24 + (k + d) % 24, list));
            }
        }
        let all = Matches { keypoints, pairs };
        let settings = Settings { open_turn: false, wide_axis: true, planar: None };
        let ring = |r: usize| Matches {
            keypoints: all.keypoints[r * 24..(r + 1) * 24].to_vec(),
            pairs: all
                .pairs
                .iter()
                .filter(|(i, j, _)| i / 24 == r && j / 24 == r)
                .map(|(i, j, l)| (i - r * 24, j - r * 24, l.clone()))
                .collect(),
        };
        let solved: Vec<Solved> = (0..2).map(|r| solve(&ring(r), &camera, &settings).unwrap()).collect();
        let joined = join(&all, &camera, &solved).unwrap();
        assert!(joined.report["reprojection_px"]["median"].as_f64().unwrap() < 0.01, "{}", joined.report);
        // Every camera relative to the first, against the truth; translations up to the free scale.
        let truth_r: Vec<M3> = rotations.iter().chain(&rb).copied().collect();
        let truth_t: Vec<V3> = translations.iter().chain(&tb).copied().collect();
        let relative = |r: &[M3], t: &[V3], v: usize| {
            let turn = mul(&r[v], &transpose(&r[0]));
            (turn, sub(t[v], apply(&turn, t[0])))
        };
        let size = |t: &[V3]| (0..48).map(|v| norm(relative(&truth_r, t, v).1)).sum::<f64>();
        let factor = size(&truth_t) / (0..48).map(|v| norm(relative(&joined.rotations, &joined.translations, v).1)).sum::<f64>();
        for v in 0..48 {
            let (r_true, t_true) = relative(&truth_r, &truth_t, v);
            let (r_found, t_found) = relative(&joined.rotations, &joined.translations, v);
            assert!(rotation_angle_deg(&r_found, &r_true) < 0.01, "view {v}");
            assert!(norm(sub(scale(t_found, factor), t_true)) < 1e-3, "view {v}");
        }
        // Points of both turns became shared tracks.
        assert!(joined.observations.iter().any(|o| o.0 < 24) && joined.observations.iter().any(|o| o.0 >= 24));
        // A turn whose own solution is no ring is registered photo by photo against the other turn.
        let mut broken = solved[1].clone_parts();
        for (v, t) in broken.translations.iter_mut().enumerate() {
            *t = scale(*t, 1.0 + 0.3 * (v as f64 * 0.7).sin());
        }
        assert!(!sound_ring(&broken) && sound_ring(&solved[0]));
        let rescued = join(&all, &camera, &[solved[0].clone_parts(), broken]).unwrap();
        assert_eq!(
            rescued.report["resected"].as_array().unwrap().iter().map(|r| r["registered"].as_u64().unwrap()).sum::<u64>(),
            24,
            "{}",
            rescued.report
        );
        let factor = size(&truth_t) / (0..48).map(|v| norm(relative(&rescued.rotations, &rescued.translations, v).1)).sum::<f64>();
        for v in 0..48 {
            let (r_true, t_true) = relative(&truth_r, &truth_t, v);
            let (r_found, t_found) = relative(&rescued.rotations, &rescued.translations, v);
            assert!(rotation_angle_deg(&r_found, &r_true) < 0.01, "view {v}");
            assert!(norm(sub(scale(t_found, factor), t_true)) < 1e-3, "view {v}");
        }
        // So is a turn without any solution of its own.
        let unsolved = Solved::unsolved(24, "no points");
        assert!(!sound_ring(&unsolved));
        let rescued = join(&all, &camera, &[solved[0].clone_parts(), unsolved]).unwrap();
        let factor = size(&truth_t) / (0..48).map(|v| norm(relative(&rescued.rotations, &rescued.translations, v).1)).sum::<f64>();
        for v in 0..48 {
            let (r_true, t_true) = relative(&truth_r, &truth_t, v);
            let (r_found, t_found) = relative(&rescued.rotations, &rescued.translations, v);
            assert!(rotation_angle_deg(&r_found, &r_true) < 0.01, "view {v}");
            assert!(norm(sub(scale(t_found, factor), t_true)) < 1e-3, "view {v}");
        }
        // A ring without links to the others is refused, not placed at random.
        let mut alone = all.clone();
        alone.pairs.retain(|(i, j, _)| i / 24 == j / 24);
        assert!(join(&alone, &camera, &solved).is_err());
    }

    #[test]
    fn steep_capture_recovers_without_relaxing_closure() {
        let (matches, camera, truth, _) = capture_tilt(24, 1.0, 65.0);
        let solved = solve(&matches, &camera, &Settings { open_turn: false, wide_axis: true, planar: None }).unwrap();
        assert!(solved.report["reprojection_px"]["median"].as_f64().unwrap() < 0.01);
        for (actual, expected) in solved.rotations.iter().zip(truth) {
            assert!(rotation_angle_deg(actual, &expected) < 0.01);
        }
    }

    #[test]
    fn turntable_capture_is_recovered_from_its_matches() {
        for (turn, open) in [(1.0, false), (0.6, true)] {
            let (matches, camera, rotations, translations) = capture(24, turn);
            let solved = solve(&matches, &camera, &Settings { open_turn: open, wide_axis: false, planar: None }).unwrap();
            assert_eq!(solved.report["closed_to_full_turn"], !open);
            assert!(solved.points.len() > 300 && solved.report["reprojection_px"]["median"].as_f64().unwrap() < 0.01, "{}", solved.report);
            // Both in the first camera's frame; the scale is free.
            let scale_of = |t: &[V3]| t.iter().map(|v| norm(*v)).sum::<f64>();
            let factor = scale_of(&translations) / scale_of(&solved.translations);
            for v in 0..24 {
                assert!(rotation_angle_deg(&solved.rotations[v], &rotations[v]) < 0.01, "view {v}");
                assert!(norm(sub(scale(solved.translations[v], factor), translations[v])) < 1e-3, "view {v}");
            }
            // The step that did not move is found as such.
            assert!(norm(sub(solved.translations[6], solved.translations[5])) < 1e-4);
        }
        let (matches, camera, _, _) = capture(6, 1.0);
        assert!(solve(&matches, &camera, &Settings { open_turn: false, wide_axis: false, planar: None }).is_err());
    }
}
