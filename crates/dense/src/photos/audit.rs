//! Read-only checks of an AliceVision pinhole scene: a port of
//! `scripts/turntable_mesh/alicevision_cameras.py` (`pixel_intrinsic`,
//! `radial_field_check`, `audit_scene`). No camera is fitted or corrected.
//!
//! The JSON focal and offset conventions follow AliceVision 1.2.14. Passing
//! supports use as photo reconstruction cameras; it is not a statement about
//! the accuracy of the shape.

use std::path::Path;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::inputs::percentile_sorted_f64;

use super::solution::{parse_sfm, sfm_lens, Solution};
use super::util;

/// `(width, height, [fx, fy, cx, cy], [k1, k2, k3])` with the principal point from the image corner pixel centre.
pub type PixelIntrinsic = (u32, u32, [f64; 4], [f64; 3]);

/// Converts the native serialization to zero-origin fx, fy, cx, cy and k.
pub fn pixel_intrinsic(row: &Value) -> anyhow::Result<PixelIntrinsic> {
    let lens = sfm_lens(row)?;
    Ok((lens.width, lens.height, lens.pixels, lens.k))
}

/// Positive real roots of `c[0] x^3 + c[1] x^2 + c[2] x + c[3]`, ascending. Leading zeros lower the degree.
fn positive_roots(coefficients: [f64; 4]) -> Vec<f64> {
    let start = coefficients.iter().position(|&c| c != 0.0).unwrap_or(4);
    let c = &coefficients[start..];
    let mut roots = match c.len() {
        4 => {
            let (a, b, cc, d) = (c[0], c[1], c[2], c[3]);
            // Depressed cubic t^3 + p t + q with x = t - b / (3 a).
            let shift = b / (3.0 * a);
            let p = (3.0 * a * cc - b * b) / (3.0 * a * a);
            let q = (2.0 * b * b * b - 9.0 * a * b * cc + 27.0 * a * a * d) / (27.0 * a * a * a);
            let discriminant = q * q / 4.0 + p * p * p / 27.0;
            if discriminant > 0.0 {
                let root = discriminant.sqrt();
                vec![(-q / 2.0 + root).cbrt() + (-q / 2.0 - root).cbrt() - shift]
            } else if p == 0.0 {
                vec![-shift]
            } else {
                let m = 2.0 * (-p / 3.0).sqrt();
                let theta = ((3.0 * q / (p * m)).clamp(-1.0, 1.0)).acos() / 3.0;
                (0..3).map(|k| m * (theta - 2.0 * std::f64::consts::PI * k as f64 / 3.0).cos() - shift).collect()
            }
        }
        3 => {
            let discriminant = c[1] * c[1] - 4.0 * c[0] * c[2];
            if discriminant < 0.0 {
                vec![]
            } else {
                let root = discriminant.sqrt();
                vec![(-c[1] - root) / (2.0 * c[0]), (-c[1] + root) / (2.0 * c[0])]
            }
        }
        2 => vec![-c[1] / c[0]],
        _ => vec![],
    };
    // Polish on the original polynomial.
    let value = |x: f64| ((coefficients[0] * x + coefficients[1]) * x + coefficients[2]) * x + coefficients[3];
    let slope = |x: f64| (3.0 * coefficients[0] * x + 2.0 * coefficients[1]) * x + coefficients[2];
    for root in &mut roots {
        for _ in 0..4 {
            let step = value(*root) / slope(*root);
            if step.is_finite() {
                *root -= step;
            }
        }
    }
    roots.retain(|r| r.is_finite() && *r > 0.0);
    roots.sort_by(f64::total_cmp);
    roots
}

