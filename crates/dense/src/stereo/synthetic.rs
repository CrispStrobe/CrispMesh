//! A small analytic turntable scene in the stage's input format: port of
//! `scripts/turntable_mesh/synthetic_scene.py`. A unit sphere with a texture
//! fixed to its surface, darker than a plain backdrop, seen by a ring of
//! pinhole cameras. Used by tests and smoke runs; exact depth is known.
//!
//! Images, masks, cameras and depths follow the Python formulas; only the
//! sparse points differ (a Fibonacci lattice instead of NumPy's generator).

use std::path::Path;

use anyhow::Context;
use serde_json::json;

use crate::inputs::Plane;
use crate::npz::{self, Array, Data};

use super::level::LevelView;

/// Settings sized for the scene (`synthetic_scene.SMALL`).
pub const SMALL: [&str; 11] = [
    "sizes=64,128",
    "grid=96",
    "planes=48",
    "neighbours=4",
    "best_of=2",
    "vote_neighbours=4",
    "min_votes=2,2",
    "crop_padding=6",
    "hull_dilate=1",
    "windows=5,7",
    "aggregates=1,1",
];

type Mat3 = [[f64; 3]; 3];

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalised(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    a.map(|v| v / n)
}

/// World-to-camera rotation and translation looking at the origin, y down.
pub fn camera(angle: f64, elevation: f64, distance: f64) -> (Mat3, [f64; 3]) {
    let (a, e) = (angle.to_radians(), elevation.to_radians());
    let centre = [distance * e.cos() * a.cos(), distance * e.cos() * a.sin(), distance * e.sin()];
    let forward = normalised(centre.map(|v| -v));
    let right = normalised(cross(forward, [0.0, 0.0, 1.0]));
    let down = cross(forward, right);
    let rotation = [right, down, forward];
    let translation = [-dot(right, centre), -dot(down, centre), -dot(forward, centre)];
    (rotation, translation)
}

fn texture(p: [f64; 3]) -> f64 {
    let [x, y, z] = p;
    let value = ((9.0 * x + 2.0 * y).sin()
        + (11.0 * y - 3.0 * z).sin()
        + (13.0 * z + 5.0 * x).sin()
        + 0.7 * (31.0 * x - 17.0 * y + 23.0 * z).sin())
        / 3.7;
    0.38 + 0.22 * value
}

/// Grey image, mask and exact z-depth of the sphere for one camera, row-major.
pub fn render(rotation: &Mat3, translation: &[f64; 3], k: &[f64; 4], width: usize, height: usize) -> (Vec<f64>, Vec<bool>, Vec<f64>) {
    let centre = [0, 1, 2].map(|a| -(rotation[0][a] * translation[0] + rotation[1][a] * translation[1] + rotation[2][a] * translation[2]));
    let (mut gray, mut mask, mut depth) = (Vec::new(), Vec::new(), Vec::new());
    for y in 0..height {
        for x in 0..width {
            let ray = [(x as f64 + 0.5 - k[2]) / k[0], (y as f64 + 0.5 - k[3]) / k[1], 1.0];
            let direction = [0, 1, 2].map(|a| ray[0] * rotation[0][a] + ray[1] * rotation[1][a] + ray[2] * rotation[2][a]);
            let b = dot(direction, centre);
            let a = dot(direction, direction);
            let discriminant = b * b - a * (dot(centre, centre) - 1.0);
            let hit = discriminant > 0.0;
            let d = if hit { (-b - discriminant.max(0.0).sqrt()) / a } else { 0.0 };
            let point = [0, 1, 2].map(|n| centre[n] + direction[n] * d);
            gray.push(if hit { texture(point) } else { 0.82 });
            mask.push(hit);
            depth.push(d);
        }
    }
    (gray, mask, depth)
}

