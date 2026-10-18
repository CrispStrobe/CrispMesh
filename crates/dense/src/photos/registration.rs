//! Experimental camera registration with ambiguous feature-to-point associations.
//! Pose hypotheses select one-to-one geometric consensus; repeated edges never add votes.
use super::markers::linalg::{add, apply, M3, V3};
use super::turntable::{
    geometry::{Random, P2},
    solver,
};
use std::collections::BTreeSet;
/// A canonical photo feature, a triangulated point ID, its pixel bearing and position.
#[derive(Clone, Copy)]
pub(crate) struct Link {
    pub feature: u32,
    pub point: usize,
    pub bearing: P2,
    pub position: V3,
}
fn assignment(links: &[Link], r: &M3, t: V3, focal: f64, bound: f64) -> (Vec<usize>, f64) {
    let mut ranked: Vec<_> = links
        .iter()
        .enumerate()
        .filter_map(|(i, link)| {
            let q = add(apply(r, link.position), t);
            if q[2] <= 0. {
                return None;
            }
            let error = (q[0] / q[2] - link.bearing[0]).hypot(q[1] / q[2] - link.bearing[1]) * focal;
            (error < bound).then_some((error, i))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (mut features, mut points) = (BTreeSet::new(), BTreeSet::new());
    let mut chosen = Vec::new();
    let mut cost = 0.;
    for (error, i) in ranked {
        let link = links[i];
        if features.contains(&link.feature) || points.contains(&link.point) {
            continue;
        }
        features.insert(link.feature);
        points.insert(link.point);
        chosen.push(i);
        cost += error * error;
    }
    (chosen, cost)
}
// Collinear world support or a subpixel image cluster cannot constrain a camera pose.
fn observable(links: &[Link], indices: &[usize], focal: f64) -> bool {
    if indices.len() < 12 {
        return false;
    }
    let n = indices.len() as f64;
    let centre = indices.iter().fold([0.; 2], |mut c, &i| {
        for (k, v) in c.iter_mut().enumerate() {
            *v += links[i].bearing[k] / n;
        }
        c
    });
    let mut covariance = [[0.; 2]; 2];
    for &i in indices {
        let p = links[i].bearing;
        let x = (p[0] - centre[0]) * focal;
        let y = (p[1] - centre[1]) * focal;
        covariance[0][0] += x * x / n;
        covariance[0][1] += x * y / n;
        covariance[1][1] += y * y / n;
    }
    let xx = covariance[0][0];
    let xy = covariance[0][1];
    let yy = covariance[1][1];
    let minor = (xx + yy - ((xx - yy).powi(2) + 4. * xy * xy).sqrt()) * 0.5;
    if !minor.is_finite() || minor < 1. {
        return false;
    }
    let centre = indices.iter().fold([0.; 3], |mut c, &i| {
        for (k, v) in c.iter_mut().enumerate() {
            *v += links[i].position[k] / n;
        }
        c
    });
    let mut covariance = vec![vec![0.; 3]; 3];
    for &i in indices {
        let d = std::array::from_fn::<_, 3, _>(|k| links[i].position[k] - centre[k]);
        for (a, row) in covariance.iter_mut().enumerate() {
            for (b, v) in row.iter_mut().enumerate() {
                *v += d[a] * d[b] / n;
            }
        }
    }
    let (values, _) = super::markers::linalg::symmetric_eigen(&covariance);
    values[2] > 0. && values[1] > values[2] * 1e-8
}
/// Votes are unique in both feature and world-point identity, even with repeated candidate edges.
pub(crate) fn recover(links: &[Link], starts: &[(M3, V3)], focal: f64, random: &mut Random) -> Option<(M3, V3, usize)> {
    if !focal.is_finite()
        || focal <= 0.
        || links.iter().map(|l| l.feature).collect::<BTreeSet<_>>().len() < 12
        || links.iter().map(|l| l.point).collect::<BTreeSet<_>>().len() < 12
    {
        return None;
    }
    let mut best: Option<(usize, f64, M3, V3)> = None;
    let consider = |r: M3, t: V3, best: &mut Option<(usize, f64, M3, V3)>| {
        let (indices, cost) = assignment(links, &r, t, focal, 4.);
        let n = indices.len();
        if best.as_ref().is_none_or(|v| n > v.0 || (n == v.0 && cost < v.1)) {
            *best = Some((n, cost, r, t));
        }
    };
    // Existing-camera starts are hypotheses only; every candidate is rescored against all edges.
    for &(r, t) in starts {
        let (indices, _) = assignment(links, &r, t, focal, 100.);
        let seen: Vec<_> = indices.iter().map(|&i| (links[i].bearing, links[i].position)).collect();
        if let Some((rr, tt, _)) = solver::resect_general(&seen, &[(r, t)], focal, random) {
            consider(rr, tt, &mut best);
        }
    }
    let mut needed = 1000;
    let mut iteration = 0;
    while iteration < needed {
        iteration += 1;
        let mut sample = Vec::new();
        for _ in 0..100 {
            let k = random.below(links.len());
            if sample.iter().all(|&i: &usize| links[i].feature != links[k].feature && links[i].point != links[k].point) {
                sample.push(k);
            }
            if sample.len() == 3 {
                break;
            }
        }
        if sample.len() != 3 {
            continue;
        }
        let seen = std::array::from_fn(|i| {
            let link = links[sample[i]];
            (link.bearing, link.position)
        });
        for (r, t) in super::p3p::solve(&seen) {
            consider(r, t, &mut best);
        }
        if let Some((count, _, _, _)) = best {
            let chance = (count as f64 / links.len() as f64).powi(3);
            let trials = if chance >= 1. - 1e-12 { 1. } else { 0.001f64.ln() / (1. - chance).ln() };
            needed = needed.min((trials.ceil() as usize).max(50));
        }
    }
    let (_, _, mut r, mut t) = best?;
    for _ in 0..3 {
        let (indices, _) = assignment(links, &r, t, focal, 4.);
        let seen: Vec<_> = indices.iter().map(|&i| (links[i].bearing, links[i].position)).collect();
        let Some((rr, tt, _)) = solver::resect_general(&seen, &[(r, t)], focal, random) else { break };
        let (before, cost) = assignment(links, &r, t, focal, 3.);
        let (after, new_cost) = assignment(links, &rr, tt, focal, 3.);
        if after.len() < before.len() || (after.len() == before.len() && new_cost > cost) {
            break;
        }
        r = rr;
        t = tt;
    }
    let (indices, _) = assignment(links, &r, t, focal, 3.);
    observable(links, &indices, focal).then_some((r, t, indices.len()))
}
#[cfg(test)]
mod tests {
    use super::super::markers::linalg::{norm, rodrigues, rotation_angle_deg, sub};
    use super::*;
    #[test]
    fn competing_points_are_resolved_without_a_nearby_pose() {
        let r = rodrigues([0.3, -0.4, 0.2]);
        let t = [0.1, -0.04, 3.];
        let points: Vec<_> = (0..40)
            .map(|i| {
                let f = i as f64;
                [(f * 1.3).sin() * 0.4, (f * 2.1).cos() * 0.3, (f * 1.7).cos() * 0.2]
            })
            .collect();
        let mut links = Vec::new();
        for i in 0..points.len() {
            let q = add(apply(&r, points[i]), t);
            let bearing = [q[0] / q[2], q[1] / q[2]];
            for p in [i, (i + 7) % points.len(), (i + 13) % points.len()] {
                links.push(Link { feature: i as u32, point: p, bearing, position: points[p] });
            }
        }
        let (rr, tt, n) = recover(&links, &[], 2000., &mut Random(12345)).unwrap();
        assert_eq!(n, 40);
        assert!(rotation_angle_deg(&rr, &r) < 0.01);
        assert!(norm(sub(tt, t)) < 0.001);
    }
    #[test]
    fn coincident_features_cannot_constrain_a_camera_even_with_many_ids() {
        let links: Vec<_> = (0..40).map(|i| Link { feature: i, point: i as usize, bearing: [0., 0.], position: [0., 0., 1.] }).collect();
        assert!(recover(&links, &[([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [0.; 3])], 2000., &mut Random(7)).is_none());
    }
    #[test]
    fn repeated_edges_and_points_do_not_inflate_consensus() {
        let mut links = Vec::new();
        for _ in 0..50 {
            for i in 0..5 {
                links.push(Link { feature: i, point: i as usize, bearing: [0., 0.], position: [0., 0., 1.] });
            }
        }
        assert!(recover(&links, &[], 2000., &mut Random(7)).is_none());
        let (chosen, _) = assignment(&links, &[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [0.; 3], 2000., 3.);
        assert_eq!(chosen.len(), 5);
    }
}
