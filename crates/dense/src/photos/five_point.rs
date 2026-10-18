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

//! Calibrated five-point essential matrices. Trace-constraint and elimination layout
//! adapted from PoseLib relpose_5pt.cc, revision 6a636b1d84273d5e3ba97676463476569611572a
//! (BSD-3-Clause). Polynomial convolution, nullspace and derivative root isolation
//! use the project's Rust arithmetic; no Eigen/Sturm source or new dependency.
use super::markers::linalg::{determinant as det3, mul, transpose, M3};
use super::turntable::geometry::{sampson, Random, P2};

// Nister elimination monomials, in the same order as PoseLib's constraints.
const POWERS: [[u8; 3]; 20] = [
    [3, 0, 0],
    [0, 3, 0],
    [2, 1, 0],
    [1, 2, 0],
    [2, 0, 1],
    [2, 0, 0],
    [0, 2, 1],
    [0, 2, 0],
    [1, 1, 1],
    [1, 1, 0],
    [1, 0, 2],
    [1, 0, 1],
    [1, 0, 0],
    [0, 1, 2],
    [0, 1, 1],
    [0, 1, 0],
    [0, 0, 3],
    [0, 0, 2],
    [0, 0, 1],
    [0, 0, 0],
];
type Poly = [f64; 20];
fn product(a: &Poly, b: &Poly) -> Poly {
    let mut c = [0.; 20];
    for i in 0..20 {
        if a[i] == 0. {
            continue;
        }
        for j in 0..20 {
            if b[j] == 0. {
                continue;
            }
            let power = std::array::from_fn(|k| POWERS[i][k] + POWERS[j][k]);
            if let Some(k) = POWERS.iter().position(|p| *p == power) {
                c[k] += a[i] * b[j];
            }
        }
    }
    c
}
fn accumulate(a: &mut Poly, b: &Poly, f: f64) {
    for i in 0..20 {
        a[i] += f * b[i];
    }
}
fn dot(a: &[f64; 9], b: &[f64; 9]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
fn orthogonal(mut v: [f64; 9], basis: &[[f64; 9]]) -> [f64; 9] {
    for _ in 0..2 {
        for q in basis {
            let s = dot(&v, q);
            for k in 0..9 {
                v[k] -= s * q[k];
            }
        }
    }
    v
}
fn nullspace(a: &[P2; 5], b: &[P2; 5]) -> Option<[[f64; 9]; 4]> {
    let mut rows = Vec::new();
    for i in 0..5 {
        let [x, y] = a[i];
        let [u, v] = b[i];
        let row = [u * x, u * y, u, v * x, v * y, v, x, y, 1.];
        let n = dot(&row, &row).sqrt();
        if !n.is_finite() || n <= 0. {
            return None;
        }
        rows.push(row.map(|v| v / n));
    }
    let mut basis = Vec::new();
    for _ in 0..5 {
        let (k, row, n) = rows
            .iter()
            .enumerate()
            .map(|(k, row)| {
                let r = orthogonal(*row, &basis);
                (k, r, dot(&r, &r))
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))?;
        if n < 1e-20 {
            return None;
        }
        basis.push(row.map(|v| v / n.sqrt()));
        rows.remove(k);
    }
    let mut null = [[0.; 9]; 4];
    for dest in &mut null {
        let (row, n) = (0..9)
            .map(|k| {
                let mut v = [0.; 9];
                v[k] = 1.;
                let v = orthogonal(v, &basis);
                (v, dot(&v, &v))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        if n < 1e-20 {
            return None;
        }
        *dest = row.map(|v| v / n.sqrt());
        basis.push(*dest);
    }
    Some(null)
}
fn constraints(n: &[[f64; 9]; 4]) -> [[f64; 20]; 10] {
    let e: [[Poly; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let mut p = [0.; 20];
            for (k, index) in [12, 15, 18, 19].into_iter().enumerate() {
                p[index] = n[k][3 * i + j];
            }
            p
        })
    });
    let mut eet = [[[0.; 20]; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for (a, b) in e[i].iter().zip(&e[j]) {
                accumulate(&mut eet[i][j], &product(a, b), 1.);
            }
        }
    }
    let mut trace = [0.; 20];
    for (i, row) in eet.iter().enumerate() {
        accumulate(&mut trace, &row[i], 0.5);
    }
    for (i, row) in eet.iter_mut().enumerate() {
        accumulate(&mut row[i], &trace, -1.);
    }
    let mut c = [[0.; 20]; 10];
    for i in 0..3 {
        for j in 0..3 {
            for (k, row) in e.iter().enumerate() {
                accumulate(&mut c[3 * i + j], &product(&eet[i][k], &row[j]), 1.);
            }
        }
    }
    for (p, sign) in [([0, 1, 2], 1.), ([1, 2, 0], 1.), ([2, 0, 1], 1.), ([0, 2, 1], -1.), ([1, 0, 2], -1.), ([2, 1, 0], -1.)] {
        accumulate(&mut c[9], &product(&product(&e[0][p[0]], &e[1][p[1]]), &e[2][p[2]]), sign);
    }
    c
}
fn eliminate(mut c: [[f64; 20]; 10]) -> Option<[[f64; 10]; 10]> {
    for col in 0..10 {
        let pivot = (col..10).max_by(|&a, &b| c[a][col].abs().total_cmp(&c[b][col].abs()))?;
        if !c[pivot][col].is_finite() || c[pivot][col].abs() < 1e-12 {
            return None;
        }
        c.swap(col, pivot);
        let scale = c[col][col];
        for value in &mut c[col][col..] {
            *value /= scale;
        }
        let pivot_row = c[col];
        for (i, row) in c.iter_mut().enumerate() {
            if i == col {
                continue;
            }
            let factor = row[col];
            for (value, pivot) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= factor * pivot;
            }
        }
    }
    Some(std::array::from_fn(|i| std::array::from_fn(|j| c[i][10 + j])))
}
fn eval(p: &[f64], x: f64) -> f64 {
    p.iter().rev().fold(0., |v, c| v * x + c)
}
fn real_roots(p: &[f64]) -> Vec<f64> {
    let scale = p.iter().fold(0f64, |s, v| s.max(v.abs()));
    if !scale.is_finite() || scale == 0. {
        return vec![];
    }
    let mut p: Vec<_> = p.iter().map(|v| v / scale).collect();
    while p.len() > 1 && p.last().unwrap().abs() < 1e-14 {
        p.pop();
    }
    let degree = p.len() - 1;
    if degree == 0 {
        return vec![];
    }
    if degree == 1 {
        return vec![-p[0] / p[1]];
    }
    let radius = 2. * (0..degree).map(|i| (p[i] / p[degree]).abs().powf(1. / (degree - i) as f64)).fold(1f64, f64::max);
    if !radius.is_finite() {
        return vec![];
    }
    let derivative: Vec<_> = (1..p.len()).map(|i| p[i] * i as f64).collect();
    let mut cuts = vec![-radius];
    cuts.extend(real_roots(&derivative).into_iter().filter(|x| x.is_finite() && *x > -radius && *x < radius));
    cuts.push(radius);
    cuts.sort_by(f64::total_cmp);
    let mut roots = Vec::new();
    for &x in &cuts[1..cuts.len() - 1] {
        let error = eval(&p, x).abs();
        let magnitude = p.iter().rev().fold(0., |v, c| v * x.abs() + c.abs());
        if error < 1e-12 * magnitude {
            roots.push(x);
        }
    }
    for interval in cuts.windows(2) {
        let (mut lo, mut hi) = (interval[0], interval[1]);
        let mut f = eval(&p, lo);
        if f == 0. {
            roots.push(lo);
        }
        if !f.is_finite() || !eval(&p, hi).is_finite() || f.signum() == eval(&p, hi).signum() {
            continue;
        }
        for _ in 0..96 {
            let mid = lo + (hi - lo) * 0.5;
            if mid == lo || mid == hi {
                break;
            }
            let fm = eval(&p, mid);
            if fm.signum() == f.signum() {
                lo = mid;
                f = fm;
            } else {
                hi = mid;
            }
        }
        roots.push(lo + (hi - lo) * 0.5);
    }
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|a, b| (*a - *b).abs() < 1e-9 * (1. + a.abs().max(b.abs())));
    roots
}
/// Up to ten calibrated essential matrices from five undistorted pixel bearings.
pub(crate) fn solve(a: &[P2; 5], b: &[P2; 5]) -> Vec<M3> {
    if a.iter().chain(b).flatten().any(|x| !x.is_finite()) {
        return vec![];
    }
    let Some(n) = nullspace(a, b) else { return vec![] };
    let Some(c) = eliminate(constraints(&n)) else { return vec![] };
    // A(z) [x,y,1]^T = 0, built by subtracting paired eliminated constraints.
    let mut aa = [[[0.; 5]; 3]; 3];
    for (i, row) in aa.iter_mut().enumerate() {
        for (col, poly) in row.iter_mut().enumerate() {
            let len = if col == 2 { 4 } else { 3 };
            let off = if col == 2 { 6 } else { col * 3 };
            for k in 0..len {
                poly[len - 1 - k] += c[4 + 2 * i][off + k];
                poly[len - k] -= c[5 + 2 * i][off + k];
            }
        }
    }
    let mut determinant = [0.; 13];
    for (perm, sign) in [([0, 1, 2], 1.), ([1, 2, 0], 1.), ([2, 0, 1], 1.), ([0, 2, 1], -1.), ([1, 0, 2], -1.), ([2, 1, 0], -1.)] {
        for i in 0..5 {
            for j in 0..5 {
                for k in 0..5 {
                    determinant[i + j + k] += sign * aa[0][perm[0]][i] * aa[1][perm[1]][j] * aa[2][perm[2]][k];
                }
            }
        }
    }
    let mut matrices = Vec::new();
    for z in real_roots(&determinant[..11]) {
        let m: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| eval(&aa[i][j], z)));
        let mut best = None;
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            let d = m[i][0] * m[j][1] - m[i][1] * m[j][0];
            if d.abs() > best.map(|(score, _, _)| score).unwrap_or(0.) {
                best = Some((d.abs(), i, j));
            }
        }
        let Some((_, i, j)) = best else { continue };
        let d = m[i][0] * m[j][1] - m[i][1] * m[j][0];
        let x = (-m[i][2] * m[j][1] + m[i][1] * m[j][2]) / d;
        let y = (-m[i][0] * m[j][2] + m[i][2] * m[j][0]) / d;
        let scale = (x * x + y * y + z * z + 1.).sqrt();
        let e: M3 = std::array::from_fn(|i| {
            std::array::from_fn(|j| (n[0][3 * i + j] * x + n[1][3 * i + j] * y + n[2][3 * i + j] * z + n[3][3 * i + j]) / scale)
        });
        if e.iter().flatten().any(|v| !v.is_finite()) || det3(&e).abs() > 1e-7 {
            continue;
        }
        let eet = mul(&e, &transpose(&e));
        let trace = eet[0][0] + eet[1][1] + eet[2][2];
        let cubed = mul(&eet, &e);
        let error = (0..3).flat_map(|i| (0..3).map(move |j| (2. * cubed[i][j] - trace * e[i][j]).powi(2))).sum::<f64>();
        if error > 1e-12 || (0..5).any(|i| sampson(&e, a[i], b[i]) > 1e-6) {
            continue;
        }
        matrices.push(e);
    }
    matrices
}
/// Robust minimal sampling; the old eight-point estimator remains a separate path.
pub(crate) fn ransac(a: &[P2], b: &[P2], bound: f64, random: &mut Random) -> Option<(M3, Vec<bool>)> {
    if a.len() != b.len() || a.len() < 5 || !bound.is_finite() || bound <= 0. {
        return None;
    }
    let mut best = None;
    let mut best_count = 0;
    let mut best_cost = f64::INFINITY;
    let mut needed = 2000;
    let mut iteration = 0;
    while iteration < needed {
        iteration += 1;
        let mut picks = Vec::new();
        while picks.len() < 5 {
            let k = random.below(a.len());
            if !picks.contains(&k) {
                picks.push(k);
            }
        }
        let sa = std::array::from_fn(|i| a[picks[i]]);
        let sb = std::array::from_fn(|i| b[picks[i]]);
        for e in solve(&sa, &sb) {
            let errors: Vec<_> = (0..a.len()).map(|i| sampson(&e, a[i], b[i])).collect();
            let count = errors.iter().filter(|&&v| v < bound).count();
            let cost = errors.iter().map(|v| (v / bound).min(1.).powi(2)).sum();
            if count > best_count || (count == best_count && cost < best_cost) {
                best = Some(e);
                best_count = count;
                best_cost = cost;
                let chance = (count as f64 / a.len() as f64).powi(5);
                let trials = if chance >= 1. - 1e-12 { 1. } else { 0.001f64.ln() / (1. - chance).ln() };
                needed = needed.min((trials.ceil() as usize).max(50));
            }
        }
    }
    let e = best?;
    Some((e, (0..a.len()).map(|i| sampson(&e, a[i], b[i]) < bound).collect()))
}
#[cfg(test)]
mod tests {
    use super::super::markers::linalg::{add, apply, rodrigues};
    use super::super::turntable::geometry::essential_of;
    use super::*;
    #[test]
    fn five_bearings_include_the_true_essential_matrix() {
        for step in 0..40 {
            let f = step as f64;
            let r = rodrigues([0.1 + 0.007 * f, -0.3, 0.03]);
            let t = [0.2, 0.05, 0.1];
            let a = std::array::from_fn(|i| {
                let v = i as f64;
                [(v * 1.3 + f).sin() * 0.3, (v * 2.1 + f).cos() * 0.2]
            });
            let b = std::array::from_fn(|i| {
                let d = 2. + (i as f64 * 1.7 + f).cos() * 0.4;
                let p = add(apply(&r, [a[i][0] * d, a[i][1] * d, d]), t);
                [p[0] / p[2], p[1] / p[2]]
            });
            let expected = essential_of(&r, t);
            let norm = expected.iter().flatten().map(|v| v * v).sum::<f64>().sqrt();
            let solutions = solve(&a, &b);
            assert!(
                solutions.iter().any(|e| {
                    let d = e.iter().flatten().zip(expected.iter().flatten()).map(|(x, y)| x * y / norm).sum::<f64>();
                    d.abs() > 1. - 1e-6
                }),
                "step{step} solutions{}",
                solutions.len()
            );
        }
    }
    #[test]
    fn narrow_off_centre_bearings_preserve_the_true_pose() {
        for spread in [0.03, 0.01] {
            for step in 0..20 {
                let f = step as f64;
                let r = rodrigues([0.04, -0.08, 0.01]);
                let t = [0.08, -0.02, 0.01];
                let a = std::array::from_fn(|i| {
                    let v = i as f64;
                    [0.15 + (v * 1.3 + f).sin() * spread, 0.4 + (v * 2.1 + f).cos() * spread]
                });
                let b = std::array::from_fn(|i| {
                    let z = 2. + (i as f64 * 1.7 + f).cos() * 0.3;
                    let q = add(apply(&r, [a[i][0] * z, a[i][1] * z, z]), t);
                    [q[0] / q[2], q[1] / q[2]]
                });
                let expected = essential_of(&r, t);
                let norm = expected.iter().flatten().map(|x| x * x).sum::<f64>().sqrt();
                let solutions = solve(&a, &b);
                assert!(
                    solutions.iter().any(|e| e
                        .iter()
                        .flatten()
                        .zip(expected.iter().flatten())
                        .map(|(x, y)| x * y / norm)
                        .sum::<f64>()
                        .abs()
                        > 1. - 1e-5),
                    "spread{spread} step{step}, solutions{}",
                    solutions.len()
                );
            }
        }
    }
    #[test]
    fn minimal_sampling_recovers_geometry_with_seventy_percent_wrong_matches() {
        let r = rodrigues([0.12, -0.24, 0.04]);
        let t = [0.3, 0.04, 0.08];
        let a: Vec<_> = (0..200)
            .map(|i| {
                let f = i as f64;
                [(f * 1.3).sin() * 0.3, (f * 2.1).cos() * 0.2]
            })
            .collect();
        let b: Vec<_> = (0..200)
            .map(|i| {
                let f = i as f64;
                if i % 10 < 3 {
                    let z = 2. + (f * 1.7).cos() * 0.4;
                    let p = add(apply(&r, [a[i][0] * z, a[i][1] * z, z]), t);
                    [p[0] / p[2], p[1] / p[2]]
                } else {
                    [(f * 3.7).sin() * 0.4, (f * 4.3).cos() * 0.3]
                }
            })
            .collect();
        let (e, inliers) = ransac(&a, &b, 0.0005, &mut Random(314159)).unwrap();
        assert!(inliers.iter().filter(|&&v| v).count() >= 60);
        assert!((0..200).filter(|i| i % 10 < 3).all(|i| inliers[i]));
        let expected = essential_of(&r, t);
        let norm = expected.iter().flatten().map(|v| v * v).sum::<f64>().sqrt();
        let agreement = e.iter().flatten().zip(expected.iter().flatten()).map(|(x, y)| x * y / norm).sum::<f64>().abs();
        assert!(agreement > 0.999, "essential agreement {agreement}");
    }
    #[test]
    fn degenerate_and_nonfinite_data_are_refused() {
        assert!(solve(&[[0., 0.]; 5], &[[0., 0.]; 5]).is_empty());
        assert!(solve(&[[f64::NAN, 0.]; 5], &[[0., 0.]; 5]).is_empty());
    }
    #[test]
    fn polynomial_isolation_keeps_multiple_and_repeated_roots() {
        let roots = real_roots(&[4., 0., -3., 1.]);
        assert!(roots.iter().any(|x| (x + 1.).abs() < 1e-7));
        assert!(roots.iter().any(|x| (x - 2.).abs() < 1e-7));
    }
}
