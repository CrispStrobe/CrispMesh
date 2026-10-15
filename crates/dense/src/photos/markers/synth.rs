//! A synthetic turntable capture with the marker mat: a dark textured object
//! standing on the printed mat, which turns under a fixed camera with a
//! distorting lens. Rendered by casting rays, with defocus blur, exposure and
//! sensor noise. The exact poses are known, in the mat's millimetres.
//!
//! This stands in for photographs of a printed mat in tests and measurements.
//! It is a rendering: paper curl, glossy toner, uneven light and a real
//! lens's residual errors are not in it.

use std::path::Path;

use anyhow::Context;
use serde_json::json;

use crate::photos::util;

use super::linalg::{apply, cross, dot, mul, norm, rodrigues, scale, sub, transpose, unit, Camera, V3};
use super::mat::Mat;
use super::pose::Pose;

#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub views: usize,
    pub width: usize,
    pub height: usize,
    /// Camera height angle above the mat plane, degrees.
    pub elevation_deg: f64,
    /// Distance of the camera from the point it looks at, millimetres.
    pub distance: f64,
    /// Horizontal field of view in degrees; fixes the focal length.
    pub field_of_view_deg: f64,
    pub k: [f64; 3],
    /// The object: an upright cylinder with a rounded top. Radius 0 leaves the mat empty.
    pub object_radius: f64,
    pub object_height: f64,
    /// Standard deviation of the defocus blur in pixels.
    pub blur: f64,
    /// Standard deviation of the sensor noise in grey levels.
    pub noise: f64,
    /// Exposure factor on the linear image (1 is a white page at grey 225).
    pub exposure: f64,
    /// Fraction of a full turn covered; the mat turns anticlockwise seen from above.
    pub turn: f64,
    pub seed: u64,
    /// Brightness of the surface around the mat, 0 (black) to 1; the paper is 0.9.
    pub table: f64,
}

impl Default for Capture {
    fn default() -> Self {
        Capture {
            views: 36,
            width: 1200,
            height: 900,
            elevation_deg: 25.0,
            distance: 420.0,
            field_of_view_deg: 42.0,
            k: [-0.12, 0.25, -0.6],
            object_radius: 32.0,
            object_height: 90.0,
            blur: 0.8,
            noise: 2.0,
            exposure: 1.0,
            turn: 1.0,
            seed: 7,
            table: 0.66,
        }
    }
}

impl Capture {
    pub fn camera(&self) -> Camera {
        let f = self.width as f64 / 2.0 / (self.field_of_view_deg.to_radians() / 2.0).tan();
        Camera { fx: f, fy: f * 1.0005, cx: (self.width as f64 - 1.0) / 2.0 + 3.3, cy: (self.height as f64 - 1.0) / 2.0 - 2.1, k: self.k }
    }

    /// The true pose of view `n`: mat frame to camera frame.
    pub fn pose(&self, n: usize) -> Pose {
        let angle = self.turn * 2.0 * std::f64::consts::PI * n as f64 / self.views as f64;
        let elevation = self.elevation_deg.to_radians();
        let target = [0.0, 0.0, 0.35 * self.object_height];
        let centre = [0.0, -self.distance * elevation.cos(), target[2] + self.distance * elevation.sin()];
        let forward = unit(sub(target, centre));
        let right = unit(cross(forward, [0.0, 0.0, 1.0]));
        let down = cross(forward, right);
        let fixed = [right, down, forward];
        // The mat turns by `angle` about its own Z axis: world = Rz(angle) * mat.
        let rotation = mul(&fixed, &rodrigues([0.0, 0.0, angle]));
        Pose { rotation, translation: scale(apply(&fixed, centre), -1.0) }
    }

