//! Experimental calibrated structure from motion without a capture-order or turntable model.
//! Starts from a non-degenerate two-view reconstruction, adds cameras by robust resection,
//! and triangulates/adjusts the connected tracks. No angles or poses are supplied.
//! This is a library experiment, not yet an advertised camera provider.

use super::markers::linalg::{add, apply, dot, mul, norm, rodrigues, solve, transpose, unit, Camera, M3, V3};
use super::solution::{Lens, Solution};
use super::turntable::{adjust, geometry, solver};
use anyhow::{bail, Context};
use serde_json::{json, Value};
use solver::Matches;
use std::collections::{BTreeMap, HashMap};

type Pose = (M3, V3);
const IDENTITY: M3 = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

/// A connected reconstruction; missing views stay explicitly unregistered.
#[derive(Debug)]
pub struct Recovered {
    pub solution: Solution,
    pub report: Value,
}

/// Alternative resection paths; retain the existing linear path for comparison.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Resection {
    #[default]
    Linear,
    P3p,
    P3pConsensus,
}

/// Alternative calibrated initial-pose estimators; eight-point remains default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Initializer {
    #[default]
    EightPoint,
    FivePoint,
}

/// Keep the strongest subtrack, or peel additional independently witnessed tracks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrackSelection {
    #[default]
    Robust,
    Split,
}

/// Score-only repeats strong local seeds; diversity spends effort across the capture graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SeedSelection {
    #[default]
    Score,
    Diverse,
}
#[derive(Debug, Clone, Copy)]
pub struct RecoveryOptions {
    pub initializer: Initializer,
    pub resection: Resection,
    pub seed_selection: SeedSelection,
    pub track_selection: TrackSelection,
    pub maximum_seed_attempts: usize,
    /// Pixel uncertainty multiplier for correspondence gates; leaves calibration/coordinates intact.
    pub measurement_error_scale: f64,
}
impl Default for RecoveryOptions {
    fn default() -> Self {
        Self {
            initializer: Initializer::EightPoint,
            resection: Resection::Linear,
            seed_selection: SeedSelection::Score,
            track_selection: TrackSelection::Robust,
            maximum_seed_attempts: 32,
            measurement_error_scale: 1.,
        }
    }
}
type Seed = (usize, usize, Pose, f64, usize);
fn select_seeds(mut seeds: Vec<Seed>, selection: SeedSelection, budget: usize, count: usize) -> Vec<Seed> {
    if selection == SeedSelection::Score {
        return seeds.into_iter().take(budget).collect();
    }
    let mut visits = vec![0usize; count];
    let mut selected = Vec::new();
    for _ in 0..budget.min(seeds.len()) {
        let mut best = 0;
        let mut score = f64::NEG_INFINITY;
        for (i, seed) in seeds.iter().enumerate() {
            let penalty = (1 + visits[seed.0] + visits[seed.1]) as f64;
            let value = seed.3 / (penalty * penalty);
            if value > score {
                best = i;
                score = value;
            }
        }
        let seed = seeds.remove(best);
        visits[seed.0] += 1;
        visits[seed.1] += 1;
        selected.push(seed);
    }
    selected
}

fn subset(matches: &Matches, placed: &[usize]) -> Matches {
    let mut index = vec![usize::MAX; matches.keypoints.len()];
    for (i, &v) in placed.iter().enumerate() {
        index[v] = i;
    }
    Matches {
        keypoints: placed.iter().map(|&v| matches.keypoints[v].clone()).collect(),
        pairs: matches
            .pairs
            .iter()
            .filter(|(a, b, _)| index[*a] != usize::MAX && index[*b] != usize::MAX)
            .map(|(a, b, list)| (index[*a], index[*b], list.clone()))
            .collect(),
    }
}

fn parallax(r0: &M3, a: [f64; 2], r1: &M3, b: [f64; 2]) -> f64 {
    let x = unit(apply(&transpose(r0), [a[0], a[1], 1.]));
    let y = unit(apply(&transpose(r1), [b[0], b[1], 1.]));
    dot(x, y).clamp(-1., 1.).acos().to_degrees()
}

// A planar match or a pure rotation can look like a strong essential pair. It cannot
// safely initialize depth. Require correspondences that a single homography does not explain.
fn homography_support(a: &[[f64; 2]], b: &[[f64; 2]], inliers: &[bool], bound: f64, random: &mut geometry::Random) -> usize {
    let indices: Vec<_> = (0..a.len()).filter(|&k| inliers[k]).collect();
    if indices.len() < 4 {
        return indices.len();
    }
    let mut best = 0;
    for _ in 0..128 {
        let mut selected = Vec::new();
        while selected.len() < 4 {
            let k = random.below(indices.len());
            if !selected.contains(&k) {
                selected.push(k);
            }
        }
        let source: Vec<_> = selected.iter().map(|&k| a[indices[k]]).collect();
        let target: Vec<_> = selected.iter().map(|&k| b[indices[k]]).collect();
        let Some(h) = super::markers::linalg::homography(&source, &target) else { continue };
        let count = indices
            .iter()
            .filter(|&&k| {
                let q = apply(&h, [a[k][0], a[k][1], 1.]);
                q[2].abs() > 1e-12 && (q[0] / q[2] - b[k][0]).hypot(q[1] / q[2] - b[k][1]) < bound
            })
            .count();
        best = best.max(count);
        if best == indices.len() {
            break;
        }
    }
    best
}

