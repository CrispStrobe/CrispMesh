//! Published masks with their statistics and contact sheet, and the sparse
//! overlay (`step_publish_masks` and `step_overlay` of `photos_to_inputs.py`).
//!
//! The numbers follow the reference exactly. The two sheets are pictures for
//! people: tiles are reduced with the crate's bicubic resampler where the
//! reference uses Pillow's Lanczos or OpenCV's bilinear filter, and labels use
//! the crate's built-in font.

use std::path::Path;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::inputs::{median_f64, round_half_even, Plane};
use crate::render::Rgb;
use crate::stereo::level::{resize, Filter};

use super::staging::{capture_name, open_photo, save_mask};
use super::util;

/// `np.linspace(0, count, min(limit, count), endpoint=False, dtype=int)`.
pub fn evenly_spaced(count: usize, limit: usize) -> Vec<usize> {
    let picks = limit.min(count);
    let step = count as f64 / picks.max(1) as f64;
    (0..picks).map(|n| (n as f64 * step) as usize).collect()
}

/// Antialiased reduction of an RGB image.
fn reduce(image: &Rgb, width: usize, height: usize) -> Rgb {
    let mut out = Rgb::filled(width, height, [0; 3]);
    for channel in 0..3 {
        let plane = Plane {
            width: image.width,
            height: image.height,
            data: image.data.iter().skip(channel).step_by(3).map(|&v| v as f32).collect(),
        };
        for (n, value) in resize(&plane, width, height, Filter::Bicubic).data.iter().enumerate() {
            out.data[n * 3 + channel] = (value + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// A mask file as 0/1, like `np.asarray(image.convert("L")) > 0` (masks) or `> 127` (inputs).
fn open_mask(path: &Path, threshold: u8) -> anyhow::Result<Plane<u8>> {
    let gray = crate::photos::fs::open_image(path).with_context(|| path.display().to_string())?.to_luma8();
    Ok(Plane {
        width: gray.width() as usize,
        height: gray.height() as usize,
        data: gray.into_raw().into_iter().map(|v| (v > threshold) as u8).collect(),
    })
}

/// Copies the cleaned masks to `masks/capture_NNNN.png` and writes
/// `masks-report.json` and `mask-contact-sheet.png`. Returns the report.
pub fn step_publish_masks(
    out: &Path,
    photo_count: usize,
    cleaned: &Path,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    let target = out.join("masks");
    crate::photos::fs::create_dir(&target).with_context(|| target.display().to_string())?;
    let names: Vec<String> = (0..photo_count).map(capture_name).collect();
    let (mut rows, mut areas, mut dropped, mut pixels) = (Vec::new(), Vec::new(), Vec::new(), 0usize);
    let mut filling = Vec::new();
    for (done, name) in names.iter().enumerate() {
        let source = cleaned.join(format!("{name}.png"));
        let mask = super::staging::open_binary_mask(&source).map_err(|_| anyhow!("mask is empty or not 0/255: {name}"))?;
        let area = mask.data.iter().filter(|&&m| m != 0).count();
        if area == 0 {
            bail!("mask is empty or not 0/255: {name}");
        }
        save_mask(&target.join(name), &mask)?;
        let coarse = open_mask(&out.join("work/coarse-masks").join(format!("{name}.png")), 0)?;
        if (coarse.width, coarse.height) != (mask.width, mask.height) {
            bail!("mask and coarse mask differ in size: {name}");
        }
        let added = mask.data.iter().zip(&coarse.data).filter(|(&m, &c)| m != 0 && c == 0).count();
        let lost = mask.data.iter().zip(&coarse.data).filter(|(&m, &c)| m == 0 && c != 0).count();
        rows.push(json!({"capture": name, "foreground_pixels": area, "added_to_coarse_pixels": added, "dropped_from_coarse_pixels": lost}));
        areas.push(area as f64);
        dropped.push(lost as f64 / area.max(1) as f64);
        pixels = mask.width * mask.height;
        if area as f64 / pixels as f64 > 0.9 {
            filling.push(name);
        }
        watch(done + 1)?;
    }
    let median = median_f64(&mut areas.clone());
    let over: Vec<&String> = names.iter().zip(&dropped).filter(|(_, &d)| d > 0.03).map(|(name, _)| name).collect();
    let report = json!({
        "count": rows.len(), "median_area_pixels": median, "median_area_fraction": median / pixels as f64,
        "minimum_area_pixels": areas.iter().cloned().fold(f64::INFINITY, f64::min) as u64,
        "maximum_area_pixels": areas.iter().cloned().fold(0.0, f64::max) as u64,
        "maximum_dropped_coarse_fraction": dropped.iter().cloned().fold(0.0, f64::max),
        "views_dropping_over_3_percent_of_coarse": over, "views": rows,
        "views_filling_over_90_percent_of_photo": filling,
    });
    util::write_json(&out.join("masks-report.json"), &report, 1)?;
    mask_sheet(out, &names)?;
    Ok(report)
}

/// Twelve photos with their mask tinted green, its outline in cyan and dark pixels outside it in red.
fn mask_sheet(out: &Path, names: &[String]) -> anyhow::Result<()> {
    let map = util::read_json(&out.join("photo-map.json"))?;
    let threshold = |name: &str| -> anyhow::Result<u32> {
        let rows = map["photos"].as_array().ok_or_else(|| anyhow!("photo-map.json has no photos"))?;
        let row = rows.iter().find(|row| row["capture"] == name).ok_or_else(|| anyhow!("photo-map.json does not list {name}"))?;
        Ok(row["threshold"].as_u64().unwrap_or(0) as u32)
    };
    let picks: Vec<&String> = evenly_spaced(names.len(), 12).into_iter().map(|n| &names[n]).collect();
    const WIDTH: usize = 460;
    const BAND: usize = 18;
    let mut sheet: Option<(Rgb, usize)> = None;
    for (n, name) in picks.iter().enumerate() {
        let photo = open_photo(&out.join("work/photos").join(name))?;
        let (width, height) = (photo.width(), photo.height());
        let gray = photo.gray();
        let mask = open_mask(&out.join("masks").join(name), 0)?;
        if (mask.width, mask.height) != (width, height) {
            bail!("mask and photo differ in size: {name}");
        }
        let level = threshold(name)?;
        let mut tile = Rgb { width, height, data: photo.rgb.into_raw() };
        let inside = |x: i64, y: i64| {
            x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height && mask.data[y as usize * width + x as usize] != 0
        };
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let mut colour = tile.pixel(x, y).map(|v| (v as f64 * 1.6 + 25.0).clamp(0.0, 255.0));
                let (xi, yi) = (x as i64, y as i64);
                if mask.data[index] != 0 {
                    colour = [colour[0] * 0.6, colour[1] * 0.6 + 255.0 * 0.4, colour[2] * 0.6];
                    // The mask minus its erosion with a cross: pixels with a neighbour outside (or beyond the frame).
                    if !(inside(xi - 1, yi) && inside(xi + 1, yi) && inside(xi, yi - 1) && inside(xi, yi + 1)) {
                        colour = [0.0, 255.0, 255.0];
                    }
                } else if (gray.data[index] as u32) < level {
                    colour = [255.0, 0.0, 0.0];
                }
                tile.set(x, y, colour.map(|v| v as u8));
            }
        }
        let tile_height = round_half_even(WIDTH as f64 * height as f64 / width as f64) as usize;
        let (sheet, tile_height) = sheet.get_or_insert_with(|| {
            let rows = picks.len().div_ceil(4);
            (Rgb::filled(4 * WIDTH, rows * (tile_height + BAND), [0x22; 3]), tile_height)
        });
        let (left, top) = (n % 4 * WIDTH, n / 4 * (*tile_height + BAND));
        sheet.paste(&reduce(&tile, WIDTH, *tile_height), left, top + BAND);
        sheet.text(name, left + 4, top + 13, 1, [255; 3]);
    }
    let (sheet, _) = sheet.ok_or_else(|| anyhow!("no photos for the mask sheet"))?;
    sheet.save(&out.join("mask-contact-sheet.png"))
}

/// Sparse points projected into the undistorted photos with their masks: do
/// cameras and masks agree? Writes `sparse-overlay.png` (four views) and
/// `sparse-overlay.json`, and returns the latter.
pub fn step_overlay(out: &Path, watch: &mut dyn FnMut(usize) -> anyhow::Result<()>) -> anyhow::Result<Value> {
    let inputs = out.join("inputs");
    let cameras = util::read_json(&inputs.join("cameras.json"))?;
    let views = cameras["views"].as_array().ok_or_else(|| anyhow!("cameras.json has no views"))?;
    let points = crate::npz::read_npy(&inputs.join("sparse_points.npy"))?;
    if points.shape.len() != 2 || points.shape[1] != 3 {
        bail!("sparse_points.npy must be N x 3");
    }
    let points = points.to_f64();
    let picks = evenly_spaced(views.len(), 4);
    let located = |value: &Value| -> anyhow::Result<std::path::PathBuf> {
        let path = Path::new(value.as_str().ok_or_else(|| anyhow!("cameras.json: a view lacks its image or mask"))?);
        Ok(if path.is_absolute() { path.to_path_buf() } else { inputs.join(path) })
    };
    let (mut tiles, mut inside) = (Vec::new(), Vec::new());
    for (n, view) in views.iter().enumerate() {
        let mask = open_mask(&located(&view["mask"])?, 127)?;
        let (width, height) = (mask.width, mask.height);
        let row: crate::inputs::ViewRow = serde_json::from_value(view.clone()).context("cameras.json")?;
        let [fx, fy, cx, cy] = row.k;
        let mut hits: Vec<(usize, usize)> = Vec::new();
        for p in points.as_chunks::<3>().0 {
            let camera =
                [0, 1, 2].map(|i| p[0] * row.rotation[i][0] + p[1] * row.rotation[i][1] + p[2] * row.rotation[i][2] + row.translation[i]);
            if camera[2] <= 1e-9 {
                continue;
            }
            let (x, y) = (fx * camera[0] / camera[2] + cx - 0.5, fy * camera[1] / camera[2] + cy - 0.5);
            if x >= 0.0 && x <= width as f64 - 1.0 && y >= 0.0 && y <= height as f64 - 1.0 {
                hits.push((round_half_even(x) as usize, round_half_even(y) as usize));
            }
        }
        // Inside the mask dilated by a 9 x 9 square: any mask pixel within four pixels.
        let near = |x: usize, y: usize| {
            (y.saturating_sub(4)..=(y + 4).min(height - 1))
                .any(|v| (x.saturating_sub(4)..=(x + 4).min(width - 1)).any(|u| mask.data[v * width + u] != 0))
        };
        inside.push(if hits.is_empty() { 0.0 } else { hits.iter().filter(|&&(x, y)| near(x, y)).count() as f64 / hits.len() as f64 });
        if picks.contains(&n) {
            let mut photo = Rgb::open(&located(&view["image"])?)?;
            if (photo.width, photo.height) != (width, height) {
                bail!("mask and image differ in size: {}", row.name);
            }
            for (index, pixel) in photo.data.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let mut colour = pixel.map(|v| (v as f64 * 1.2).clamp(0.0, 255.0));
                if mask.data[index] != 0 {
                    colour = [colour[0] * 0.6, colour[1] * 0.6 + 255.0 * 0.4, colour[2] * 0.6];
                }
                *pixel = colour.map(|v| v as u8);
            }
            for &(x, y) in &hits {
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        let (u, v) = (x as i64 + dx, y as i64 + dy);
                        if dx * dx + dy * dy <= 5 && u >= 0 && v >= 0 && (u as usize) < width && (v as usize) < height {
                            photo.set(u as usize, v as usize, [255, 0, 0]);
                        }
                    }
                }
            }
            photo.text(view["source"].as_str().unwrap_or_default(), 12, 34, 3, [0; 3]);
            tiles.push(reduce(&photo, 800, round_half_even(800.0 * height as f64 / width as f64) as usize));
        }
        watch(n + 1)?;
    }
    let Some(first) = tiles.first() else { bail!("no views for the sparse overlay") };
    let (tile_width, tile_height) = (first.width, first.height);
    let mut sheet = Rgb::filled(2 * tile_width, tiles.len().div_ceil(2) * tile_height, [0; 3]);
    for (n, tile) in tiles.iter().enumerate() {
        sheet.paste(tile, n % 2 * tile_width, n / 2 * tile_height);
    }
    sheet.save(&out.join("sparse-overlay.png"))?;
    let report = json!({
        "sparse_points_inside_mask_fraction": {"min": inside.iter().cloned().fold(f64::INFINITY, f64::min), "median": median_f64(&mut inside.clone())},
        "mask_dilation_pixels": 4,
    });
    util::write_json(&out.join("sparse-overlay.json"), &report, 1)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_follow_numpy_linspace() {
        // np.linspace(0, 73, 12, endpoint=False, dtype=int)
        assert_eq!(evenly_spaced(73, 12), [0, 6, 12, 18, 24, 30, 36, 42, 48, 54, 60, 66]);
        assert_eq!(evenly_spaced(73, 4), [0, 18, 36, 54]);
        assert_eq!(evenly_spaced(5, 12), [0, 1, 2, 3, 4]);
        assert_eq!(evenly_spaced(14, 12), [0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]);
        assert!(evenly_spaced(0, 4).is_empty());
    }

    #[test]
    fn publishes_masks_with_statistics_and_a_sheet() {
        let out = std::env::temp_dir().join(format!("crispmesh-sheets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        for folder in ["work/photos", "work/coarse-masks", "work/mask-cleanup/masks"] {
            std::fs::create_dir_all(out.join(folder)).unwrap();
        }
        let mut rows = Vec::new();
        for n in 0..3 {
            let name = capture_name(n);
            let mut photo = image::RgbImage::from_pixel(60, 40, image::Rgb([200, 200, 200]));
            let (mut coarse, mut clean) = (Plane::<u8>::new(60, 40), Plane::<u8>::new(60, 40));
            for y in 10..30 {
                for x in 20..40 {
                    photo.put_pixel(x, y, image::Rgb([30, 30, 30]));
                    coarse.data[y as usize * 60 + x as usize] = 1;
                    // The final mask loses a strip of n columns and gains one row.
                    clean.data[y as usize * 60 + x as usize] = (x >= 20 + 2 * n as u32) as u8;
                }
                clean.data[9 * 60 + y as usize + 10] = 1;
            }
            photo.save(out.join("work/photos").join(&name)).unwrap();
            save_mask(&out.join("work/coarse-masks").join(format!("{name}.png")), &coarse).unwrap();
            save_mask(&out.join("work/mask-cleanup/masks").join(format!("{name}.png")), &clean).unwrap();
            rows.push(json!({"capture": name, "threshold": 70}));
        }
        util::write_json(&out.join("photo-map.json"), &json!({"photos": rows}), 1).unwrap();
        let mut seen = 0;
        let report = step_publish_masks(&out, 3, &out.join("work/mask-cleanup/masks"), &mut |done| {
            seen = done;
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 3);
        assert_eq!(report["count"], 3);
        assert_eq!(
            report["views"][2],
            json!({"capture": "capture_0002.png", "foreground_pixels": 340, "added_to_coarse_pixels": 20, "dropped_from_coarse_pixels": 80})
        );
        assert_eq!(
            (report["median_area_pixels"].as_f64(), report["minimum_area_pixels"].as_u64(), report["maximum_area_pixels"].as_u64()),
            (Some(380.0), Some(340), Some(420))
        );
        assert!((report["median_area_fraction"].as_f64().unwrap() - 380.0 / 2400.0).abs() < 1e-15);
        assert!((report["maximum_dropped_coarse_fraction"].as_f64().unwrap() - 80.0 / 340.0).abs() < 1e-15);
        assert_eq!(report["views_dropping_over_3_percent_of_coarse"], json!(["capture_0001.png", "capture_0002.png"]));
        assert_eq!(util::read_json(&out.join("masks-report.json")).unwrap(), report);
        assert_eq!(
            super::super::staging::open_binary_mask(&out.join("masks/capture_0001.png")).unwrap().data.iter().filter(|&&m| m == 1).count(),
            380
        );
        let sheet = Rgb::open(&out.join("mask-contact-sheet.png")).unwrap();
        assert_eq!((sheet.width, sheet.height), (4 * 460, 307 + 18));
        // Middle of the first tile is mask (green tint over a dark object); the dropped strip of the third is red.
        let green = sheet.pixel(230, 18 + 153);
        assert!(green[1] > 100 && green[0] < 80, "{green:?}");
        let red = sheet.pixel(2 * 460 + 168, 18 + 153);
        assert!(red[0] > 200 && red[1] < 60, "{red:?}");
        assert!(step_publish_masks(&out, 3, &out.join("work/mask-cleanup/masks"), &mut |_| Ok(())).is_err());
        std::fs::remove_dir_all(&out).unwrap();
    }
}