    /// Where a ray from `origin` along `direction` (mat frame) first meets the object, with the surface point.
    fn hit_object(&self, origin: V3, direction: V3) -> Option<(f64, V3)> {
        let (r, top) = (self.object_radius, self.object_height - self.object_radius);
        if r <= 0.0 {
            return None;
        }
        let mut best: Option<f64> = None;
        let mut consider = |t: f64, accept: &dyn Fn(V3) -> bool| {
            if t > 1e-6 && best.is_none_or(|b| t < b) {
                let p = [origin[0] + t * direction[0], origin[1] + t * direction[1], origin[2] + t * direction[2]];
                if accept(p) {
                    best = Some(t);
                }
            }
        };
        // Side of the cylinder.
        let (a, b, c) = (
            direction[0] * direction[0] + direction[1] * direction[1],
            2.0 * (origin[0] * direction[0] + origin[1] * direction[1]),
            origin[0] * origin[0] + origin[1] * origin[1] - r * r,
        );
        let discriminant = b * b - 4.0 * a * c;
        if a > 1e-12 && discriminant >= 0.0 {
            for sign in [-1.0, 1.0] {
                consider((-b + sign * discriminant.sqrt()) / (2.0 * a), &|p| p[2] >= 0.0 && p[2] <= top);
            }
        }
        // Rounded top.
        let to_centre = sub(origin, [0.0, 0.0, top]);
        let (b, c) = (2.0 * dot(to_centre, direction), dot(to_centre, to_centre) - r * r);
        let discriminant = b * b - 4.0 * c;
        if discriminant >= 0.0 {
            for sign in [-1.0, 1.0] {
                consider((-b + sign * discriminant.sqrt()) / 2.0, &|p| p[2] >= top);
            }
        }
        best.map(|t| (t, [origin[0] + t * direction[0], origin[1] + t * direction[1], origin[2] + t * direction[2]]))
    }

    /// Linear brightness (0..1) seen along a ray, and whether it is the object.
    fn shade(&self, mat: &Mat, origin: V3, direction: V3) -> (f64, bool) {
        if let Some((_, p)) = self.hit_object(origin, direction) {
            // A dark surface with texture fixed to it at several scales.
            let texture = 0.5
                + 0.2 * (0.31 * p[0] + 0.17 * p[2]).sin() * (0.23 * p[1] - 0.29 * p[2]).cos()
                + 0.18 * (0.9 * p[0] - 0.7 * p[1] + 0.5 * p[2]).sin()
                + 0.12 * (2.1 * p[0] + 1.7 * p[1] + 2.9 * p[2]).sin() * (1.3 * p[2] - 2.3 * p[0]).cos();
            return (0.06 + 0.2 * texture.clamp(0.0, 1.0), true);
        }
        if direction[2] >= -1e-9 {
            return (0.72, false);
        }
        let t = -origin[2] / direction[2];
        let (x, y) = (origin[0] + t * direction[0], origin[1] + t * direction[1]);
        let paper = match mat.colour(x, y) {
            Some(white) => 0.07 + 0.83 * white,
            // A light table with a faint grain beyond the page.
            None => self.table * (1.0 + 0.06 * (0.05 * x).sin() * (0.043 * y).cos()),
        };
        // Soft contact shadow around the foot of the object.
        let distance = x.hypot(y) / self.object_radius.max(1e-9);
        let shadow = if self.object_radius > 0.0 && distance < 1.6 { 0.55 + 0.45 * ((distance - 1.0) / 0.6).clamp(0.0, 1.0) } else { 1.0 };
        (paper * shadow, false)
    }