fn relative_residual(r: &M3, t: V3, a: &[[f64; 2]], b: &[[f64; 2]], selected: &[usize], f: f64) -> Vec<f64> {
    let e = geometry::essential_of(r, t);
    let et = transpose(&e);
    selected
        .iter()
        .map(|&k| {
            let x = [a[k][0], a[k][1], 1.];
            let y = [b[k][0], b[k][1], 1.];
            let ex = apply(&e, x);
            let ey = apply(&et, y);
            dot(y, ex) / (ex[0] * ex[0] + ex[1] * ex[1] + ey[0] * ey[0] + ey[1] * ey[1] + 1e-18).sqrt() * f
        })
        .collect()
}
#[allow(clippy::needless_range_loop)]
fn fit_relative(mut r: M3, mut t: V3, a: &[[f64; 2]], b: &[[f64; 2]], selected: &[usize], f: f64) -> (M3, V3) {
    t = unit(t);
    let cost = |v: &[f64]| v.iter().map(|e| (1. + e * e / 4.).ln()).sum::<f64>();
    let mut damping = 1e-3;
    for _ in 0..80 {
        let base = relative_residual(&r, t, a, b, selected, f);
        let old = cost(&base);
        let moved = |delta: &[f64]| {
            let mut d = delta.to_vec();
            let rw = norm([d[0], d[1], d[2]]);
            let tw = norm([d[3], d[4], d[5]]);
            let factor = (0.15 / rw.max(1e-12)).min(0.4 / tw.max(1e-12)).min(1.);
            for x in &mut d {
                *x *= factor;
            }
            (mul(&rodrigues([d[0], d[1], d[2]]), &r), unit(add(t, [d[3], d[4], d[5]])))
        };
        let columns: Vec<Vec<f64>> = (0..6)
            .map(|k| {
                let mut d = [0.; 6];
                d[k] = 1e-6;
                let (r1, t1) = moved(&d);
                let v = relative_residual(&r1, t1, a, b, selected, f);
                v.iter().zip(&base).map(|(x, y)| (x - y) / 1e-6).collect()
            })
            .collect();
        let mut h = vec![vec![0.; 6]; 6];
        let mut g = vec![0.; 6];
        for k in 0..base.len() {
            let w = 1. / (1. + base[k] * base[k] / 4.);
            for i in 0..6 {
                g[i] -= w * columns[i][k] * base[k];
                for j in 0..6 {
                    h[i][j] += w * columns[i][k] * columns[j][k];
                }
            }
        }
        for i in 0..6 {
            h[i][i] += damping * h[i][i].max(1e-6);
        }
        let Some(delta) = solve(h, g) else { break };
        let (r1, t1) = moved(&delta);
        let new = cost(&relative_residual(&r1, t1, a, b, selected, f));
        if new < old {
            r = r1;
            t = t1;
            damping = (damping * 0.3).max(1e-9);
            if old - new < 1e-8 {
                break;
            }
        } else {
            damping = (damping * 10.).min(1e12);
        }
    }
    (r, t)
}

fn prune(tracks: &mut solver::Tracks, normalised: &[Vec<[f64; 2]>], rotations: &[M3]) {
    let mut first: Vec<Option<(usize, u32)>> = vec![None; tracks.points.len()];
    let mut angle = vec![0f64; tracks.points.len()];
    for &(v, p, f) in &tracks.observations {
        if let Some((a, b)) = first[p] {
            angle[p] = angle[p].max(parallax(&rotations[a], normalised[a][b as usize], &rotations[v], normalised[v][f as usize]));
        } else {
            first[p] = Some((v, f));
        }
    }
    let mut index = vec![usize::MAX; tracks.points.len()];
    let mut points = Vec::new();
    for (p, x) in tracks.points.iter().enumerate() {
        if angle[p] >= 1. && x.iter().all(|v| v.is_finite()) {
            index[p] = points.len();
            points.push(*x);
        }
    }
    tracks.observations =
        tracks.observations.iter().filter(|(_, p, _)| index[*p] != usize::MAX).map(|&(v, p, f)| (v, index[p], f)).collect();
    tracks.points = points;
}

