//! A synthetic turntable capture as raw photos: what a camera with a mildly
//! distorting lens would see of a richly textured dark sphere on a light
//! backdrop, with the lens calibration and the true orbit. It exercises the
//! whole chain from photos (masks, a camera provider such as COLMAP, the dense
//! stages) without any dataset; [`verify`] then checks a run made from it.

#![allow(clippy::needless_range_loop)]

use std::path::Path;

use anyhow::{bail, ensure, Context};
use serde_json::{json, Value};

use super::synthetic::camera;

/// Lattice hash to [0, 1).
fn hash(x: i64, y: i64, z: i64, seed: u32) -> f64 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f)
        ^ seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h as f64 / 4_294_967_296.0
}

/// Smooth value noise in [0, 1): lattice values interpolated with a smoothstep.
fn noise(p: [f64; 3], seed: u32) -> f64 {
    let base = p.map(f64::floor);
    let t = [0, 1, 2].map(|a| {
        let f = p[a] - base[a];
        f * f * (3.0 - 2.0 * f)
    });
    let mut value = 0.0;
    for corner in 0..8 {
        let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let weight: f64 = (0..3).map(|a| if o[a] == 1 { t[a] } else { 1.0 - t[a] }).product();
        value += weight * hash(base[0] as i64 + o[0] as i64, base[1] as i64 + o[1] as i64, base[2] as i64 + o[2] as i64, seed);
    }
    value
}

/// Grey of the sphere's surface: blobs and speckle over five octaves, dark (0.06 to 0.5).
pub fn texture(p: [f64; 3]) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 4.0;
    for octave in 0..5 {
        value += amplitude * noise(p.map(|v| v * frequency + 7.3), octave);
        amplitude *= 0.6;
        frequency *= 2.3;
    }
    0.06 + 0.44 * (value / 1.1528).clamp(0.0, 1.0)
}

/// The lens of the capture: pixel-centre principal point, three radial coefficients.
#[derive(Debug, Clone, Copy)]
pub struct CaptureLens {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub k: [f64; 3],
}

/// Writes `photos/shot_NNN.png`, `lens.json` and `truth.json` into a fresh directory.
/// The camera circles the unit sphere at six radii, twenty degrees above its equator.
/// The unit sphere, and for a capture of known handedness three bumps of different sizes on its
/// +x, +y and +z axes (centres and radii), large enough that much of the surface carries the asymmetry:
/// a mirror image of this object is not a rotation of it.
pub const BUMPS: [([f64; 3], f64); 3] = [([1.0, 0.0, 0.0], 0.7), ([0.0, 1.0, 0.0], 0.5), ([0.0, 0.0, 1.0], 0.3)];

/// Nearest hit of a ray with the object (the sphere, with the bumps when `asymmetric`), as a ray parameter.
fn hit(origin: [f64; 3], direction: [f64; 3], asymmetric: bool) -> Option<f64> {
    let spheres = std::iter::once(([0.0; 3], 1.0)).chain(BUMPS.iter().copied().filter(|_| asymmetric));
    let mut best: Option<f64> = None;
    for (centre, radius) in spheres {
        let o = [origin[0] - centre[0], origin[1] - centre[1], origin[2] - centre[2]];
        let a: f64 = direction.iter().map(|d| d * d).sum();
        let b: f64 = (0..3).map(|i| direction[i] * o[i]).sum();
        let c: f64 = o.iter().map(|v| v * v).sum::<f64>() - radius * radius;
        let discriminant = b * b - a * c;
        if discriminant > 0.0 {
            let t = (-b - discriminant.sqrt()) / a;
            if t > 0.0 && best.is_none_or(|old| t < old) {
                best = Some(t);
            }
        }
    }
    best
}

pub fn write(output: &Path, views: usize, width: usize, height: usize) -> anyhow::Result<()> {
    write_object(output, views, width, height, false)
}