    /// One photo as 8-bit grey, with the object's mask (0/255).
    pub fn render(&self, mat: &Mat, n: usize) -> (Vec<u8>, Vec<u8>) {
        let (w, h) = (self.width, self.height);
        let camera = self.camera();
        let pose = self.pose(n);
        let to_mat = transpose(&pose.rotation);
        let origin = pose.centre();
        let mut linear = vec![0f32; w * h];
        let mut mask = vec![0u8; w * h];
        const SAMPLES: usize = 3;
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut object) = (0.0, 0);
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let pixel =
                            [x as f64 + (sx as f64 + 0.5) / SAMPLES as f64 - 0.5, y as f64 + (sy as f64 + 0.5) / SAMPLES as f64 - 0.5];
                        let ray = camera.undistort(pixel);
                        let direction = apply(&to_mat, [ray[0], ray[1], 1.0]);
                        let (value, is_object) = self.shade(mat, origin, scale(direction, 1.0 / norm(direction)));
                        sum += value;
                        object += is_object as usize;
                    }
                }
                linear[y * w + x] = (sum / (SAMPLES * SAMPLES) as f64) as f32;
                mask[y * w + x] = if object * 2 > SAMPLES * SAMPLES { 255 } else { 0 };
            }
        }
        if self.blur > 0.05 {
            linear = blur(&linear, w, h, self.blur);
        }
        // Splitmix-style generator; two uniform numbers give one normal one.
        let mut state = self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(n as u64);
        let mut uniform = move || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        let gray = linear
            .iter()
            .map(|&v| {
                let normal = (-2.0 * uniform().max(1e-300).ln()).sqrt() * (2.0 * std::f64::consts::PI * uniform()).cos();
                (250.0 * v as f64 * self.exposure + self.noise * normal).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        (gray, mask)
    }

    /// Writes `photos/shot_N.png`, `masks/capture_NNNN.png`, `lens.json` (the true lens) and
    /// `truth.json` (the true poses) into a fresh folder, with the mat's description as `mat.json`.
    pub fn write(&self, mat: &Mat, folder: &Path, threads: usize) -> anyhow::Result<()> {
        crate::photos::fs::create_dir_all(folder.join("photos")).with_context(|| folder.display().to_string())?;
        crate::photos::fs::create_dir_all(folder.join("masks"))?;
        util::parallel(
            self.views,
            threads,
            |n| {
                let (gray, mask) = self.render(mat, n);
                let rgb: Vec<u8> = gray.iter().flat_map(|&g| [g, g, g]).collect();
                util::save_rgb(&folder.join(format!("photos/shot_{n}.png")), self.width, self.height, rgb)?;
                util::save_gray(&folder.join(format!("masks/capture_{n:04}.png")), self.width, self.height, mask)
            },
            &mut |_| Ok(()),
        )?;
        let camera = self.camera();
        let lens = json!({
            "schema": "crispmesh_lens_calibration_v1", "model": "radialk3", "calibration_width": self.width, "calibration_height": self.height,
            "fx": camera.fx, "fy": camera.fy, "cx": camera.cx, "cy": camera.cy, "k1": self.k[0], "k2": self.k[1], "k3": self.k[2],
            "principal_point_convention": "pixel_centre", "provenance": {"source": "synthetic marker-mat capture"},
        });
        util::write_json(&folder.join("lens.json"), &lens, 1)?;
        let views: Vec<_> = (0..self.views)
            .map(|n| {
                let pose = self.pose(n);
                json!({"source": format!("capture_{n:04}.png"), "rotation": pose.rotation, "translation": pose.translation})
            })
            .collect();
        let truth = json!({
            "schema": "crispmesh_marker_capture_truth_v1", "unit": "mm", "views": views,
            "object": {"radius": self.object_radius, "height": self.object_height},
            "capture": {"elevation_deg": self.elevation_deg, "distance": self.distance, "blur": self.blur, "noise": self.noise, "exposure": self.exposure},
        });
        util::write_json(&folder.join("truth.json"), &truth, 1)?;
        util::write_json(&folder.join("mat.json"), &mat.to_json(), 1)
    }
}

/// Separable Gaussian blur with the edge pixels repeated.
fn blur(source: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    let radius = (3.0 * sigma).ceil() as i64;
    let kernel: Vec<f32> = (-radius..=radius).map(|d| (-(d * d) as f64 / (2.0 * sigma * sigma)).exp() as f32).collect();
    let total: f32 = kernel.iter().sum();
    let mut rows = vec![0f32; source.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            for (k, weight) in kernel.iter().enumerate() {
                let u = (x as i64 + k as i64 - radius).clamp(0, width as i64 - 1) as usize;
                sum += weight * source[y * width + u];
            }
            rows[y * width + x] = sum / total;
        }
    }
    let mut out = vec![0f32; source.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            for (k, weight) in kernel.iter().enumerate() {
                let v = (y as i64 + k as i64 - radius).clamp(0, height as i64 - 1) as usize;
                sum += weight * rows[v * width + x];
            }
            out[y * width + x] = sum / total;
        }
    }
    out
}
