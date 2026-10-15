//! The neutral scene writer: every camera provider ends here. From a
//! [`Solution`], the photos and their masks (both in the photos' own,
//! distorted frame) it writes the directory the dense stages read:
//! `cameras.json`, undistorted `images/`, undistorted `masks/` and
//! `sparse_points.npy`.
//!
//! Undistortion is done here for photos and masks alike, with the map of
//! `crate::scene::Lens::undistort_map` (OpenCV's `initUndistortRectifyMap`):
//! masks by nearest sampling as `dense_all_views_inputs.py` does, photos by
//! bilinear sampling in floating point, black outside the photo. No camera
//! provider has to deliver undistorted images.

use crate::photos::fs::Stored as _;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::scene::{read_gray, remap_mask};

use super::solution::Solution;
use super::util;

/// Bilinear resampling of interleaved 8-bit RGB through a coordinate map; black outside the source.
pub fn remap_rgb(source: &[u8], width: usize, height: usize, map_x: &[f32], map_y: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; map_x.len() * 3];
    let pixel = |x: i64, y: i64, channel: usize| -> f32 {
        if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
            0.0
        } else {
            source[(y as usize * width + x as usize) * 3 + channel] as f32
        }
    };
    for (n, (&x, &y)) in map_x.iter().zip(map_y).enumerate() {
        if !(x > -1.0 && y > -1.0 && x < width as f32 && y < height as f32) {
            continue;
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        for channel in 0..3 {
            let top = pixel(x0, y0, channel) * (1.0 - tx) + pixel(x0 + 1, y0, channel) * tx;
            let bottom = pixel(x0, y0 + 1, channel) * (1.0 - tx) + pixel(x0 + 1, y0 + 1, channel) * tx;
            out[n * 3 + channel] = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
        }
    }
    out
}

/// The undistortion map of a solution's lens: source coordinates for every pixel of the pinhole image.
pub fn undistort_map(solution: &Solution) -> (Vec<f32>, Vec<f32>) {
    let lens = &solution.lens;
    let [fx, fy, cx, cy] = lens.pixels;
    crate::scene::Lens { width: lens.width as usize, height: lens.height as usize, fx, fy, cx, cy, k: lens.k }.undistort_map()
}