/// Does the increasing branch of the radial model reach every image corner?
///
/// `g(r) = r (1 + k1 r^2 + k2 r^4 + k3 r^6)`. The first positive root of its
/// derivative ends the strictly increasing branch; the image is representable
/// on that branch iff its largest distorted corner radius is below `g(root)`.
pub fn radial_field_check(width: u32, height: u32, p: [f64; 4], k: [f64; 3]) -> Value {
    if p[0] <= 0.0 || p[1] <= 0.0 || !(0.0 <= p[2] && p[2] < width as f64) || !(0.0 <= p[3] && p[3] < height as f64) {
        return json!({"passed": false, "reason": "invalid focal length or principal point"});
    }
    let mut radius = 0f64;
    for x in [0.0, width as f64 - 1.0] {
        for y in [0.0, height as f64 - 1.0] {
            radius = radius.max((((x - p[2]) / p[0]).powi(2) + ((y - p[3]) / p[1]).powi(2)).sqrt());
        }
    }
    let first = positive_roots([7.0 * k[2], 5.0 * k[1], 3.0 * k[0], 1.0]).first().map(|r| r.sqrt());
    let maximum = first.map(|r| r * (1.0 + k[0] * r.powi(2) + k[1] * r.powi(4) + k[2] * r.powi(6)));
    let passed = maximum.is_none_or(|m| m.is_finite() && radius < m * (1.0 - 1e-8));
    json!({
        "passed": passed, "maximum_distorted_corner_radius": radius, "first_radial_derivative_zero": first,
        "increasing_branch_distorted_radius_limit": maximum,
        "method": "analytic first positive root of 1+3k1*r²+5k2*r⁴+7k3*r⁶",
    })
}

fn stats(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "count": sorted.len(), "median": percentile_sorted_f64(&sorted, 50.0),
        "p95": percentile_sorted_f64(&sorted, 95.0), "max": sorted[sorted.len() - 1],
    })
}

/// What the audit is asked to hold the scene to.
#[derive(Debug, Clone)]
pub struct Policy<'a> {
    pub expected_names: Option<&'a [String]>,
    /// The declared fixed lens `([fx, fy, cx, cy], [k1, k2, k3])` at the photo resolution.
    pub expected_calibration: Option<([f64; 4], [f64; 3])>,
    pub minimum_coverage: f64,
    pub minimum_observations_per_view: i64,
    pub maximum_reprojection_p95: f64,
}

impl Default for Policy<'_> {
    fn default() -> Self {
        Policy {
            expected_names: None,
            expected_calibration: None,
            minimum_coverage: 0.8,
            minimum_observations_per_view: 20,
            maximum_reprojection_p95: 4.0,
        }
    }
}

