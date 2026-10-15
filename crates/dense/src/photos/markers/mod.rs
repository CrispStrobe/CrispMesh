//! Camera provider `markers`: poses from a printed mat of square fiducials
//! that lies under the object (`docs/MARKER-MAT.md`).
//!
//! | Module | Content |
//! | --- | --- |
//! | `mat` | the mat: layout, description (`crispmesh_marker_mat_v1`), SVG, PDF and PNG |
//! | `detect` | marker detection in a photo |
//! | `pose` | pose per photo from all visible corners, print aspect from all photos, the solution |
//! | `synth` | a rendered capture with known poses, for tests and measurements |
//! | `linalg` | the small linear algebra and the lens model these need |
//! | `provider` | the provider as the `photos` command runs it (not in a browser build) |
//!
//! Everything but `provider` is plain Rust without external programs and
//! builds for every target. The scene comes out in the mat's frame and in
//! millimetres: origin at the page centre, +Z up from the page, right-handed.

pub mod detect;
pub mod linalg;
pub mod mat;
pub mod pose;
pub mod synth;

pub mod provider;

use crate::photos::fs::Stored as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::inputs::{median_f64, Plane};

use self::linalg::Camera;
use self::mat::Mat;
use self::pose::Pose;

pub const MAT_USAGE: &str = "\
usage: crispmesh-dense mat --size a4|letter|a3 --output DIR
           writes crispmesh-marker-mat-SIZE.pdf, .svg, .png (300 dpi) and .json (crispmesh_marker_mat_v1)
       crispmesh-dense mat --detect PHOTO --mat MAT.json [--calibration LENS.json]
           prints the markers found in one photo as JSON
       crispmesh-dense mat --capture DIR [--size a4 | --mat MAT.json] [--views 36] [--width 1200] [--height 900]
           [--elevation 25] [--distance 420] [--blur 0.8] [--noise 2] [--exposure 1] [--object-radius 32]
           [--object-height 90] [--turn 1] [--seed 7] [--table 0.66] [--threads 2]
           renders a synthetic turntable capture of the mat with known poses (photos/, masks/, lens.json, truth.json)
       crispmesh-dense mat --compare CAMERAS.json --truth TRUTH.json
           rotation and position differences between a scene's cameras and a capture's true poses

The mat lies under the object and turns with it. Print at 100 % and check the scale bar. See docs/MARKER-MAT.md.";

/// A photo as the grey plane the detector reads (Pillow's luma of the 8-bit RGB values).
pub fn gray_of(path: &Path) -> anyhow::Result<Plane<u8>> {
    Ok(crate::photos::staging::open_photo(path)?.gray())
}

fn detections_json(found: &[detect::Detection]) -> Value {
    json!(found
        .iter()
        .map(|d| json!({"id": d.id, "corners": d.corners, "corrected_cells": d.corrected, "contrast": d.contrast}))
        .collect::<Vec<_>>())
}

/// Poses listed in a `cameras.json` or a capture's `truth.json`, by source photo name.
fn poses_by_source(value: &Value) -> anyhow::Result<Vec<(String, Pose)>> {
    let mut out = Vec::new();
    for view in value["views"].as_array().ok_or_else(|| anyhow!("no views"))? {
        let rotation: [[f64; 3]; 3] = serde_json::from_value(view["rotation"].clone()).context("rotation")?;
        let translation: [f64; 3] = serde_json::from_value(view["translation"].clone()).context("translation")?;
        out.push((view["source"].as_str().unwrap_or_default().to_string(), Pose { rotation, translation }));
    }
    Ok(out)
}