/// [`write`] of the sphere, or with `asymmetric` of the sphere with three bumps ([`BUMPS`]).
pub fn write_object(output: &Path, views: usize, width: usize, height: usize, asymmetric: bool) -> anyhow::Result<()> {
    ensure!(views >= 8 && width >= 160 && height >= 120, "a capture needs at least 8 views of 160 x 120 pixels");
    ensure!(!output.exists(), "output directory exists: {}", output.display());
    std::fs::create_dir_all(output.join("photos")).with_context(|| output.display().to_string())?;
    // The object (radius 1, with the bumps up to 1.7 from the centre) fills nine tenths of the frame height.
    let reach = if asymmetric { 1.75 } else { 1.0 };
    let focal = 0.9 * height as f64 / (2.0 * (reach / 6.0f64).asin().tan());
    let lens =
        CaptureLens { fx: focal, fy: focal * 1.002, cx: width as f64 / 2.0 - 0.7, cy: height as f64 / 2.0 + 0.4, k: [-0.06, 0.02, -0.004] };
    let (distance, elevation) = (6.0, 20.0);
    let mut truth = Vec::new();
    for n in 0..views {
        let (rotation, translation) = camera(360.0 * n as f64 / views as f64, elevation, distance);
        let centre =
            [0, 1, 2].map(|a| -(rotation[0][a] * translation[0] + rotation[1][a] * translation[1] + rotation[2][a] * translation[2]));
        let mut pixels = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                // Four samples per pixel; each is undistorted to a ray and intersected with the sphere.
                let mut sum = 0.0;
                for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let (xd, yd) = ((x as f64 - 0.5 + sx - lens.cx) / lens.fx, (y as f64 - 0.5 + sy - lens.cy) / lens.fy);
                    let (mut u, mut v) = (xd, yd);
                    for _ in 0..10 {
                        let r2 = u * u + v * v;
                        let gain = 1.0 + r2 * (lens.k[0] + r2 * (lens.k[1] + r2 * lens.k[2]));
                        (u, v) = (xd / gain, yd / gain);
                    }
                    let direction = [0, 1, 2].map(|a| u * rotation[0][a] + v * rotation[1][a] + rotation[2][a]);
                    sum += match hit(centre, direction, asymmetric) {
                        Some(t) => texture([0, 1, 2].map(|i| centre[i] + direction[i] * t)),
                        None => 0.86,
                    };
                }
                pixels.extend([(255.0 * sum / 4.0 + 0.5) as u8; 3]);
            }
        }
        let name = format!("shot_{n:03}.png");
        crate::storage::save_png(output.join("photos").join(&name), width, height, 3, &pixels)?;
        truth.push(json!({"photo": name, "centre": centre, "rotation": rotation}));
    }
    let calibration = json!({
        "schema": "crispmesh_lens_calibration_v1", "model": "radialk3", "calibration_width": width, "calibration_height": height,
        "fx": lens.fx, "fy": lens.fy, "cx": lens.cx, "cy": lens.cy, "k1": lens.k[0], "k2": lens.k[1], "k3": lens.k[2],
        "principal_point_convention": "pixel_centre", "sensor_width_mm": 36.0,
        "provenance": {"source": "crispmesh-dense synthetic --capture: the lens the photos were rendered with"},
    });
    std::fs::write(output.join("lens.json"), serde_json::to_string_pretty(&calibration)? + "\n")?;
    let object = if asymmetric { "unit sphere at the origin with bumps" } else { "unit sphere at the origin" };
    let bumps: Vec<Value> = BUMPS.iter().filter(|_| asymmetric).map(|(c, r)| json!({"centre": c, "radius": r})).collect();
    let truth = json!({"views": truth, "orbit_radius": distance, "elevation_degrees": elevation, "object": object, "bumps": bumps,
                       "camera_convention": "x right, y down, z forward (right-handed); rotation maps world to camera"});
    std::fs::write(output.join("truth.json"), serde_json::to_string_pretty(&truth)? + "\n")?;
    Ok(())
}

