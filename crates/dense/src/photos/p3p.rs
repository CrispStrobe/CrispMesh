// Copyright (c) 2020, Viktor Larsson
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//
//     * Neither the name of the copyright holder nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL <COPYRIGHT HOLDER> BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

// Author: Yaqing Ding
//         Mark Shachkov

//! Calibrated P3P, adapted to Rust from PoseLib at
//! 6a636b1d84273d5e3ba97676463476569611572a (BSD-3-Clause).
//! Source: solvers/p3p.cc, p3p_common.h and misc/univariate.cc.
//! Experimental general camera recovery only; no new runtime dependencies.

use super::markers::linalg::{apply, cross, determinant, dot, inverse, mul, scale, sub, transpose, unit, M3, V3};
use super::turntable::geometry::P2;

fn cubic(c2: f64, c1: f64, c0: f64) -> (f64, bool) {
    let a = c1 - c2 * c2 / 3.;
    let b = (2. * c2 * c2 * c2 - 9. * c2 * c1) / 27. + c0;
    let d = b * b / 4. + a * a * a / 27.;
    if d > 0. {
        let s = d.sqrt();
        return ((-b / 2. + s).cbrt() + (-b / 2. - s).cbrt() - c2 / 3., true);
    }
    if d < 0. {
        return (2. * (-a / 3.).sqrt() * ((3. * b / (2. * a) * (-3. / a).sqrt()).clamp(-1., 1.).acos() / 3.).cos() - c2 / 3., false);
    }
    (-c2 / 3. + if a != 0. { 3. * b / a } else { 0. }, false)
}
fn quadratic(b: f64, c: f64) -> Vec<f64> {
    if !b.is_finite() || !c.is_finite() {
        return vec![];
    }
    let d = b * b - 4. * c;
    if d < -1e-12 {
        return vec![];
    }
    if d <= 0. {
        return vec![-b / 2.];
    }
    let q = -0.5 * (b + d.sqrt().copysign(b));
    if q == 0. {
        vec![0.]
    } else {
        vec![q, c / q]
    }
}
fn columns(a: V3, b: V3, c: V3) -> M3 {
    transpose(&[a, b, c])
}
fn factors(mut c: M3) -> Option<[V3; 2]> {
    let co = transpose(&[cross(c[1], c[2]), cross(c[2], c[0]), cross(c[0], c[1])]).map(|r| r.map(|x| -x));
    let i = (0..3).max_by(|&a, &b| co[a][a].total_cmp(&co[b][b]))?;
    if co[i][i] <= 0. || !co[i][i].is_finite() {
        return None;
    }
    let v = scale([co[0][i], co[1][i], co[2][i]], 1. / co[i][i].sqrt());
    c[0][1] -= v[2];
    c[0][2] += v[1];
    c[1][2] -= v[0];
    c[1][0] += v[2];
    c[2][0] -= v[1];
    c[2][1] += v[0];
    Some([[c[0][0], c[1][0], c[2][0]], c[0]])
}
fn refine(d: &mut V3, a: V3, m: V3) {
    for _ in 0..5 {
        let [l0, l1, l2] = *d;
        let [m01, m02, m12] = m;
        let r0 = l0 * l0 - 2. * l0 * l1 * m01 + l1 * l1 - a[0];
        let r1 = l0 * l0 - 2. * l0 * l2 * m02 + l2 * l2 - a[1];
        let r2 = l1 * l1 - 2. * l1 * l2 * m12 + l2 * l2 - a[2];
        if r0.abs() + r1.abs() + r2.abs() < 1e-10 {
            return;
        }
        let x11 = l0 - l1 * m01;
        let x12 = l1 - l0 * m01;
        let x21 = l0 - l2 * m02;
        let x23 = l2 - l0 * m02;
        let x32 = l1 - l2 * m12;
        let x33 = l2 - l1 * m12;
        let det = 0.5 / (x11 * x23 * x32 + x12 * x21 * x33);
        d[0] += (-x23 * x32 * r0 - x12 * x33 * r1 + x12 * x23 * r2) * det;
        d[1] += (-x21 * x33 * r0 + x11 * x33 * r1 - x11 * x23 * r2) * det;
        d[2] += (x21 * x32 * r0 - x11 * x32 * r1 - x12 * x21 * r2) * det;
    }
}
/// Up to four world-to-camera hypotheses. Bearings are undistorted coordinates.
/// Degenerate/collinear triples and non-finite solutions are rejected.
pub(crate) fn solve(seen: &[(P2, V3); 3]) -> Vec<(M3, V3)> {
    if seen.iter().any(|(p, x)| p.iter().chain(x).any(|v| !v.is_finite())) {
        return vec![];
    }
    let origin = seen[0].1;
    let extent = (0..3).map(|i| dot(sub(seen[i].1, seen[(i + 1) % 3].1), sub(seen[i].1, seen[(i + 1) % 3].1))).fold(0f64, f64::max).sqrt();
    if extent <= 0. || !extent.is_finite() {
        return vec![];
    }
    let normalised = seen.map(|(p, x)| (p, scale(sub(x, origin), 1. / extent)));
    solve_normalised(&normalised).into_iter().map(|(r, t)| (r, sub(scale(t, extent), apply(&r, origin)))).collect()
}