fn refresh(
    matches: &Matches,
    camera: &Camera,
    placed: &[usize],
    poses: &mut [Option<Pose>],
    bundle: bool,
    selection: TrackSelection,
    focal: f64,
) -> anyhow::Result<(Matches, solver::Tracks)> {
    let own = subset(matches, placed);
    let normalised: Vec<Vec<_>> = own.keypoints.iter().map(|list| list.iter().map(|p| camera.undistort(*p)).collect()).collect();
    let mut rotations: Vec<_> = placed.iter().map(|&v| poses[v].unwrap().0).collect();
    let mut translations: Vec<_> = placed.iter().map(|&v| poses[v].unwrap().1).collect();
    let build = |rotations: &[M3], translations: &[V3]| match selection {
        TrackSelection::Robust => solver::build_robust(&own, &normalised, rotations, translations, focal, 2., 2),
        TrackSelection::Split => solver::build_split(&own, &normalised, rotations, translations, focal, 2., 2),
    };
    let mut tracks = build(&rotations, &translations);
    prune(&mut tracks, &normalised, &rotations);
    if tracks.points.len() < 20 {
        bail!("incremental cameras: fewer than 20 triangulated tracks with one degree of parallax");
    }
    // The existing dense Schur solver is deliberately bounded here. Larger experiments
    // still resect cameras, but need sparse/local adjustment before provider adoption.
    if bundle && placed.len() <= 128 {
        let obs: Vec<_> = tracks
            .observations
            .iter()
            .map(|&(v, p, f)| adjust::Observation { camera: v, point: p, target: normalised[v][f as usize] })
            .collect();
        adjust::adjust(&mut rotations, &mut translations, &mut tracks.points, &obs, focal, 10);
        for (i, &v) in placed.iter().enumerate() {
            poses[v] = Some((rotations[i], translations[i]));
        }
        tracks = build(&rotations, &translations);
        prune(&mut tracks, &normalised, &rotations);
    }
    Ok((own, tracks))
}

/// Uses only pixel correspondences and one calibrated lens. File order, regular spacing,
/// a common orbit plane, a full turn and a known rotation axis are never required.
/// `check` is called between bounded geometry operations for cancellation/progress.
pub fn recover(
    matches: &Matches,
    lens: &Lens,
    names: &[String],
    check: &mut dyn FnMut(usize, usize) -> anyhow::Result<()>,
) -> anyhow::Result<Recovered> {
    recover_with_resection(matches, lens, names, Resection::Linear, check)
}

/// Explicit experimental path selection. `recover` retains its previous solver.
pub fn recover_with_resection(
    matches: &Matches,
    lens: &Lens,
    names: &[String],
    resection: Resection,
    check: &mut dyn FnMut(usize, usize) -> anyhow::Result<()>,
) -> anyhow::Result<Recovered> {
    recover_with_solvers(matches, lens, names, Initializer::EightPoint, resection, check)
}