/// Checks a camera solution against the policy: lens, coverage, sparse
/// observations in front of their cameras, reprojection per view. The report
/// has the keys of `alicevision_cameras.audit_scene` (without the file's
/// name and hash, which [`audit_scene`] adds).
pub fn audit_solution(solution: &Solution, policy: &Policy) -> anyhow::Result<Value> {
    if !(policy.minimum_coverage.is_finite() && policy.maximum_reprojection_p95.is_finite())
        || !(policy.minimum_coverage > 0.0 && policy.minimum_coverage <= 1.0)
        || !(policy.maximum_reprojection_p95 > 0.0 && policy.maximum_reprojection_p95 <= 100.0)
    {
        bail!("invalid audit bound");
    }
    if !(1..=100_000).contains(&policy.minimum_observations_per_view) {
        bail!("invalid observation bound");
    }
    let mut names: Vec<&String> = solution.views.iter().map(|v| &v.source).chain(&solution.unregistered).collect();
    let input_images = names.len();
    names.sort();
    names.dedup();
    if names.len() != input_images
        || policy.expected_names.is_some_and(|expected| {
            let mut expected: Vec<&String> = expected.iter().collect();
            expected.sort();
            expected != names
        })
    {
        bail!("camera image inventory mismatch");
    }
    let lens = &solution.lens;
    let (p, k) = (lens.pixels, lens.k);
    let mut reasons: Vec<&str> = Vec::new();
    if solution.landmarks.is_empty() {
        reasons.push("no triangulated landmarks");
    }
    let check = radial_field_check(lens.width, lens.height, p, k);
    if check["passed"] != true {
        reasons.push("lens principal branch does not cover the image field");
    }
    if let Some((expected_pixels, expected_k)) = policy.expected_calibration {
        let same = p.iter().zip(expected_pixels).all(|(a, b)| (a - b).abs() <= 1e-7)
            && k.iter().zip(expected_k).all(|(a, b)| (a - b).abs() <= 1e-12);
        if !same || solution.lens_locked == Some(false) {
            reasons.push("declared fixed calibration was changed or unlocked");
        }
    }
    let coverage = solution.views.len() as f64 / input_images.max(1) as f64;
    if coverage < policy.minimum_coverage {
        reasons.push("insufficient registered camera coverage");
    }
    let mut per_camera: Vec<(Vec<f64>, usize, usize)> = vec![(Vec::new(), 0, 0); solution.views.len()];
    let (mut errors, mut positive, mut observation_count) = (Vec::new(), 0usize, 0usize);
    for landmark in &solution.landmarks {
        let x = landmark.position;
        for &(view, measured) in &landmark.observations {
            observation_count += 1;
            let camera = &solution.views[view];
            let d = [x[0] - camera.centre[0], x[1] - camera.centre[1], x[2] - camera.centre[2]];
            let q = camera.rotation.map(|row| row[0] * d[0] + row[1] * d[1] + row[2] * d[2]);
            per_camera[view].1 += 1;
            if q[2] <= 1e-12 || q[2].is_nan() {
                if !reasons.contains(&"landmark behind camera") {
                    reasons.push("landmark behind camera");
                }
                continue;
            }
            positive += 1;
            per_camera[view].2 += 1;
            let n = [q[0] / q[2], q[1] / q[2]];
            let rr = n[0] * n[0] + n[1] * n[1];
            let gain = 1.0 + k[0] * rr + k[1] * rr.powi(2) + k[2] * rr.powi(3);
            let error = ((n[0] * gain * p[0] + p[2] - measured[0]).powi(2) + (n[1] * gain * p[1] + p[3] - measured[1]).powi(2)).sqrt();
            if !error.is_finite() {
                bail!("nonfinite reprojection");
            }
            errors.push(error);
            per_camera[view].0.push(error);
        }
    }
    let mut order: Vec<usize> = (0..solution.views.len()).collect();
    order.sort_by(|&a, &b| solution.views[a].source.cmp(&solution.views[b].source));
    let mut per_view = Vec::new();
    for view in order {
        let (view_errors, seen, in_front) = &per_camera[view];
        let summary = stats(view_errors);
        if (view_errors.len() as i64) < policy.minimum_observations_per_view {
            reasons.push("insufficient distinct landmark support per registered view");
        } else if summary["p95"].as_f64().is_some_and(|p95| p95 > policy.maximum_reprojection_p95) {
            reasons.push("per-view reprojection exceeds declared bound");
        }
        per_view.push(json!({
            "name": solution.views[view].source, "observations": seen, "reprojection_pixels": summary,
            "positive_depth_fraction": if *seen == 0 { Value::Null } else { json!(*in_front as f64 / *seen as f64) },
        }));
    }
    reasons.sort_unstable();
    reasons.dedup();
    let mut missing: Vec<&String> = solution.unregistered.iter().collect();
    missing.sort();
    Ok(json!({
        "passed": reasons.is_empty(), "reasons": reasons,
        "input_images": input_images, "registered_cameras": solution.views.len(), "coverage": coverage, "missing_names": missing,
        "landmarks": solution.landmarks.len(), "observations": observation_count, "reprojection_pixels": stats(&errors),
        "per_view": per_view,
        "lenses": [{"intrinsic_id": "shared", "pixel_intrinsic": p, "radial_coefficients": k, "field_check": check}],
        "positive_depth_fraction": if observation_count == 0 { Value::Null } else { json!(positive as f64 / observation_count as f64) },
        "policy": {
            "minimum_coverage": policy.minimum_coverage, "minimum_observations_per_view": policy.minimum_observations_per_view,
            "maximum_per_view_reprojection_p95_pixels": policy.maximum_reprojection_p95,
            "fixed_calibration_required": policy.expected_calibration.is_some(),
        },
        "reference_geometry_used": false, "shape_accuracy_claim": false,
    }))
}