/// Checks a run made from a capture: every photo registered, the recovered
/// camera centres on one circle in capture order with even steps (properties
/// that do not depend on the frame or scale the cameras were recovered in), the
/// object at the true distance relative to the orbit, and a closed mesh from the
/// dense stages when the run has them. Returns the measurements; fails with the
/// first one out of bounds.
pub fn verify(run: &Path, capture: &Path) -> anyhow::Result<Value> {
    let read = |path: &Path| -> anyhow::Result<Value> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path).with_context(|| path.display().to_string())?)?)
    };
    let truth = read(&capture.join("truth.json"))?;
    let expected = truth["views"].as_array().map(Vec::len).unwrap_or(0);
    let frontend = if run.join("frontend/frontend.json").is_file() { run.join("frontend") } else { run.to_path_buf() };
    let report = read(&frontend.join("frontend.json"))?;
    ensure!(report["status"] == "complete", "the photos stage ended with status {}: {}", report["status"], report["reasons"]);
    let rows = crate::inputs::load_views(&frontend.join("inputs"))?;
    ensure!(rows.len() == expected, "{} of {expected} photos registered", rows.len());
    let centres: Vec<[f64; 3]> = rows
        .iter()
        .map(|row| {
            [0, 1, 2].map(|a| {
                -(row.rotation[0][a] * row.translation[0]
                    + row.rotation[1][a] * row.translation[1]
                    + row.rotation[2][a] * row.translation[2])
            })
        })
        .collect();
    let count = centres.len() as f64;
    let middle = [0, 1, 2].map(|a| centres.iter().map(|c| c[a]).sum::<f64>() / count);
    let mut scatter = [[0.0f64; 3]; 3];
    for c in &centres {
        let d = [0, 1, 2].map(|a| c[a] - middle[a]);
        for i in 0..3 {
            for j in 0..3 {
                scatter[i][j] += d[i] * d[j];
            }
        }
    }
    let normal = crate::repair::smallest_eigenvector(scatter);
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let offsets: Vec<[f64; 3]> = centres.iter().map(|c| [0, 1, 2].map(|a| c[a] - middle[a])).collect();
    let radii: Vec<f64> = offsets.iter().map(|d| (dot(*d, *d) - dot(*d, normal).powi(2)).max(0.0).sqrt()).collect();
    let radius = radii.iter().sum::<f64>() / count;
    let radius_spread = radii.iter().map(|r| (r - radius).abs()).fold(0.0, f64::max) / radius;
    let out_of_plane = offsets.iter().map(|d| dot(*d, normal).abs()).fold(0.0, f64::max) / radius;
    // Angle between consecutive photos around the orbit axis.
    let step = 360.0 / count;
    let mut worst_step = 0.0f64;
    for n in 0..centres.len() {
        let (a, b) = (offsets[n], offsets[(n + 1) % centres.len()]);
        let flat = |d: [f64; 3]| [0, 1, 2].map(|i| d[i] - dot(d, normal) * normal[i]);
        let (a, b) = (flat(a), flat(b));
        let angle = (dot(a, b) / (dot(a, a).sqrt() * dot(b, b).sqrt())).clamp(-1.0, 1.0).acos().to_degrees();
        worst_step = worst_step.max((angle - step).abs());
    }
    // The cameras look at the object: its distance from the orbit's middle along the axis, in orbit radii,
    // is the tangent of the elevation.
    let towards: Vec<f64> = rows.iter().map(|row| dot(row.rotation[2], normal)).collect();
    let tilt = towards.iter().sum::<f64>() / count;
    let elevation = tilt.abs().asin().to_degrees();
    let mut measured = json!({
        "registered": rows.len(), "radius_spread_percent": 100.0 * radius_spread, "out_of_plane_percent": 100.0 * out_of_plane,
        "largest_step_error_degrees": worst_step, "elevation_degrees": elevation,
        "true_elevation_degrees": truth["elevation_degrees"],
    });
    let true_elevation = truth["elevation_degrees"].as_f64().unwrap_or(20.0);
    if radius_spread > 0.02 || out_of_plane > 0.02 || worst_step > 1.5 || (elevation - true_elevation).abs() > 2.0 {
        bail!("the recovered orbit is not the captured one: {measured}");
    }
    if run.join("pipeline.json").is_file() {
        let pipeline = read(&run.join("pipeline.json"))?;
        measured["pipeline"] = json!({"status": pipeline["status"], "closed": pipeline["closed"], "triangles": pipeline["triangles"],
            "genus": pipeline["genus"], "silhouette_iou_median": pipeline["photo_check"]["silhouette_iou_input_masks"]["median"]});
        let iou = pipeline["photo_check"]["silhouette_iou_input_masks"]["median"].as_f64().unwrap_or(0.0);
        if pipeline["status"] != "complete" || pipeline["closed"] != true || iou < 0.9 {
            bail!("the dense stages did not give a closed mesh that fits the photos: {measured}");
        }
    }
    Ok(measured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_has_photos_a_lens_and_the_true_orbit() {
        let root = std::env::temp_dir().join(format!("crispmesh-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root, 8, 160, 120).unwrap();
        let photo = crate::storage::open_image(root.join("photos/shot_003.png")).unwrap().to_luma8();
        assert_eq!((photo.width(), photo.height()), (160, 120));
        // Light backdrop in the corner, a dark and varied object in the middle.
        assert!(photo.get_pixel(2, 2).0[0] > 200);
        let middle: Vec<u8> = (60..100).flat_map(|x| (45..75).map(move |y| (x, y))).map(|(x, y)| photo.get_pixel(x, y).0[0]).collect();
        let (low, high) = (middle.iter().min().unwrap(), middle.iter().max().unwrap());
        assert!(*high < 150 && high - low > 30, "{low} {high}");
        let lens: Value = serde_json::from_str(&std::fs::read_to_string(root.join("lens.json")).unwrap()).unwrap();
        assert_eq!((lens["model"].as_str(), lens["calibration_width"].as_u64()), (Some("radialk3"), Some(160)));
        let truth: Value = serde_json::from_str(&std::fs::read_to_string(root.join("truth.json")).unwrap()).unwrap();
        assert_eq!(truth["views"].as_array().unwrap().len(), 8);
        assert!(write(&root, 8, 160, 120).is_err());
        std::fs::remove_dir_all(&root).unwrap();
        assert!((0..50).map(|n| texture([n as f64 * 0.1, 0.3, -0.2])).all(|v| (0.05..=0.51).contains(&v)));
    }
}
