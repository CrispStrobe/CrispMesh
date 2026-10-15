//! Dark-object/bright-background cleanup of small enclosed mask holes: a port
//! of `scripts/turntable_mesh/silhouette_cleanup.py`.
//!
//! A hole (4-connected background that does not reach the image border) is
//! filled only when it has at most 2048 pixels and every photo pixel in it has
//! a luminance of at most 128. A photo whose filled pixels exceed the budget
//! fails, and no mask is published unless every photo passes.

use crate::photos::fs::Stored as _;
use std::path::Path;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::inputs::Plane;

use super::coarse::label;
use super::staging::{file_name, open_binary_mask, open_photo, save_mask};
use super::util;

pub const MAXIMUM_HOLE_PIXELS: usize = 2048;
pub const MAXIMUM_LUMINANCE: u8 = 128;

pub fn validate_budget(value: f64) -> anyhow::Result<f64> {
    if !value.is_finite() || !(0.0..=0.05).contains(&value) {
        bail!("maximum_total_filled_foreground_fraction must be finite in 0..0.05");
    }
    Ok(value)
}

/// Fill only small enclosed background whose every photo pixel is dark.
/// `gray` is Pillow's `convert("L")` of the photo, `mask` is 0/1.
pub fn clean_holes(gray: &Plane<u8>, mask: &Plane<u8>, budget: f64) -> anyhow::Result<(Plane<u8>, Value)> {
    let budget = validate_budget(budget)?;
    let (width, height) = (mask.width, mask.height);
    let original = mask.data.iter().filter(|&&m| m != 0).count();
    if (gray.width, gray.height) != (width, height) || original == 0 {
        bail!("mask must be nonempty boolean with matching RGB dimensions");
    }
    let background = Plane { width, height, data: mask.data.iter().map(|&m| (m == 0) as u8).collect() };
    let (labels, count) = label(&background, false);
    let mut boundary = vec![false; count + 1];
    for x in 0..width {
        boundary[labels.data[x] as usize] = true;
        boundary[labels.data[(height - 1) * width + x] as usize] = true;
    }
    for y in 0..height {
        boundary[labels.data[y * width] as usize] = true;
        boundary[labels.data[y * width + width - 1] as usize] = true;
    }
    let (mut areas, mut brightest) = (vec![0usize; count + 1], vec![0u8; count + 1]);
    for (index, &component) in labels.data.iter().enumerate() {
        areas[component as usize] += 1;
        brightest[component as usize] = brightest[component as usize].max(gray.data[index]);
    }
    let mut fill = vec![false; count + 1];
    let mut rows = Vec::new();
    let mut filled_pixels = 0;
    for component in 1..=count {
        if boundary[component] {
            continue;
        }
        let filled = areas[component] <= MAXIMUM_HOLE_PIXELS && brightest[component] <= MAXIMUM_LUMINANCE;
        rows.push(json!({"label": component, "pixels": areas[component], "maximum_luminance": brightest[component], "filled": filled}));
        if filled {
            fill[component] = true;
            filled_pixels += areas[component];
        }
    }
    if filled_pixels as f64 > original as f64 * budget {
        bail!("total filled pixels exceed {} fraction original foreground", python_general(budget));
    }
    let clean =
        Plane { width, height, data: mask.data.iter().zip(&labels.data).map(|(&m, &l)| (m != 0 || fill[l as usize]) as u8).collect() };
    Ok((clean, json!({"original_foreground_pixels": original, "filled_pixels": filled_pixels, "enclosed_components": rows})))
}

/// Python's `{:g}` for the budgets that can occur here (0..0.05).
fn python_general(value: f64) -> String {
    let text = format!("{:.6}", value);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() {
        "0".into()
    } else {
        text.into()
    }
}