fn solve_normalised(seen: &[(P2, V3); 3]) -> Vec<(M3, V3)> {
    let mut x = seen.map(|(p, _)| unit([p[0], p[1], 1.]));
    let mut world = seen.map(|(_, p)| p);
    let squared = |v: V3| dot(v, v);
    let mut a01 = squared(sub(world[0], world[1]));
    let mut a02 = squared(sub(world[0], world[2]));
    let mut a12 = squared(sub(world[1], world[2]));
    if a01 > a02 {
        if a01 > a12 {
            x.swap(0, 2);
            world.swap(0, 2);
            std::mem::swap(&mut a01, &mut a12);
        }
    } else if a02 > a12 {
        x.swap(0, 1);
        world.swap(0, 1);
        std::mem::swap(&mut a02, &mut a12);
    }
    if a12 <= 0. || a01.min(a02) <= a12 * 1e-14 {
        return vec![];
    }
    let e1 = sub(world[0], world[1]);
    let e2 = sub(world[0], world[2]);
    if squared(cross(e1, e2)) < a12 * a12 * 1e-12 {
        return vec![];
    }
    let Some(xx) = inverse(&columns(e1, e2, cross(e1, e2))) else {
        return vec![];
    };
    let a = a01 / a12;
    let b = a02 / a12;
    let m01 = dot(x[0], x[1]);
    let m02 = dot(x[0], x[2]);
    let m12 = dot(x[1], x[2]);
    let m12sq = 1. - m12 * m12;
    let m02sq = m02 * m02 - 1.;
    let m01sq = m01 * m01 - 1.;
    let ab = a * b;
    let bsq = b * b;
    let asq = a * a;
    let m013 = -2. + 2. * m01 * m02 * m12;
    let bsqm12sq = bsq * m12sq;
    let asqm12sq = asq * m12sq;
    let abm12sq = 2. * ab * m12sq;
    let denom = bsqm12sq + b * m02sq;
    if denom == 0. || !denom.is_finite() {
        return vec![];
    }
    let (s, single) = cubic(
        ((-1. + a) * m02sq + abm12sq + bsqm12sq + b * m013) / denom,
        (asqm12sq + abm12sq + a * m013 + (-1. + b) * m01sq) / denom,
        (asqm12sq + a * m01sq) / denom,
    );
    let c =
        [[-a + s * (1. - b), -m02 * s, a * m12 + b * m12 * s], [-m02 * s, s + 1., -m01], [a * m12 + b * m12 * s, -m01, -a - b * s + 1.]];
    let Some(pq) = factors(c) else {
        return vec![];
    };
    let mut out = Vec::new();
    for [p0, p1, p2] in pq {
        let switch = p0.abs() <= p1.abs();
        let (w0, w1, cb, cc) = if switch {
            let w0 = -p0 / p1;
            let w1 = -p2 / p1;
            let ca = 1. / (w1 * w1 - b);
            (w0, w1, 2. * (b * m12 - m02 * w1 + w0 * w1) * ca, (w0 * w0 - 2. * m02 * w0 - b + 1.) * ca)
        } else {
            let w0 = -p1 / p0;
            let w1 = -p2 / p0;
            let ca = 1. / (-a * w1 * w1 + 2. * a * m12 * w1 - a + 1.);
            (w0, w1, 2. * (a * m12 * w0 - m01 - a * w0 * w1) * ca, (1. - a * w0 * w0) * ca)
        };
        for tau in quadratic(cb, cc) {
            if tau <= 0. {
                continue;
            }
            let mut d = if switch {
                let d2 = (a12 / (tau * (tau - 2. * m12) + 1.)).sqrt();
                let d1 = tau * d2;
                [w0 * d2 + w1 * d1, d1, d2]
            } else {
                let d0 = (a01 / (tau * (tau - 2. * m01) + 1.)).sqrt();
                let d1 = tau * d0;
                [d0, d1, w0 * d0 + w1 * d1]
            };
            if d.iter().any(|v| !v.is_finite() || *v <= 0.) {
                continue;
            }
            refine(&mut d, [a01, a02, a12], [m01, m02, m12]);
            if d.iter().any(|v| !v.is_finite() || *v <= 0.) {
                continue;
            }
            let v1 = sub(scale(x[0], d[0]), scale(x[1], d[1]));
            let v2 = sub(scale(x[0], d[0]), scale(x[2], d[2]));
            let r = mul(&columns(v1, v2, cross(v1, v2)), &xx);
            let t = sub(scale(x[0], d[0]), apply(&r, world[0]));
            let rr = mul(&r, &transpose(&r));
            let orthogonal = (0..3).all(|i| (0..3).all(|j| (rr[i][j] - if i == j { 1. } else { 0. }).abs() < 1e-4));
            if r.iter().flatten().chain(t.iter()).all(|v| v.is_finite()) && orthogonal && (determinant(&r) - 1.).abs() < 1e-4 {
                out.push((r, t));
            }
        }
        if !out.is_empty() && single {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::markers::linalg::{add, norm, rodrigues, rotation_angle_deg};
    use super::*;
    #[test]
    fn calibrated_triplets_recover_true_pose_at_varied_scales_and_narrow_fields() {
        for distance in [2., 8., 40.] {
            for world_scale in [0.001, 1., 100.] {
                for k in 0..24 {
                    let r = rodrigues([0.15 + (k as f64) * 0.03, -0.4, 0.05]);
                    let t = scale([0.2, -0.1, distance], world_scale);
                    let points = [[0.12, -0.3, 0.03], [-0.31, 0.15, 0.19], [0.22, 0.24, -0.21]].map(|p| scale(p, world_scale));
                    let seen = points.map(|p| {
                        let q = add(apply(&r, p), t);
                        ([q[0] / q[2], q[1] / q[2]], p)
                    });
                    let solutions = solve(&seen);
                    assert!(
                        solutions.iter().any(|(rr, tt)| rotation_angle_deg(rr, &r) < 0.01 && norm(sub(*tt, t)) < world_scale * 0.001),
                        "distance {distance}, scale {world_scale}, k {k}, solutions {}",
                        solutions.len()
                    );
                }
            }
        }
    }
    #[test]
    fn degenerate_triples_do_not_invent_a_pose() {
        for points in [[[0., 0., 0.]; 3], [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]], [[f64::NAN, 0., 0.], [0., 0., 0.], [1., 1., 1.]]] {
            assert!(solve(&points.map(|p| ([0., 0.], p))).is_empty());
        }
    }
}