/// Writes the scene into the fresh directory `output`. `photos` and `masks`
/// hold one file per view under the view's source name. Returns `{"views",
/// "sparse_points"}`. `texture_photos`, when supplied, is original RGB in the
/// same source frame; it is undistorted separately and never used for matching.
/// `watch(done)` is called after every view.
pub fn write_scene(
    solution: &Solution,
    photos: &Path,
    texture_photos: Option<&BTreeMap<String, PathBuf>>,
    masks: &Path,
    output: &Path,
    threads: usize,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    if output.stored() {
        bail!("output directory exists: {}", output.display());
    }
    crate::photos::fs::create_dir_all(output.join("masks")).with_context(|| output.display().to_string())?;
    crate::photos::fs::create_dir_all(output.join("images"))?;
    if texture_photos.is_some() {
        crate::photos::fs::create_dir_all(output.join("texture_images"))?;
    }
    let (width, height) = (solution.lens.width as usize, solution.lens.height as usize);
    let [fx, fy, cx, cy] = solution.lens.pixels;
    let (map_x, map_y) = undistort_map(solution);
    let mut order: Vec<usize> = (0..solution.views.len()).collect();
    order.sort_by(|&a, &b| solution.views[a].source.cmp(&solution.views[b].source));
    let rows = util::parallel_with_input(
        order.len(),
        threads,
        |n| {
            // Host-backed browser files must be read on the calling thread.
            texture_photos
                .map(|sources| {
                    let source = &solution.views[order[n]].source;
                    let path = sources.get(source).with_context(|| format!("missing original RGB for {source}"))?;
                    Ok((path.clone(), crate::photos::fs::read(path)?))
                })
                .transpose()
        },
        |n, original_bytes| {
            let view = &solution.views[order[n]];
            let name = format!("view_{}", view.id);
            let photo_path = photos.join(&view.source);
            let photo = crate::photos::fs::open_image(&photo_path)
                .with_context(|| format!("photo of view {}: {}", view.id, photo_path.display()))?
                .to_rgb8();
            let mask_path = masks.join(&view.source);
            let mask = read_gray(&mask_path).with_context(|| format!("mask of view {}", view.id))?;
            if (photo.width() as usize, photo.height() as usize) != (width, height) || (mask.width, mask.height) != (width, height) {
                bail!("photo or mask of {} is not {width}x{height}, the size the cameras were recovered at", view.source);
            }
            util::save_rgb(
                &output.join(format!("images/{name}.png")),
                width,
                height,
                remap_rgb(photo.as_raw(), width, height, &map_x, &map_y),
            )?;
            util::save_gray(&output.join(format!("masks/{name}.png")), width, height, remap_mask(&mask, &map_x, &map_y))?;
            let mut row = json!({
                "name": name, "source": view.source, "image": format!("images/{name}.png"), "mask": format!("masks/{name}.png"),
                "width": width, "height": height, "k": [fx, fy, cx + 0.5, cy + 0.5],
                "rotation": view.rotation, "translation": view.translation,
            });
            if let Some((path, bytes)) = original_bytes {
                let original = super::staging::decode_photo(&path, &bytes)?.rgb;
                if (original.width() as usize, original.height() as usize) != (width, height) {
                    bail!("texture photo of {} is not {width}x{height}", view.source);
                }
                let relative = format!("texture_images/{name}.png");
                util::save_rgb(&output.join(&relative), width, height, remap_rgb(original.as_raw(), width, height, &map_x, &map_y))?;
                row["texture_image"] = json!(relative);
            }
            Ok(row)
        },
        watch,
    )?;
    let points: Vec<f64> = match &solution.object_points {
        Some(points) => points.iter().flatten().copied().collect(),
        None => solution.landmarks.iter().flat_map(|l| l.position).collect(),
    };
    crate::photos::fs::write(output.join("sparse_points.npy"), crate::npz::npy_f64(&[points.len() / 3, 3], &points))?;
    let mut cameras = json!({ "views": rows });
    if let Some((unit, source)) = &solution.scale {
        // One scene unit is one `unit`; absent for a scene of arbitrary scale.
        cameras["scale"] = json!({"unit": unit, "source": source});
    }
    crate::photos::fs::write(output.join("cameras.json"), serde_json::to_string_pretty(&cameras)? + "\n")?;
    Ok(json!({"views": rows.len(), "sparse_points": points.len() / 3}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::solution::{Landmark, Lens, View};

    #[test]
    fn bilinear_remap_interpolates_and_blanks_the_outside() {
        // 3 x 2 image, red channel ramps: 0 10 20 / 30 40 50.
        let source: Vec<u8> = [0u8, 10, 20, 30, 40, 50].iter().flat_map(|&v| [v, 255 - v, 7]).collect();
        let map_x = [0.0f32, 0.5, 1.25, 2.0, -3.0, 2.5];
        let map_y = [0.0f32, 0.0, 0.5, 1.0, 0.0, 0.0];
        let out = remap_rgb(&source, 3, 2, &map_x, &map_y);
        let red: Vec<u8> = out.as_chunks::<3>().0.iter().map(|p| p[0]).collect();
        // 0; (0+10)/2; (12.5 + 42.5)/2 = 27.5 -> 28; 50; outside -> 0; half over the right edge -> 10
        assert_eq!(red, [0, 5, 28, 50, 0, 10]);
        assert_eq!(out[3..6], [5, 250, 7]);
        assert_eq!(out[12..15], [0, 0, 0]);
    }

    #[test]
    fn writes_cameras_images_masks_and_points() {
        let root = std::env::temp_dir().join(format!("crispmesh-scene-writer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("photos")).unwrap();
        std::fs::create_dir_all(root.join("masks")).unwrap();
        let (width, height) = (64u32, 48u32);
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut views = Vec::new();
        for (n, name) in ["capture_0001.png", "capture_0000.png"].iter().enumerate() {
            let photo = image::RgbImage::from_fn(width, height, |x, y| image::Rgb([(x * 4) as u8, (y * 5) as u8, 9]));
            photo.save(root.join("photos").join(name)).unwrap();
            let mask = image::GrayImage::from_fn(width, height, |x, y| {
                image::Luma([if (20..40).contains(&x) && (10..30).contains(&y) { 255 } else { 0 }])
            });
            mask.save(root.join("masks").join(name)).unwrap();
            let centre = [n as f64, 2.0, -3.0];
            views.push(View {
                id: (70 + n).to_string(),
                source: name.to_string(),
                rotation: identity,
                centre,
                translation: [-(n as f64), -2.0, 3.0],
            });
        }
        // No distortion: the scene's images and masks are the inputs.
        let lens = Lens { width, height, pixels: [80.0, 80.0, 31.5, 23.5], k: [0.0; 3] };
        let landmarks = vec![Landmark { position: [0.5, 0.25, 4.0], observations: vec![] }];
        let solution = Solution { lens, views, unregistered: vec![], landmarks, lens_locked: None, scale: None, object_points: None };
        let mut seen = 0;
        let report = write_scene(&solution, &root.join("photos"), None, &root.join("masks"), &root.join("scene"), 2, &mut |done| {
            seen = done;
            Ok(())
        })
        .unwrap();
        assert_eq!((report["views"].as_u64(), report["sparse_points"].as_u64(), seen), (Some(2), Some(1), 2));
        let cameras = util::read_json(&root.join("scene/cameras.json")).unwrap();
        let first = &cameras["views"][0];
        // Sorted by source name; half-pixel-centre principal point.
        assert_eq!((first["name"].as_str(), first["source"].as_str()), (Some("view_71"), Some("capture_0000.png")));
        assert_eq!(first["k"], json!([80.0, 80.0, 32.0, 24.0]));
        assert_eq!(first["translation"], json!([-1.0, -2.0, 3.0]));
        assert_eq!((first["image"].as_str(), first["mask"].as_str()), (Some("images/view_71.png"), Some("masks/view_71.png")));
        let image = image::open(root.join("scene/images/view_71.png")).unwrap().to_rgb8();
        assert_eq!(image, image::open(root.join("photos/capture_0000.png")).unwrap().to_rgb8());
        let mask = image::open(root.join("scene/masks/view_70.png")).unwrap().to_luma8();
        assert_eq!(mask, image::open(root.join("masks/capture_0001.png")).unwrap().to_luma8());
        let points = crate::npz::read_npy(&root.join("scene/sparse_points.npy")).unwrap();
        assert_eq!((points.shape.clone(), points.to_f64()), (vec![1, 3], vec![0.5, 0.25, 4.0]));
        // The dense stage's loader accepts the directory (relative paths resolve against it).
        let config = crate::config::DenseConfig { neighbours: 1, ..Default::default() };
        let inputs = crate::inputs::Inputs::load(&root.join("scene"), &config).unwrap();
        assert_eq!(inputs.count(), 2);
        // Matching images stay identical; unenhanced colour has its own calibrated image.
        std::fs::create_dir_all(root.join("originals")).unwrap();
        for name in ["capture_0000.png", "capture_0001.png"] {
            image::RgbImage::from_pixel(width, height, image::Rgb([19, 67, 183])).save(root.join("originals").join(name)).unwrap();
        }
        let sources = ["capture_0000.png", "capture_0001.png"]
            .into_iter()
            .map(|name| (name.to_string(), root.join("originals").join(name)))
            .collect();
        write_scene(&solution, &root.join("photos"), Some(&sources), &root.join("masks"), &root.join("colour-scene"), 2, &mut |_| Ok(()))
            .unwrap();
        let colour_rows = crate::inputs::load_views(&root.join("colour-scene")).unwrap();
        assert_eq!(colour_rows[0].k, inputs.rows[0].k);
        assert_eq!(colour_rows[0].rotation, inputs.rows[0].rotation);
        assert_eq!(colour_rows[0].translation, inputs.rows[0].translation);
        assert_eq!(image::open(&colour_rows[0].image).unwrap().to_rgb8(), image);
        let original = image::open(colour_rows[0].texture_image.as_ref().unwrap()).unwrap().to_rgb8();
        assert!(original.pixels().all(|pixel| pixel.0 == [19, 67, 183]));
        assert!(inputs.rows[0].texture_image.is_none()); // Old scenes remain valid.
        let colour_inputs = crate::inputs::Inputs::load(&root.join("colour-scene"), &config).unwrap();
        assert!(colour_inputs.gray.iter().zip(&inputs.gray).all(|(a, b)| a.data == b.data));
        assert!(write_scene(&solution, &root.join("photos"), None, &root.join("masks"), &root.join("scene"), 2, &mut |_| Ok(())).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