/// Cleans `<masks>/<photo name>.png` for every photo in `images` and writes
/// `<output>/masks/<photo name>.png`, `frozen.json` and `result.json`.
pub fn run(
    images: &Path,
    masks: &Path,
    output: &Path,
    budget: f64,
    threads: usize,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    let budget = validate_budget(budget)?;
    if output.stored() {
        bail!("output exists: {}", output.display());
    }
    let mut paths = Vec::new();
    for path in crate::photos::fs::list(images).with_context(|| images.display().to_string())? {
        let suffix = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["png", "jpg", "jpeg"].contains(&suffix.as_str()) {
            paths.push(path);
        }
    }
    paths.sort();
    if !(1..=crate::photos::MOST_PHOTOS).contains(&paths.len()) {
        bail!("requires 1..{} RGB photos", crate::photos::MOST_PHOTOS);
    }
    let mask_path = |photo: &Path| masks.join(format!("{}.png", file_name(photo)));
    let sealed: Vec<_> = paths.iter().cloned().chain(paths.iter().map(|p| mask_path(p))).collect();
    let digests = |watch: &mut dyn FnMut(usize) -> anyhow::Result<()>| -> anyhow::Result<Value> {
        let hashes = util::parallel(sealed.len(), threads, |n| util::sha256_file(&sealed[n]), &mut |_| watch(0))?;
        Ok(Value::Object(sealed.iter().zip(hashes).map(|(p, h)| (p.to_string_lossy().to_string(), json!(h))).collect()))
    };
    let configuration = json!({
        "maximum_hole_pixels": MAXIMUM_HOLE_PIXELS, "maximum_all_pixel_luminance": MAXIMUM_LUMINANCE,
        "maximum_total_filled_foreground_fraction": budget, "grayscale": "Pillow RGB.convert L",
        "background_connectivity": 4, "dark_object_bright_background": true,
    });
    let before = digests(watch)?;
    crate::photos::fs::create_dir_all(output)?;
    util::write_json(&output.join("frozen.json"), &json!({"configuration": configuration, "source_hashes_before": before}), 2)?;
    let mut report = json!({
        "schema": "photo_dark_hole_cleanup_v1", "status": "running", "configuration": configuration,
        "quality_accepted": false, "reference_used": false, "rows": [],
    });
    let attempt = (|| -> anyhow::Result<()> {
        // All masks must pass the same policy before any mask is published.
        let prepared = util::parallel(
            paths.len(),
            threads,
            |n| {
                let photo = open_photo(&paths[n])?;
                if !photo.upright {
                    bail!("upright RGB required");
                }
                let (clean, row) = clean_holes(&photo.gray(), &open_binary_mask(&mask_path(&paths[n]))?, budget)?;
                Ok((clean, row))
            },
            watch,
        )?;
        let mut rows = Vec::new();
        for (path, (_, row)) in paths.iter().zip(&prepared) {
            let mut row = row.clone();
            row["name"] = json!(file_name(path));
            rows.push(row);
        }
        report["rows"] = json!(rows);
        let after = digests(watch)?;
        if before != after {
            bail!("source changed during cleanup");
        }
        crate::photos::fs::create_dir_all(output.join("masks"))?;
        let mut outputs = serde_json::Map::new();
        for (path, (clean, _)) in paths.iter().zip(&prepared) {
            let target = output.join("masks").join(format!("{}.png", file_name(path)));
            save_mask(&target, clean)?;
            outputs.insert(target.to_string_lossy().to_string(), json!(util::sha256_file(&target)?));
        }
        report["status"] = json!("complete_unreviewed");
        report["source_hashes_before"] = before.clone();
        report["source_hashes_after"] = after;
        report["output_hashes"] = Value::Object(outputs);
        Ok(())
    })();
    if let Err(error) = &attempt {
        report["status"] = json!("failed");
        report["error"] = json!({"type": "ValueError", "message": error.to_string()});
    }
    util::write_json(&output.join("result.json"), &report, 2)?;
    attempt.map(|()| report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(width: usize, height: usize, value: u8) -> Plane<u8> {
        Plane { width, height, data: vec![value; width * height] }
    }

    fn paint(plane: &mut Plane<u8>, rows: std::ops::Range<usize>, columns: std::ops::Range<usize>, value: u8) {
        for y in rows {
            for x in columns.clone() {
                plane.data[y * plane.width + x] = value;
            }
        }
    }

    #[test]
    fn small_dark_hole_filled_bright_large_boundary_preserved() {
        let mut gray = plane(150, 150, 90);
        let mut mask = plane(150, 150, 1);
        paint(&mut mask, 20..23, 20..23, 0);
        paint(&mut mask, 30..33, 30..33, 0);
        gray.data[31 * 150 + 31] = 129;
        paint(&mut mask, 50..100, 50..100, 0);
        paint(&mut mask, 0..5, 120..125, 0);
        let (clean, report) = clean_holes(&gray, &mask, 0.01).unwrap();
        let mut expected = mask.clone();
        paint(&mut expected, 20..23, 20..23, 1);
        assert_eq!(clean, expected);
        assert_eq!(report["filled_pixels"], 9);
        assert_eq!(report["enclosed_components"].as_array().unwrap().len(), 3);
        assert_eq!(report["enclosed_components"][0], json!({"label": 2, "pixels": 9, "maximum_luminance": 90, "filled": true}));
        assert!(clean.at(31, 31) == 0 && clean.at(70, 70) == 0 && clean.at(122, 2) == 0);
    }

    #[test]
    fn threshold_equality_and_total_budget_fail_closed() {
        let gray = plane(100, 100, 128);
        let mut mask = plane(100, 100, 1);
        paint(&mut mask, 20..25, 20..25, 0);
        let (clean, report) = clean_holes(&gray, &mask, 0.01).unwrap();
        assert_eq!(report["filled_pixels"], 25);
        assert_eq!(clean.at(22, 22), 1);
        paint(&mut mask, 40..50, 40..50, 0);
        let error = clean_holes(&gray, &mask, 0.01).unwrap_err().to_string();
        assert!(error.contains("0.01 fraction"), "{error}");
    }

    #[test]
    fn explicit_budget_bounds_and_bright_hole_preservation() {
        let mut gray = plane(100, 100, 90);
        let mut mask = plane(100, 100, 1);
        paint(&mut mask, 20..32, 20..32, 0);
        paint(&mut mask, 50..55, 50..55, 0);
        gray.data[52 * 100 + 52] = 200;
        assert!(clean_holes(&gray, &mask, 0.01).unwrap_err().to_string().contains("0.01 fraction"));
        let (clean, _) = clean_holes(&gray, &mask, 0.05).unwrap();
        assert!(clean.at(22, 22) == 1 && clean.at(52, 52) == 0);
        for invalid in [f64::NAN, f64::INFINITY, -0.01, 0.051] {
            assert!(clean_holes(&gray, &mask, invalid).unwrap_err().to_string().contains("finite"));
        }
        // A diagonal gap does not connect a hole to the outside: the background is 4-connected.
        let mut mask = plane(20, 20, 1);
        paint(&mut mask, 0..1, 0..1, 0);
        paint(&mut mask, 1..2, 1..2, 0);
        assert_eq!(clean_holes(&plane(20, 20, 90), &mask, 0.05).unwrap().1["filled_pixels"], 1);
    }

    #[test]
    fn run_failure_publishes_no_masks_and_success_binds_sources() {
        let root = std::env::temp_dir().join(format!("crispmesh-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (images, masks) = (root.join("images"), root.join("masks"));
        std::fs::create_dir_all(&images).unwrap();
        std::fs::create_dir_all(&masks).unwrap();
        for i in 0..2 {
            image::RgbImage::from_pixel(100, 100, image::Rgb([90, 90, 90])).save(images.join(format!("{i}.png"))).unwrap();
            let mut mask = plane(100, 100, 1);
            paint(&mut mask, 20..25, 20..25, 0);
            if i == 1 {
                paint(&mut mask, 40..50, 40..50, 0);
            }
            save_mask(&masks.join(format!("{i}.png.png")), &mask).unwrap();
        }
        let failed = root.join("failed");
        let error = run(&images, &masks, &failed, 0.01, 2, &mut |_| Ok(())).unwrap_err().to_string();
        assert!(error.contains("0.01 fraction"), "{error}");
        assert!(!failed.join("masks").exists());
        assert_eq!(util::read_json(&failed.join("result.json")).unwrap()["status"], "failed");
        let accepted = run(&images, &masks, &root.join("explicit-budget"), 0.02, 2, &mut |_| Ok(())).unwrap();
        assert_eq!(accepted["configuration"]["maximum_total_filled_foreground_fraction"], 0.02);
        assert_eq!(accepted["rows"][1]["filled_pixels"], 125);
        assert_eq!(accepted["rows"][1]["name"], "1.png");
        assert_eq!(accepted["source_hashes_before"], accepted["source_hashes_after"]);
        assert_eq!(accepted["output_hashes"].as_object().unwrap().len(), 2);
        let clean = open_binary_mask(&root.join("explicit-budget/masks/1.png.png")).unwrap();
        assert_eq!(clean.data.iter().filter(|&&m| m == 0).count(), 0);
        assert!(run(&images, &masks, &root.join("explicit-budget"), 0.02, 2, &mut |_| Ok(())).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