/// Differences between the cameras of a scene and the true poses of a synthetic capture, both in the mat's frame.
pub fn compare_with_truth(cameras: &Value, truth: &Value) -> anyhow::Result<Value> {
    let (ours, theirs) = (poses_by_source(cameras)?, poses_by_source(truth)?);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for (source, pose) in &ours {
        let Some((_, exact)) = theirs.iter().find(|(name, _)| name == source) else { bail!("{source} is not in the truth") };
        a.push(*pose);
        b.push(*exact);
    }
    if a.is_empty() {
        bail!("no views to compare");
    }
    let differences = pose::compare(&a, &b);
    let angles: Vec<f64> = differences.iter().map(|d| d.0).collect();
    let positions: Vec<f64> = differences.iter().map(|d| d.1).collect();
    let distance = b.iter().map(|p| linalg::norm(p.centre())).sum::<f64>() / b.len() as f64;
    // Scale: distances between all pairs of camera centres, ours over the true ones.
    let (mut ours_sum, mut true_sum) = (0.0, 0.0);
    for i in 0..a.len() {
        for j in i + 1..a.len() {
            ours_sum += linalg::norm(linalg::sub(a[i].centre(), a[j].centre()));
            true_sum += linalg::norm(linalg::sub(b[i].centre(), b[j].centre()));
        }
    }
    let largest = |values: &[f64]| values.iter().cloned().fold(0.0, f64::max);
    Ok(json!({
        "views": a.len(), "of": theirs.len(),
        "rotation_error_deg": {"median": median_f64(&mut angles.clone()), "max": largest(&angles)},
        "position_error_mm": {"median": median_f64(&mut positions.clone()), "max": largest(&positions)},
        "position_error_percent_of_distance": {"median": 100.0 * median_f64(&mut positions.clone()) / distance, "max": 100.0 * largest(&positions) / distance},
        "scale_ratio": if true_sum > 0.0 { json!(ours_sum / true_sum) } else { Value::Null },
    }))
}

/// Entry point of the `mat` subcommand.
pub fn command(arguments: &[String]) -> anyhow::Result<()> {
    let mut values = std::collections::HashMap::new();
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        if flag == "--help" || flag == "-h" {
            println!("{MAT_USAGE}");
            return Ok(());
        }
        let name = flag.strip_prefix("--").ok_or_else(|| anyhow!("unexpected argument {flag:?}\n{MAT_USAGE}"))?;
        values.insert(name.to_string(), rest.next().ok_or_else(|| anyhow!("--{name} needs a value"))?.clone());
    }
    let text = |name: &str| values.get(name).cloned();
    let number = |name: &str, default: f64| -> anyhow::Result<f64> {
        match values.get(name) {
            Some(value) => value.parse::<f64>().map_err(|_| anyhow!("--{name} needs a number")),
            None => Ok(default),
        }
    };
    let mat = || -> anyhow::Result<Mat> {
        match text("mat") {
            Some(path) => Mat::load(Path::new(&path)),
            None => Mat::standard(&text("size").unwrap_or_else(|| "a4".into())),
        }
    };
    if let Some(folder) = text("capture") {
        let d = synth::Capture::default();
        let capture = synth::Capture {
            views: number("views", d.views as f64)? as usize,
            width: number("width", d.width as f64)? as usize,
            height: number("height", d.height as f64)? as usize,
            elevation_deg: number("elevation", d.elevation_deg)?,
            distance: number("distance", d.distance)?,
            blur: number("blur", d.blur)?,
            noise: number("noise", d.noise)?,
            exposure: number("exposure", d.exposure)?,
            object_radius: number("object-radius", d.object_radius)?,
            object_height: number("object-height", d.object_height)?,
            turn: number("turn", d.turn)?,
            seed: number("seed", d.seed as f64)? as u64,
            table: number("table", d.table)?,
            ..d
        };
        if PathBuf::from(&folder).stored() {
            bail!("output exists: {folder}");
        }
        capture.write(&mat()?, Path::new(&folder), number("threads", 2.0)? as usize)?;
        println!("{}", json!({"capture": folder, "views": capture.views, "width": capture.width, "height": capture.height}));
    } else if let Some(photo) = text("detect") {
        let camera = match text("calibration") {
            Some(path) => {
                let gray = gray_of(Path::new(&photo))?;
                let calibration = crate::photos::calibration::load_calibration(Path::new(&path))?;
                let scaled = crate::photos::calibration::scale_calibration(&calibration, gray.width as u32, gray.height as u32)?;
                Some(Camera::from_lens(&scaled.lens(gray.width as u32, gray.height as u32)))
            }
            None => None,
        };
        let found = detect::detect(&gray_of(Path::new(&photo))?, &mat()?, camera.as_ref(), &detect::Settings::default());
        println!("{}", serde_json::to_string(&detections_json(&found))?);
    } else if let Some(cameras) = text("compare") {
        let truth = text("truth").ok_or_else(|| anyhow!("--compare needs --truth"))?;
        let report =
            compare_with_truth(&crate::photos::util::read_json(Path::new(&cameras))?, &crate::photos::util::read_json(Path::new(&truth))?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let folder = text("output").ok_or_else(|| anyhow!("{MAT_USAGE}"))?;
        let mat = mat()?;
        mat.write(Path::new(&folder))?;
        println!(
            "{}",
            json!({"mat": mat.name, "markers": mat.markers.len(), "marker_size_mm": mat.marker_size, "object_radius_mm": mat.object_radius, "output": folder})
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