/// Writes the scene (`views` cameras, square images of `size` pixels) into a fresh directory.
pub fn write(output: &Path, views: usize, size: usize) -> anyhow::Result<()> {
    let (distance, elevation, focal) = (6.0, 10.0, 227.0 * size as f64 / 128.0);
    crate::storage::create_dir_all(output.join("images")).with_context(|| output.display().to_string())?;
    crate::storage::create_dir_all(output.join("masks"))?;
    let k = [focal, focal, size as f64 / 2.0, size as f64 / 2.0];
    let mut rows = Vec::new();
    let mut depths: Vec<(String, Array)> = Vec::new();
    for n in 0..views {
        let (rotation, translation) = camera(360.0 * n as f64 / views as f64, elevation, distance);
        let (gray, mask, depth) = render(&rotation, &translation, &k, size, size);
        let name = format!("view_{n:03}");
        let rgb: Vec<u8> = gray.iter().flat_map(|&g| [(255.0 * g.clamp(0.0, 1.0)) as u8; 3]).collect();
        crate::storage::save_png(output.join(format!("images/{name}.png")), size, size, 3, &rgb)?;
        let luma: Vec<u8> = mask.iter().map(|&m| if m { 255 } else { 0 }).collect();
        crate::storage::save_png(output.join(format!("masks/{name}.png")), size, size, 1, &luma)?;
        rows.push(json!({
            "name": name, "source": format!("{name}.png"), "image": format!("images/{name}.png"),
            "mask": format!("masks/{name}.png"), "width": size, "height": size, "k": k,
            "rotation": rotation, "translation": translation,
        }));
        depths.push((name, Array::new(&[size, size], Data::F32(depth.iter().map(|&d| d as f32).collect()))));
    }
    // Points on the sphere for the scene box and the viewing directions.
    let count = 400;
    let golden = std::f64::consts::PI * (3.0 - 5.0f64.sqrt());
    let mut sparse = Vec::with_capacity(count * 3);
    for n in 0..count {
        let z = 1.0 - 2.0 * (n as f64 + 0.5) / count as f64;
        let r = (1.0 - z * z).sqrt();
        sparse.extend_from_slice(&[r * (golden * n as f64).cos(), r * (golden * n as f64).sin(), z]);
    }
    crate::storage::write(output.join("sparse_points.npy"), crate::npz::npy_f64(&[count, 3], &sparse))?;
    let members: Vec<(&str, &Array)> = depths.iter().map(|(name, array)| (name.as_str(), array)).collect();
    npz::write(&output.join("exact_depths.npz"), &members, true)?;
    crate::storage::write(output.join("cameras.json"), serde_json::to_string_pretty(&json!({ "views": rows }))? + "\n")?;
    Ok(())
}