/// Independent explicit choices for initial relative pose and later resection.
pub fn recover_with_solvers(
    matches: &Matches,
    lens: &Lens,
    names: &[String],
    initializer: Initializer,
    resection: Resection,
    check: &mut dyn FnMut(usize, usize) -> anyhow::Result<()>,
) -> anyhow::Result<Recovered> {
    recover_with_options(matches, lens, names, RecoveryOptions { initializer, resection, ..RecoveryOptions::default() }, check)
}
/// Bounded search alternatives. File order and fixed angular spacing are not priors.
pub fn recover_with_options(
    matches: &Matches,
    lens: &Lens,
    names: &[String],
    options: RecoveryOptions,
    check: &mut dyn FnMut(usize, usize) -> anyhow::Result<()>,
) -> anyhow::Result<Recovered> {
    if !options.measurement_error_scale.is_finite() || !(0.125..=16.).contains(&options.measurement_error_scale) {
        bail!("incremental cameras: measurement error scale must be finite and between 0.125 and 16");
    }
    if !(1..=256).contains(&options.maximum_seed_attempts) {
        bail!("incremental cameras: seed budget must be between 1 and 256");
    }
    let (initializer, resection) = (options.initializer, options.resection);
    let count = matches.keypoints.len();
    if !(3..=super::MOST_PHOTOS).contains(&count) || names.len() != count {
        bail!("incremental cameras: expected 3 to {} named views", super::MOST_PHOTOS);
    }
    if matches.keypoints.iter().flatten().flatten().any(|x| !x.is_finite())
        || matches.pairs.iter().any(|(a, b, list)| {
            *a >= count
                || *b >= count
                || a == b
                || list.iter().any(|&(x, y)| x as usize >= matches.keypoints[*a].len() || y as usize >= matches.keypoints[*b].len())
        })
    {
        bail!("incremental cameras: invalid feature coordinates or pair indices");
    }
    if lens.width == 0
        || lens.height == 0
        || lens.pixels.iter().chain(lens.k.iter()).any(|v| !v.is_finite())
        || lens.pixels[0] <= 0.
        || lens.pixels[1] <= 0.
    {
        bail!("incremental cameras: invalid lens");
    }
    let camera = Camera::from_lens(lens);
    let focal = (camera.fx + camera.fy) / (2. * options.measurement_error_scale);
    if !focal.is_finite() || focal <= 0. {
        bail!("incremental cameras: invalid lens");
    }
    let normalised: Vec<Vec<_>> = matches.keypoints.iter().map(|list| list.iter().map(|p| camera.undistort(*p)).collect()).collect();
    let mut ranked: Vec<_> = matches.pairs.iter().filter(|(_, _, l)| l.len() >= 30).collect();
    ranked.sort_by_key(|(a, b, list)| (std::cmp::Reverse(list.len()), *a, *b));
    let mut random = geometry::Random(73129);
    let mut seeds = Vec::new();
    for (rank, &(i, j, list)) in ranked.iter().take(512).enumerate() {
        check(0, count)?;
        let (a, b): (Vec<_>, Vec<_>) = list.iter().map(|&(x, y)| (normalised[*i][x as usize], normalised[*j][y as usize])).unzip();
        let estimate = match initializer {
            Initializer::EightPoint => geometry::essential_ransac(&a, &b, 2. / focal, &mut random),
            Initializer::FivePoint => super::five_point::ransac(&a, &b, 2. / focal, &mut random),
        };
        let Some((e, flags)) = estimate else { continue };
        let Some((r, t)) = geometry::recover_pose(&e, &a, &b, &flags) else { continue };
        let selected: Vec<_> = (0..a.len()).filter(|&k| flags[k]).collect();
        if selected.len() < 30 {
            continue;
        }
        let mut variants = vec![(r, t)];
        // Small objects occupy a narrow bearing range. Linear essential projection can
        // favour a nearly pure forward motion even when the real motion has parallax.
        // Explore translation directions on the calibrated pose manifold, then let a
        // third view decide; no object axis or angular spacing is prescribed.
        if rank < 192 {
            let mut directions = vec![t];
            for x in -1..=1 {
                for y in -1..=1 {
                    for z in -1..=1 {
                        if (x, y, z) > (0, 0, 0) {
                            directions.push([x as f64, y as f64, z as f64]);
                        }
                    }
                }
            }
            for direction in directions {
                let (rr, tt) = fit_relative(r, direction, &a, &b, &selected, focal);
                let ee = geometry::essential_of(&rr, tt);
                let more: Vec<_> = (0..a.len()).filter(|&k| geometry::sampson(&ee, a[k], b[k]) * focal < 2.).collect();
                if more.len() < 30 {
                    continue;
                }
                let (rr, tt) = fit_relative(rr, tt, &a, &b, &more, focal);
                if variants.iter().any(|(old_r, old_t)| {
                    norm(geometry::rotation_vector(&mul(&rr, &transpose(old_r)))) < 0.02 && dot(unit(tt), unit(*old_t)).abs() > 0.998
                }) {
                    continue;
                }
                variants.push((rr, tt));
            }
        }
        let mut supported = Vec::new();
        for (r, t) in variants {
            let e = geometry::essential_of(&r, t);
            let inliers: Vec<_> = (0..a.len()).map(|k| geometry::sampson(&e, a[k], b[k]) * focal < 2.).collect();
            let total = inliers.iter().filter(|&&v| v).count();
            let Some((r, t)) = geometry::recover_pose(&e, &a, &b, &inliers) else { continue };
            let mut angles = Vec::new();
            for k in (0..a.len()).filter(|&k| inliers[k]) {
                let Some(x) = geometry::triangulate(&[(IDENTITY, [0.; 3], a[k]), (r, t, b[k])]) else { continue };
                if x[2] > 0. && add(apply(&r, x), t)[2] > 0. {
                    angles.push(parallax(&IDENTITY, a[k], &r, b[k]));
                }
            }
            if angles.len() < 30 || angles.len() * 10 < total * 8 {
                continue;
            }
            angles.sort_by(f64::total_cmp);
            let median = angles[angles.len() / 2];
            if median < 2. {
                continue;
            }
            supported.push((r, t, inliers, angles.len(), median, total as f64 * median.min(15.)));
        }
        supported.sort_by(|a, b| b.5.total_cmp(&a.5));
        let mut kept = 0;
        for (r, t, inliers, n, median, _) in supported.into_iter().take(3) {
            let total = inliers.iter().filter(|&&v| v).count();
            let planar = homography_support(&a, &b, &inliers, 2. / focal, &mut random);
            if planar * 10 >= total * 9 || total.saturating_sub(planar) < 12 {
                continue;
            }
            seeds.push((*i, *j, (r, t), total.saturating_sub(planar) as f64 * median.min(15.), n));
            kept += 1;
            if kept == 2 {
                break;
            }
        }
    }
    seeds.sort_by(|a, b| b.3.total_cmp(&a.3).then_with(|| (a.0, a.1).cmp(&(b.0, b.1))));
    let seed_candidates = seeds.len();
    let seeds = select_seeds(seeds, options.seed_selection, options.maximum_seed_attempts, count);
    let mut best: Option<Recovered> = None;
    let mut attempts = Vec::new();
    'seeds: for (a, b, pose, _, seed_points) in seeds {
        let mut poses = vec![None; count];
        poses[a] = Some((IDENTITY, [0.; 3]));
        poses[b] = Some(pose);
        let mut placed = vec![a, b];
        let mut rounds = Vec::new();
        loop {
            check(placed.len(), count)?;
            let Ok((_, tracks)) =
                refresh(matches, &camera, &placed, &mut poses, placed.len() <= 12 || placed.len() % 4 == 0, options.track_selection, focal)
            else {
                attempts.push(json!({"seed":[a,b],"registered":placed.len(),"failed":"insufficient consistent tracks"}));
                continue 'seeds;
            };
            let mut point_at = HashMap::new();
            for &(v, p, f) in &tracks.observations {
                point_at.insert((placed[v], f), p);
            }
            let mut found: Option<(usize, Pose, usize, usize)> = None;
            for u in (0..count).filter(|&v| poses[v].is_none()) {
                check(placed.len(), count)?;
                let mut seen: BTreeMap<u32, Option<usize>> = BTreeMap::new();
                let mut candidates: BTreeMap<(u32, usize), super::registration::Link> = BTreeMap::new();
                let mut partners: BTreeMap<usize, usize> = BTreeMap::new();
                for (i, j, list) in &matches.pairs {
                    if *i != u && *j != u {
                        continue;
                    }
                    for &(x, y) in list {
                        let (own, other) = if *i == u { (x, (*j, y)) } else { (y, (*i, x)) };
                        if let Some(&point) = point_at.get(&other) {
                            if resection == Resection::P3pConsensus {
                                candidates.entry((own, point)).or_insert(super::registration::Link {
                                    feature: own,
                                    point,
                                    bearing: normalised[u][own as usize],
                                    position: tracks.points[point],
                                });
                            }
                            seen.entry(own)
                                .and_modify(|old| {
                                    if *old != Some(point) {
                                        *old = None;
                                    }
                                })
                                .or_insert(Some(point));
                            *partners.entry(other.0).or_default() += 1;
                        }
                    }
                }
                let links: Vec<_> =
                    seen.into_iter().filter_map(|(f, p)| p.map(|p| (normalised[u][f as usize], tracks.points[p]))).collect();
                let linked = if resection == Resection::P3pConsensus {
                    candidates.keys().map(|(f, _)| *f).collect::<std::collections::BTreeSet<_>>().len()
                } else {
                    links.len()
                };
                if linked < 20 {
                    continue;
                }
                let mut partners: Vec<_> = partners.into_iter().collect();
                partners.sort_by_key(|&(v, n)| (std::cmp::Reverse(n), v));
                let starts: Vec<_> = partners.iter().take(3).filter_map(|&(v, _)| poses[v]).collect();
                let candidate = match resection {
                    Resection::Linear => solver::resect(&links, &starts, focal, &mut random),
                    Resection::P3p => solver::resect_general(&links, &starts, focal, &mut random),
                    Resection::P3pConsensus => {
                        super::registration::recover(&candidates.into_values().collect::<Vec<_>>(), &starts, focal, &mut random)
                    }
                };
                if let Some((r, t, n)) = candidate {
                    if n >= 20 && n * 5 >= linked * 2 && found.as_ref().is_none_or(|v| n > v.2) {
                        found = Some((u, (r, t), n, linked));
                    }
                }
            }
            match found {
                Some((u, p, inliers, linked)) => {
                    poses[u] = Some(p);
                    placed.push(u);
                    rounds.push(json!({"view":u,"inliers":inliers,"linked":linked}));
                }
                None => {
                    if placed.len() < 3 {
                        attempts.push(json!({"seed":[a,b],"registered":2,"failed":"no third view"}));
                        continue 'seeds;
                    }
                    let (own, tracks) = refresh(matches, &camera, &placed, &mut poses, true, options.track_selection, focal)?;
                    let rotations = placed.iter().map(|&v| poses[v].unwrap().0).collect();
                    let translations = placed.iter().map(|&v| poses[v].unwrap().1).collect();
                    let report = json!({"method":"experimental incremental SfM","measurement_error_scale":options.measurement_error_scale,"track_selection":match options.track_selection {TrackSelection::Robust=>"robust",TrackSelection::Split=>"split"},"seed_selection":match options.seed_selection {SeedSelection::Score=>"score",SeedSelection::Diverse=>"diverse"},"seed_budget":options.maximum_seed_attempts,"seed_candidates":seed_candidates,"initializer":match initializer {Initializer::EightPoint=>"eight_point",Initializer::FivePoint=>"five_point"},"resection":match resection {Resection::Linear=>"linear",Resection::P3p=>"p3p",Resection::P3pConsensus=>"p3p_consensus"},"seed":[a,b],"seed_points":seed_points,"registered":placed.len(),"input":count,"points":tracks.points.len(),"registration":rounds,"global_bundle_adjustment":placed.len()<=128,"fixed_angles":false,"supplied_poses":false});
                    let solved = solver::Solved {
                        rotations,
                        translations,
                        points: tracks.points,
                        observations: tracks.observations,
                        report: report.clone(),
                    };
                    let names_placed: Vec<_> = placed.iter().map(|&v| names[v].clone()).collect();
                    let mut solution = solver::solution(lens, &names_placed, &own, &solved);
                    solution.unregistered = (0..count).filter(|&v| poses[v].is_none()).map(|v| names[v].clone()).collect();
                    attempts.push(json!({"seed":[a,b],"registered":placed.len()}));
                    let recovered = Recovered { solution, report };
                    if best.as_ref().is_none_or(|old| recovered.solution.views.len() > old.solution.views.len()) {
                        best = Some(recovered);
                    }
                    if placed.len() * 5 >= count * 4 {
                        let mut recovered = best.unwrap();
                        recovered.report["seed_attempts"] = json!(attempts);
                        return Ok(recovered);
                    }
                    continue 'seeds;
                }
            }
        }
    }
    let mut recovered = best.context("incremental cameras: no seed reached a consistent third view")?;
    recovered.report["seed_attempts"] = json!(attempts);
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::super::markers::linalg::{cross, mul, norm, scale, sub};
    use super::*;
    fn capture(duplicate_only: bool, realistic: bool) -> (Matches, Lens, Vec<String>, Vec<M3>, Vec<V3>) {
        let lens = Lens { width: 1280, height: 960, pixels: [900., 920., 639.5, 479.5], k: [-0.02, 0.003, 0.] };
        let camera = Camera::from_lens(&lens);
        let angles = [0., 0.14, 0.39, 0.39, 0.73, 1.12, 1.43, 1.81, 2.06, 2.55, 2.92, 3.31, 3.55, 3.98, 4.35, 4.86, 5.37];
        let order = [8, 0, 16, 4, 11, 2, 14, 1, 6, 12, 3, 9, 5, 15, 7, 13, 10];
        let points: Vec<V3> = (0..if realistic { 360 } else { 180 })
            .map(|k| {
                if realistic {
                    let face = k / 60;
                    let u = ((k % 60) as f64 * 1.73).sin() * 0.28;
                    let v = ((k % 60) as f64 * 2.41).cos() * 0.28;
                    let mut p = [u, v, 0.];
                    p[face / 2] = if face % 2 == 0 { 0.3 } else { -0.3 };
                    p[(face / 2 + 1) % 3] = u;
                    p[(face / 2 + 2) % 3] = v;
                    return p;
                }
                let k = k as f64;
                [0.4 * (k * 1.73).sin(), 0.5 * (k * 2.41).cos(), 0.35 * (k * 0.91).sin()]
            })
            .collect();
        let mut keypoints = Vec::new();
        let mut rotations = Vec::new();
        let mut positions = Vec::new();
        let mut visibility = Vec::new();
        for &v in &order {
            let a: f64 = if duplicate_only { 0. } else { angles[v] };
            let h = if duplicate_only { 0.4 } else { 0.4 + 0.5 * (a * 1.7).sin() };
            let distance = if realistic { 5. } else { 3. };
            let centre = [distance * a.sin(), h, distance * a.cos()];
            let forward = unit(scale(centre, -1.));
            let right = unit(cross(forward, [0., 1., 0.]));
            let down = cross(forward, right);
            let r = [right, down, forward];
            let t = scale(apply(&r, centre), -1.);
            let pixels = points
                .iter()
                .enumerate()
                .map(|(k, &p)| {
                    let q = add(apply(&r, p), t);
                    let mut xy = camera.distort([q[0] / q[2], q[1] / q[2]]);
                    if realistic {
                        xy[0] += 0.25 * ((k * 73 + v * 11) as f64).sin();
                        xy[1] += 0.25 * ((k * 43 + v * 19) as f64).cos();
                    }
                    xy
                })
                .collect();
            visibility.push(
                points
                    .iter()
                    .enumerate()
                    .map(|(k, p)| {
                        if !realistic {
                            return true;
                        }
                        let axis = (k / 60) / 2;
                        let sign = if (k / 60) % 2 == 0 { 1. } else { -1. };
                        sign * (centre[axis] - p[axis]) > 0.05
                    })
                    .collect::<Vec<_>>(),
            );
            keypoints.push(pixels);
            rotations.push(r);
            positions.push(centre);
        }
        let mut pairs = Vec::new();
        for i in 0..order.len() {
            for j in i + 1..order.len() {
                if norm(sub(positions[i], positions[j])) < 3. {
                    let list = (0..points.len() as u32)
                        .filter(|&p| visibility[i][p as usize] && visibility[j][p as usize])
                        .map(|p| if p % 6 != 5 { (p, p) } else { (p, (p * 17 + 31) % points.len() as u32) })
                        .collect();
                    pairs.push((i, j, list));
                }
            }
        }
        // A disconnected photo must remain unregistered, rather than getting an invented angle/pose.
        keypoints.push(vec![[10., 10.]; 40]);
        let names = (0..keypoints.len()).map(|i| format!("photo_{i}.jpg")).collect();
        (Matches { keypoints, pairs }, lens, names, rotations, points)
    }
    #[test]
    fn seed_diversity_does_not_spend_the_whole_budget_on_repeated_pairs() {
        let pose = (IDENTITY, [0.; 3]);
        let mut seeds = vec![(0, 1, pose, 100., 50); 10];
        seeds.extend([(2, 3, pose, 20., 40), (4, 5, pose, 20., 40)]);
        let score = select_seeds(seeds.clone(), SeedSelection::Score, 3, 6);
        assert!(score.iter().all(|s| (s.0, s.1) == (0, 1)));
        let diverse = select_seeds(seeds, SeedSelection::Diverse, 3, 6);
        assert_eq!(diverse.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(), vec![(0, 1), (2, 3), (4, 5)]);
    }
    #[test]
    fn seed_budget_is_bounded_before_reconstruction() {
        let matches = Matches::default();
        let lens = Lens { width: 100, height: 100, pixels: [100., 100., 50., 50.], k: [0.; 3] };
        for budget in [0, 257] {
            let error = recover_with_options(
                &matches,
                &lens,
                &[],
                RecoveryOptions { maximum_seed_attempts: budget, ..RecoveryOptions::default() },
                &mut |_, _| Ok(()),
            )
            .err()
            .unwrap();
            assert!(error.to_string().contains("seed budget"));
        }
    }
    #[test]
    fn uncertain_measurements_keep_original_calibration_and_pixels() {
        let (mut matches, lens, names, truth, _) = capture(false, false);
        for (v, features) in matches.keypoints.iter_mut().enumerate().take(17) {
            for (k, p) in features.iter_mut().enumerate() {
                p[0] += 1.5 * ((v * 37 + k * 71) as f64).sin();
                p[1] += 1.5 * ((v * 19 + k * 43) as f64).cos();
            }
        }
        let recovered = recover_with_options(
            &matches,
            &lens,
            &names,
            RecoveryOptions { measurement_error_scale: 4., resection: Resection::P3p, ..RecoveryOptions::default() },
            &mut |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(recovered.solution.lens, lens);
        assert_eq!(recovered.solution.views.len(), 17);
        assert_eq!(recovered.solution.unregistered, ["photo_17.jpg"]);
        let first = &recovered.solution.views[0];
        let index = |view: &super::super::solution::View| names.iter().position(|name| name == &view.source).unwrap();
        for view in &recovered.solution.views {
            let expected = mul(&truth[index(view)], &transpose(&truth[index(first)]));
            let actual = mul(&view.rotation, &transpose(&first.rotation));
            assert!(norm(geometry::rotation_vector(&mul(&actual, &transpose(&expected)))) < 0.03);
        }
        for point in &recovered.solution.landmarks {
            for &(v, xy) in &point.observations {
                let source = index(&recovered.solution.views[v]);
                assert!(matches.keypoints[source].contains(&xy));
            }
        }
        assert_eq!(recovered.report["measurement_error_scale"], 4.);
    }
    #[test]
    fn measurement_uncertainty_is_bounded_before_geometry() {
        let matches = Matches::default();
        let lens = Lens { width: 100, height: 100, pixels: [100., 100., 50., 50.], k: [0.; 3] };
        for scale in [0., -1., 0.124, 16.001, f64::INFINITY, f64::NAN] {
            let error = recover_with_options(
                &matches,
                &lens,
                &[],
                RecoveryOptions { measurement_error_scale: scale, ..RecoveryOptions::default() },
                &mut |_, _| Ok(()),
            )
            .err()
            .unwrap();
            assert!(error.to_string().contains("measurement error scale"));
        }
    }
    #[test]
    fn irregular_shuffled_multilevel_capture_with_duplicates_and_outliers() {
        let (matches, lens, names, truth, _) = capture(false, false);
        let recovered = recover(&matches, &lens, &names, &mut |_, _| Ok(())).unwrap();
        assert_eq!(recovered.solution.views.len(), 17);
        assert_eq!(recovered.solution.unregistered, ["photo_17.jpg"]);
        let camera = Camera::from_lens(&lens);
        let mut errors = Vec::new();
        for point in &recovered.solution.landmarks {
            for &(v, p) in &point.observations {
                let view = &recovered.solution.views[v];
                let q = add(apply(&view.rotation, point.position), view.translation);
                assert!(q[2] > 0.);
                let projected = camera.distort([q[0] / q[2], q[1] / q[2]]);
                errors.push((projected[0] - p[0]).hypot(projected[1] - p[1]));
            }
        }
        errors.sort_by(f64::total_cmp);
        assert!(errors[errors.len() / 2] < 0.2, "median {}", errors[errors.len() / 2]);
        let views = &recovered.solution.views;
        let i = views[0].source.trim_start_matches("photo_").trim_end_matches(".jpg").parse::<usize>().unwrap();
        let j = views[1].source.trim_start_matches("photo_").trim_end_matches(".jpg").parse::<usize>().unwrap();
        let expected = mul(&truth[j], &transpose(&truth[i]));
        let actual = mul(&views[1].rotation, &transpose(&views[0].rotation));
        let difference = mul(&actual, &transpose(&expected));
        assert!(norm(geometry::rotation_vector(&difference)) < 0.01);
        assert_eq!(recovered.report["fixed_angles"], false);
    }
    #[test]
    fn noisy_narrow_object_with_surface_occlusion_recovers_camera_rotations() {
        check_noisy_capture(Resection::Linear);
    }
    #[test]
    fn minimal_resection_recovers_noisy_narrow_camera_rotations_and_surface() {
        check_noisy_capture(Resection::P3p);
    }
    #[test]
    fn five_point_initializer_recovers_noisy_camera_rotations_and_surface() {
        check_noisy_capture_with(Initializer::FivePoint, Resection::P3p);
    }
    fn check_noisy_capture(resection: Resection) {
        check_noisy_capture_with(Initializer::EightPoint, resection);
    }
    fn check_noisy_capture_with(initializer: Initializer, resection: Resection) {
        let (matches, lens, names, truth, truth_points) = capture(false, true);
        let recovered = recover_with_solvers(&matches, &lens, &names, initializer, resection, &mut |_, _| Ok(())).unwrap();
        assert!(recovered.solution.views.len() >= 15, "registered {}", recovered.solution.views.len());
        let views = &recovered.solution.views;
        let index = |name: &str| name.trim_start_matches("photo_").trim_end_matches(".jpg").parse::<usize>().unwrap();
        let reference = &views[0];
        let mut angular_errors = Vec::new();
        for view in &views[1..] {
            let expected = mul(&truth[index(&view.source)], &transpose(&truth[index(&reference.source)]));
            let actual = mul(&view.rotation, &transpose(&reference.rotation));
            angular_errors.push(norm(geometry::rotation_vector(&mul(&actual, &transpose(&expected)))).to_degrees());
        }
        angular_errors.sort_by(f64::total_cmp);
        assert!(angular_errors[angular_errors.len() / 2] < 2., "median camera rotation error {:?}", angular_errors);
        // A small reprojection error alone can hide a distorted or reflected reconstruction.
        // Align with a proper similarity and check the known synthetic surface as well.
        let mut source = Vec::new();
        let mut target = Vec::new();
        let mut alignment_source = Vec::new();
        let mut alignment_target = Vec::new();
        for point in &recovered.solution.landmarks {
            let (view, pixel) = point.observations[0];
            let original = index(&views[view].source);
            let feature = matches.keypoints[original].iter().position(|p| *p == pixel).unwrap();
            source.push(point.position);
            target.push(truth_points[feature]);
            let consistent = point.observations.iter().all(|&(v, p)| {
                let original = index(&views[v].source);
                matches.keypoints[original].iter().position(|q| *q == p) == Some(feature)
            });
            if consistent {
                alignment_source.push(point.position);
                alignment_target.push(truth_points[feature]);
            }
        }
        assert!(alignment_source.len() * 100 >= source.len() * 85, "too many inconsistent recovered tracks");
        let mean = |points: &[V3]| points.iter().fold([0.; 3], |a, &p| add(a, scale(p, 1. / points.len() as f64)));
        let (c0, c1) = (mean(&alignment_source), mean(&alignment_target));
        let mut covariance = [[0.; 3]; 3];
        for (&x, &y) in alignment_source.iter().zip(&alignment_target) {
            let (x, y) = (sub(x, c0), sub(y, c1));
            for i in 0..3 {
                for j in 0..3 {
                    covariance[i][j] += x[i] * y[j];
                }
            }
        }
        let (u, _, mut v) = geometry::svd3(&covariance);
        let mut rotation = mul(&v, &transpose(&u));
        if super::super::markers::linalg::determinant(&rotation) < 0. {
            for row in &mut v {
                row[2] *= -1.;
            }
            rotation = mul(&v, &transpose(&u));
        }
        let numerator: f64 =
            alignment_source.iter().zip(&alignment_target).map(|(&x, &y)| dot(apply(&rotation, sub(x, c0)), sub(y, c1))).sum();
        let denominator: f64 = alignment_source.iter().map(|&x| dot(sub(x, c0), sub(x, c0))).sum();
        let factor = numerator / denominator;
        let mut errors: Vec<_> =
            source.iter().zip(&target).map(|(&x, &y)| norm(sub(add(scale(apply(&rotation, sub(x, c0)), factor), c1), y))).collect();
        errors.sort_by(f64::total_cmp);
        assert!(
            errors[errors.len() / 2] < 0.03,
            "median aligned surface error {}, scale {}, consistent {}/{}",
            errors[errors.len() / 2],
            factor,
            alignment_source.len(),
            source.len()
        );
        assert!(recovered.solution.unregistered.contains(&"photo_17.jpg".to_owned()));
    }
    #[test]
    fn identical_views_do_not_invent_a_depth_baseline() {
        let (matches, lens, names, _, _) = capture(true, false);
        assert!(recover(&matches, &lens, &names, &mut |_, _| Ok(())).is_err());
    }
    #[test]
    fn malformed_matches_are_rejected_before_indexing() {
        let (mut matches, lens, names, _, _) = capture(false, false);
        matches.pairs[0].2.push((u32::MAX, 0));
        assert!(recover(&matches, &lens, &names, &mut |_, _| Ok(())).is_err());
    }
}