/// Audits an AliceVision scene file; the file is never changed.
pub fn audit_scene(path: &Path, policy: &Policy) -> anyhow::Result<Value> {
    let size = crate::photos::fs::file_len(path);
    if size.is_none_or(|s| s > 128 << 20) {
        bail!("native scene missing or exceeds 128 MiB bound");
    }
    let raw = crate::photos::fs::read(path).with_context(|| path.display().to_string())?;
    let sha = util::sha256(&raw);
    let scene: Value = serde_json::from_slice(&raw).with_context(|| path.display().to_string())?;
    let mut report = audit_solution(&parse_sfm(&scene)?, policy)?;
    if util::sha256_file(path)? != sha {
        bail!("native scene changed during audit");
    }
    let absolute = crate::photos::fs::absolute(path);
    report["scene"] = json!(absolute.to_string_lossy());
    report["scene_sha256"] = json!(sha);
    report["source_unchanged"] = json!(true);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Value {
        let intrinsic = json!({
            "intrinsicId": "1", "type": "pinhole", "distortionType": "radialk3", "width": "100", "height": "80", "sensorWidth": "36",
            "focalLength": "36", "pixelRatio": "1.1", "principalPoint": ["2", "-1"], "distortionParams": ["0", "0", "0"], "locked": "true",
        });
        let (_, _, p, _) = pixel_intrinsic(&intrinsic).unwrap();
        let (mut views, mut poses, mut points) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..3 {
            views.push(json!({"viewId": i.to_string(), "poseId": i.to_string(), "intrinsicId": "1", "path": format!("frame{i}.png")}));
            poses.push(json!({"poseId": i.to_string(), "pose": {"transform": {
                "rotation": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], "center": [i as f64 * 0.1, 0.0, 0.0]}}}));
        }
        for i in 0..25 {
            let x = [i as f64 * 0.004, 0.01 * (i % 5) as f64, 2.0];
            let observations: Vec<Value> = (0..3)
                .map(|v| json!({"observationId": v.to_string(), "x": [(x[0] - v as f64 * 0.1) / x[2] * p[0] + p[2], x[1] / x[2] * p[1] + p[3]]}))
                .collect();
            points.push(json!({"landmarkId": i.to_string(), "X": x, "observations": observations}));
        }
        json!({"version": ["1", "2", "14"], "views": views, "poses": poses, "intrinsics": [intrinsic], "structure": points})
    }

    fn audit(data: &Value, policy: &Policy) -> anyhow::Result<Value> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("crispmesh-audit-{}-{n}.sfm", std::process::id()));
        std::fs::write(&path, serde_json::to_vec(data).unwrap()).unwrap();
        let before = util::sha256_file(&path).unwrap();
        let out = audit_scene(&path, policy);
        assert_eq!(before, util::sha256_file(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        out
    }

    #[test]
    fn serialized_calibration_and_full_valid_scene() {
        let data = scene();
        let (width, height, p, k) = pixel_intrinsic(&data["intrinsics"][0]).unwrap();
        assert_eq!((width, height), (100, 80));
        assert!((p[0] - 100.0 / 1.1).abs() < 1e-12 && p[1] == 100.0 && p[2] == 52.0 && p[3] == 39.0 && k == [0.0; 3]);
        let names: Vec<String> = (0..3).map(|i| format!("frame{i}.png")).collect();
        let policy = Policy { expected_names: Some(&names), expected_calibration: Some((p, k)), ..Policy::default() };
        let out = audit(&data, &policy).unwrap();
        assert_eq!(out["passed"], true, "{}", out["reasons"]);
        assert_eq!(
            (out["registered_cameras"].as_u64(), out["landmarks"].as_u64(), out["observations"].as_u64()),
            (Some(3), Some(25), Some(75))
        );
        assert!(out["reprojection_pixels"]["max"].as_f64().unwrap() < 1e-9);
        assert_eq!(out["positive_depth_fraction"], 1.0);
        assert_eq!(out["per_view"][1]["name"], "frame1.png");
        assert_eq!(out["per_view"][1]["reprojection_pixels"]["count"], 25);
        assert_eq!(out["policy"]["fixed_calibration_required"], true);
        assert_eq!(out["lenses"][0]["field_check"]["first_radial_derivative_zero"], Value::Null);
    }

    #[test]
    fn changed_or_unlocked_lens_and_inventory_are_reported() {
        let data = scene();
        let (_, _, p, k) = pixel_intrinsic(&data["intrinsics"][0]).unwrap();
        let policy = Policy { expected_calibration: Some(([p[0] + 1e-3, p[1], p[2], p[3]], k)), ..Policy::default() };
        assert_eq!(audit(&data, &policy).unwrap()["reasons"], json!(["declared fixed calibration was changed or unlocked"]));
        let mut unlocked = data.clone();
        unlocked["intrinsics"][0]["locked"] = json!("false");
        let policy = Policy { expected_calibration: Some((p, k)), ..Policy::default() };
        assert_eq!(audit(&unlocked, &policy).unwrap()["passed"], false);
        let names = vec!["frame0.png".to_string(), "frame1.png".to_string(), "other.png".to_string()];
        let policy = Policy { expected_names: Some(&names), ..Policy::default() };
        assert_eq!(audit(&data, &policy).unwrap_err().to_string(), "camera image inventory mismatch");
        let mut legacy = data.clone();
        legacy["version"] = json!(["1", "2", "10"]);
        assert_eq!(audit(&legacy, &Policy::default()).unwrap_err().to_string(), "unsupported legacy focal serialization");
        let mut fisheye = data.clone();
        fisheye["intrinsics"][0]["type"] = json!("fisheye");
        assert_eq!(audit(&fisheye, &Policy::default()).unwrap_err().to_string(), "unsupported camera model");
    }

    #[test]
    fn coverage_support_reprojection_and_depth_gates() {
        let mut data = scene();
        data["poses"].as_array_mut().unwrap().pop();
        for point in data["structure"].as_array_mut().unwrap() {
            point["observations"].as_array_mut().unwrap().pop();
        }
        let out = audit(&data, &Policy::default()).unwrap();
        assert_eq!(out["reasons"], json!(["insufficient registered camera coverage"]));
        assert_eq!(out["missing_names"], json!(["frame2.png"]));
        assert!((out["coverage"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-15);
        let out = audit(&scene(), &Policy { minimum_observations_per_view: 26, ..Policy::default() }).unwrap();
        assert_eq!(out["reasons"], json!(["insufficient distinct landmark support per registered view"]));
        let mut shifted = scene();
        for point in shifted["structure"].as_array_mut().unwrap() {
            let x = point["observations"][0]["x"][0].as_f64().unwrap();
            point["observations"][0]["x"][0] = json!(x + 5.0);
        }
        let out = audit(&shifted, &Policy::default()).unwrap();
        assert_eq!(out["reasons"], json!(["per-view reprojection exceeds declared bound"]));
        assert!((out["per_view"][0]["reprojection_pixels"]["p95"].as_f64().unwrap() - 5.0).abs() < 1e-9);
        let mut behind = scene();
        behind["structure"][0]["X"] = json!([0.0, 0.0, -2.0]);
        let out = audit(&behind, &Policy::default()).unwrap();
        assert_eq!(out["reasons"], json!(["landmark behind camera"]));
        assert!((out["positive_depth_fraction"].as_f64().unwrap() - 72.0 / 75.0).abs() < 1e-15);
        let mut improper = scene();
        improper["poses"][0]["pose"]["transform"]["rotation"][0] = json!(-1.0);
        assert_eq!(audit(&improper, &Policy::default()).unwrap_err().to_string(), "camera rotation is not proper");
        let mut repeated = scene();
        repeated["structure"][0]["observations"][1]["observationId"] = json!("0");
        assert_eq!(audit(&repeated, &Policy::default()).unwrap_err().to_string(), "invalid bounded landmark observations");
        assert!(audit(&scene(), &Policy { minimum_coverage: 0.0, ..Policy::default() }).is_err());
    }

    #[test]
    fn radial_field_check_uses_the_first_derivative_root() {
        // Barrel distortion that folds inside the frame: g(r) = r (1 - r^2) peaks at r = 1 / sqrt(3).
        let check = radial_field_check(100, 80, [50.0, 50.0, 49.5, 39.5], [-1.0, 0.0, 0.0]);
        assert_eq!(check["passed"], false);
        assert!((check["first_radial_derivative_zero"].as_f64().unwrap() - (1f64 / 3.0).sqrt()).abs() < 1e-12);
        assert!((check["increasing_branch_distorted_radius_limit"].as_f64().unwrap() - (1f64 / 3.0).sqrt() * (2.0 / 3.0)).abs() < 1e-12);
        // The 3DLF lens at the photo size passes (numbers from the reference audit of the Dragon cameras).
        let k = [-0.17180718751091592, 0.41234681155235325, -3.7169739384093505];
        let check = radial_field_check(1749, 1155, [2328.2847290039062, 2329.8724365234375, 874.1592102050781, 555.7409973144531], k);
        assert_eq!(check["passed"], true);
        assert!((check["first_radial_derivative_zero"].as_f64().unwrap() - 0.5872807150077594).abs() < 1e-9, "{check}");
        assert_eq!(radial_field_check(100, 80, [50.0, 50.0, 100.0, 39.5], [0.0; 3])["passed"], false);
        assert_eq!(positive_roots([1.0, -6.0, 11.0, -6.0]).iter().map(|r| r.round()).collect::<Vec<_>>(), [1.0, 2.0, 3.0]);
        assert_eq!(positive_roots([0.0, 1.0, -3.0, 2.0]), [1.0, 2.0]);
        assert_eq!(positive_roots([0.0, 0.0, 2.0, -1.0]), [0.5]);
        assert!(positive_roots([0.0, 0.0, 0.0, 1.0]).is_empty());
    }
}