/// Exact z-depth of the unit sphere at every pixel of a level view (0 where the ray misses).
pub fn sphere_depth(view: &LevelView) -> Plane<f32> {
    let origin = view.camera.centre().map(|v| v as f64);
    let mut depth = Plane::<f32>::new(view.width, view.height);
    for y in 0..view.height {
        for x in 0..view.width {
            let at_one = view.unproject(x, y, 1.0);
            let d = [0, 1, 2].map(|a| at_one[a] as f64 - origin[a]);
            let (a, b) = (d.iter().map(|v| v * v).sum::<f64>(), d.iter().zip(&origin).map(|(d, o)| d * o).sum::<f64>());
            let c = origin.iter().map(|v| v * v).sum::<f64>() - 1.0;
            let discriminant = b * b - a * c;
            if discriminant > 0.0 {
                depth.data[y * view.width + x] = ((-b - discriminant.sqrt()) / a) as f32;
            }
        }
    }
    depth
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const USAGE: &str = "usage: crispmesh-dense synthetic --output DIR [--views N] [--size PIXELS]      an inputs directory (sphere)
       crispmesh-dense synthetic --capture --output DIR [--views N] [--width W] [--height H] [--asymmetric]   raw photos, lens.json, truth.json
       crispmesh-dense synthetic --verify RUN --truth CAPTURE_DIR      check a run made from such a capture";

/// The `synthetic` command: an inputs directory of the analytic sphere (and the
/// settings sized for it), or a capture of raw photos with its lens, or the
/// check of a run made from such a capture.
#[cfg(not(target_arch = "wasm32"))]
pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    use std::path::PathBuf;
    let (mut output, mut views, mut size, mut capture, mut asymmetric) = (None, None, 128usize, false, false);
    let (mut width, mut height, mut verify, mut truth) = (640usize, 480usize, None, None);
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().ok_or_else(|| anyhow::anyhow!("{flag} needs a value\n{USAGE}"));
        match flag.as_str() {
            "--output" => output = Some(PathBuf::from(value()?)),
            "--views" => views = Some(value()?.parse::<usize>()?),
            "--size" => size = value()?.parse()?,
            "--width" => width = value()?.parse()?,
            "--height" => height = value()?.parse()?,
            "--capture" => capture = true,
            "--asymmetric" => asymmetric = true,
            "--verify" => verify = Some(PathBuf::from(value()?)),
            "--truth" => truth = Some(PathBuf::from(value()?)),
            other => anyhow::bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    if let Some(run) = verify {
        let truth = truth.ok_or_else(|| anyhow::anyhow!("--verify needs --truth CAPTURE_DIR\n{USAGE}"))?;
        println!("{}", serde_json::to_string_pretty(&super::capture::verify(&run, &truth)?)?);
        return Ok(());
    }
    let output = output.ok_or_else(|| anyhow::anyhow!("--output is required\n{USAGE}"))?;
    anyhow::ensure!(!crate::storage::exists(&output), "output directory exists: {}", output.display());
    if capture {
        super::capture::write_object(&output, views.unwrap_or(40), width, height, asymmetric)?;
        println!("photos: {}", output.join("photos").display());
        println!("calibration: {}", output.join("lens.json").display());
        return Ok(());
    }
    write(&output, views.unwrap_or(24), size)?;
    println!("suggested overrides: {}", SMALL.iter().map(|item| format!("--set {item}")).collect::<Vec<_>>().join(" "));
    Ok(())
}

/// A fresh scene in a temporary directory; the caller removes it.
pub fn temporary(tag: &str, views: usize, size: usize) -> anyhow::Result<std::path::PathBuf> {
    let root = std::env::temp_dir().join(format!("crispmesh-synthetic-{tag}-{}", std::process::id()));
    let _ = crate::storage::remove_dir_all(&root);
    write(&root.join("inputs"), views, size)?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::Inputs;
    use crate::stereo::options;

    #[test]
    fn scene_loads_with_one_common_canvas() {
        let root = temporary("load", 12, 64).unwrap();
        let overrides: Vec<String> = SMALL.iter().map(|s| s.to_string()).collect();
        let config = options::build(None, &overrides).unwrap();
        let inputs = Inputs::load(&root.join("inputs"), &config).unwrap();
        assert_eq!(inputs.count(), 12);
        let (w, h) = (inputs.boxes[0][2] - inputs.boxes[0][0], inputs.boxes[0][3] - inputs.boxes[0][1]);
        assert!(inputs.boxes.iter().all(|b| b[2] - b[0] == w && b[3] - b[1] == h) && w % 2 == 0 && h % 2 == 0);
        assert_eq!(inputs.longest, w.max(h));
        // The stretch maps the object's 1st..99th grey percentile to 0..1; the backdrop is brighter.
        let (gray, mask) = (&inputs.gray[0], &inputs.masks[0]);
        let inside: Vec<f32> = gray.data.iter().zip(&mask.data).filter(|(_, &m)| m != 0).map(|(&g, _)| g).collect();
        let below = inside.iter().filter(|&&g| g < 0.0).count() as f64 / inside.len() as f64;
        let above = inside.iter().filter(|&&g| g > 1.0).count() as f64 / inside.len() as f64;
        assert!(below <= 0.011 && above <= 0.011, "{below} {above}");
        assert!(gray.data[0] > 1.0);
        // A ring of 12 views: neighbours are the adjacent cameras, 30 degrees apart.
        let neighbours = inputs.neighbours(0, 4, &config).unwrap();
        assert_eq!(neighbours.len(), 2);
        assert!(neighbours.contains(&1) && neighbours.contains(&11));
        assert!((inputs.angles[0][1] - 30.0).abs() < 1.0);
        crate::storage::remove_dir_all(&root).unwrap();
    }
}
