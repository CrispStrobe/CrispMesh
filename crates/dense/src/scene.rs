//! Inputs directory from an AliceVision scene: port of
//! `scripts/turntable_mesh/dense_all_views_inputs.py`.
//!
//! Reads a `.sfm` JSON with one shared `radialk3` intrinsic, points at the
//! prepared (undistorted) images in place, and remaps each raw photo mask
//! through the lens undistortion with nearest sampling. The map follows
//! OpenCV's `initUndistortRectifyMap` (double precision, pixel coordinates
//! accumulated along each row, stored as float32) and `remap` with
//! `INTER_NEAREST` (float32 coordinates rounded half to even, zero outside).
//! No reference geometry, supplied poses or depth are read.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::inputs::{parallel_map, Plane};

/// A JSON number, or a string holding one (AliceVision writes strings).
fn number(value: &Value) -> anyhow::Result<f64> {
    match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| anyhow!("number out of range")),
        Value::String(s) => s.trim().parse::<f64>().map_err(|_| anyhow!("not a number: {s:?}")),
        other => bail!("expected a number, found {other}"),
    }
}

fn numbers(value: &Value, count: usize) -> anyhow::Result<Vec<f64>> {
    let list = value.as_array().ok_or_else(|| anyhow!("expected a list of {count} numbers"))?;
    if list.len() != count {
        bail!("expected {count} numbers, found {}", list.len());
    }
    list.iter().map(number).collect()
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// Lens of the scene: pinhole with three radial coefficients.
#[derive(Debug, Clone, Copy)]
pub struct Lens {
    pub width: usize,
    pub height: usize,
    pub fx: f64,
    pub fy: f64,
    /// Principal point in OpenCV's integer-pixel convention.
    pub cx: f64,
    pub cy: f64,
    pub k: [f64; 3],
}

impl Lens {
    /// Source coordinates in the distorted photo for every pixel of the
    /// undistorted image: `cv2.initUndistortRectifyMap(K, (k1, k2, 0, 0, k3), None, K, size, CV_32FC1)`.
    pub fn undistort_map(&self) -> (Vec<f32>, Vec<f32>) {
        let (fx, fy, cx, cy) = (self.fx, self.fy, self.cx, self.cy);
        let [k1, k2, k3] = self.k;
        // OpenCV inverts the 3x3 camera matrix by cofactors.
        let d = 1.0 / (fx * fy);
        let (ir0, ir2, ir4, ir5, ir8) = (fy * d, -(cx * fy) * d, fx * d, -(fx * cy) * d, fx * fy * d);
        let mut map_x = vec![0f32; self.width * self.height];
        let mut map_y = vec![0f32; self.width * self.height];
        for i in 0..self.height {
            let (mut sx, sy, sw) = (i as f64 * 0.0 + ir2, i as f64 * ir4 + ir5, i as f64 * 0.0 + ir8);
            for j in 0..self.width {
                let w = 1.0 / sw;
                let (x, y) = (sx * w, sy * w);
                let (x2, y2) = (x * x, y * y);
                let r2 = x2 + y2;
                let kr = 1.0 + ((k3 * r2 + k2) * r2 + k1) * r2;
                map_x[i * self.width + j] = (fx * (x * kr) + cx) as f32;
                map_y[i * self.width + j] = (fy * (y * kr) + cy) as f32;
                sx += ir0;
            }
        }
        (map_x, map_y)
    }
}

/// `cv2.remap(source, map_x, map_y, INTER_NEAREST, borderValue=0)` followed by the 127 threshold, as 0/255.
pub fn remap_mask(source: &Plane<u8>, map_x: &[f32], map_y: &[f32]) -> Vec<u8> {
    map_x
        .iter()
        .zip(map_y)
        .map(|(&x, &y)| {
            let (sx, sy) = (x.round_ties_even(), y.round_ties_even());
            let inside = sx >= 0.0 && sx < source.width as f32 && sy >= 0.0 && sy < source.height as f32;
            if inside && source.data[sy as usize * source.width + sx as usize] > 127 {
                255
            } else {
                0
            }
        })
        .collect()
}

/// `cv2.imread(path, IMREAD_GRAYSCALE)` for 8-bit files: grey as stored, colour through OpenCV's BGR weights.
pub fn read_gray(path: &Path) -> anyhow::Result<Plane<u8>> {
    let decoded = crate::storage::open_image(path).with_context(|| path.display().to_string())?;
    let (width, height) = (decoded.width() as usize, decoded.height() as usize);
    let luma = |r: u8, g: u8, b: u8| ((r as u32 * 4899 + g as u32 * 9617 + b as u32 * 1868 + 8192) >> 14) as u8;
    let data = match decoded {
        image::DynamicImage::ImageLuma8(l) => l.into_raw(),
        image::DynamicImage::ImageLumaA8(la) => la.pixels().map(|p| p.0[0]).collect(),
        image::DynamicImage::ImageRgb8(rgb) => rgb.pixels().map(|p| luma(p.0[0], p.0[1], p.0[2])).collect(),
        image::DynamicImage::ImageRgba8(rgba) => rgba.pixels().map(|p| luma(p.0[0], p.0[1], p.0[2])).collect(),
        other => other.to_luma8().into_raw(),
    };
    Ok(Plane { width, height, data })
}

/// Writes `cameras.json`, `masks/` and `sparse_points.npy` into a fresh directory.
/// Returns `{"views", "sparse_points"}` like the Python module.
pub fn run(scene: &Path, prepared: &Path, raw_masks: &Path, output: &Path) -> anyhow::Result<Value> {
    let data: Value = serde_json::from_str(&crate::storage::read_to_string(scene).with_context(|| scene.display().to_string())?)
        .with_context(|| scene.display().to_string())?;
    let intrinsics = data["intrinsics"].as_array().ok_or_else(|| anyhow!("scene has no intrinsics"))?;
    if intrinsics.len() != 1 || intrinsics[0]["distortionType"] != "radialk3" {
        bail!("expected one shared radialk3 intrinsic");
    }
    let intrinsic = &intrinsics[0];
    let (width, height) = (number(&intrinsic["width"])? as usize, number(&intrinsic["height"])? as usize);
    let fy = number(&intrinsic["focalLength"])? * width as f64 / number(&intrinsic["sensorWidth"])?;
    let fx = fy / number(&intrinsic["pixelRatio"])?;
    let principal = numbers(&intrinsic["principalPoint"], 2)?;
    // Integer-pixel principal point for the undistortion; half-centre K for the cameras.
    let (cx, cy) = (width as f64 / 2.0 + principal[0], height as f64 / 2.0 + principal[1]);
    let k = numbers(&intrinsic["distortionParams"], 3)?;
    let lens = Lens { width, height, fx, fy, cx, cy, k: [k[0], k[1], k[2]] };
    let (map_x, map_y) = lens.undistort_map();

    let mut poses = std::collections::BTreeMap::new();
    for pose in data["poses"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        poses.insert(text(&pose["poseId"]), &pose["pose"]["transform"]);
    }
    let mut views: Vec<&Value> = data["views"].as_array().ok_or_else(|| anyhow!("scene has no views"))?.iter().collect();
    views.sort_by_key(|v| file_name(&text(&v["path"])));
    let views: Vec<&Value> = views.into_iter().filter(|v| poses.contains_key(&text(&v["poseId"]))).collect();

    if crate::storage::exists(output) {
        bail!("output directory exists: {}", output.display());
    }
    crate::storage::create_dir_all(output.join("masks")).with_context(|| output.display().to_string())?;
    let rows: Vec<anyhow::Result<Value>> = parallel_map(views.len(), |n| {
        let view = views[n];
        let id = text(&view["viewId"]);
        let pose = poses[&text(&view["poseId"])];
        // Column-major rotation, as `reshape(3, 3, order="F")` reads it.
        let flat = numbers(&pose["rotation"], 9)?;
        let rotation = [[flat[0], flat[3], flat[6]], [flat[1], flat[4], flat[7]], [flat[2], flat[5], flat[8]]];
        let centre = numbers(&pose["center"], 3)?;
        let translation: Vec<f64> =
            rotation.iter().map(|r| (-r[2]).mul_add(centre[2], (-r[1]).mul_add(centre[1], -r[0] * centre[0]))).collect();
        let image = prepared.join(format!("{id}.png"));
        let source = file_name(&text(&view["path"]));
        let raw = read_gray(&raw_masks.join(&source)).ok().filter(|m| (m.width, m.height) == (width, height));
        let (true, Some(raw)) = (crate::storage::is_file(&image), raw) else { bail!("missing prepared image or raw mask for {id}") };
        let name = format!("view_{id}");
        let mask = remap_mask(&raw, &map_x, &map_y);
        crate::storage::save_png(output.join(format!("masks/{name}.png")), width, height, 1, &mask)?;
        let absolute = crate::storage::canonicalize(&image).unwrap_or(image);
        Ok(json!({
            "name": name, "source": source, "image": absolute.to_string_lossy(), "mask": format!("masks/{name}.png"),
            "width": width, "height": height, "k": [fx, fy, cx + 0.5, cy + 0.5],
            "rotation": rotation, "translation": translation,
        }))
    });
    let rows: Vec<Value> = rows.into_iter().collect::<anyhow::Result<_>>()?;
    let mut points = Vec::new();
    for landmark in data["structure"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        points.extend(numbers(&landmark["X"], 3)?);
    }
    crate::storage::write(output.join("sparse_points.npy"), crate::npz::npy_f64(&[points.len() / 3, 3], &points))?;
    crate::storage::write(output.join("cameras.json"), serde_json::to_string_pretty(&json!({ "views": rows }))? + "\n")?;
    Ok(json!({"views": rows.len(), "sparse_points": points.len() / 3}))
}

/// `crispmesh-dense inputs --scene F --prepared DIR --raw-masks DIR --output DIR`.
/// Makes an inputs directory self-contained: every photo and mask lies inside
/// it (`images/`, `masks/`) and `cameras.json` names them by relative path, so
/// the folder can be moved, archived or picked in a browser.
///
/// `output` is the directory to write (`None`: `source` itself is completed in
/// place). Files that are not already there are copied, or hard-linked with
/// `link` (falling back to a copy across file systems). Everything else in
/// `cameras.json` is kept as it is.
pub fn pack(source: &Path, output: Option<&Path>, link: bool) -> anyhow::Result<Value> {
    use crate::storage;
    let target = output.unwrap_or(source);
    let text = storage::read_to_string(source.join("cameras.json")).with_context(|| source.join("cameras.json").display().to_string())?;
    let mut cameras: Value = serde_json::from_str(&text).context("cameras.json")?;
    let same = |a: &Path, b: &Path| storage::canonicalize(a).ok().zip(storage::canonicalize(b).ok()).is_some_and(|(a, b)| a == b);
    let in_place = same(source, target);
    for folder in ["images", "masks"] {
        storage::create_dir_all(target.join(folder))?;
    }
    let (mut copied, mut linked, mut bytes) = (0usize, 0usize, 0u64);
    let views = cameras["views"].as_array_mut().ok_or_else(|| anyhow!("cameras.json has no views"))?;
    for view in views.iter_mut() {
        let name = view["name"].as_str().ok_or_else(|| anyhow!("a view has no name"))?.to_string();
        for (key, folder) in [("image", "images"), ("mask", "masks")] {
            let given = view[key].as_str().ok_or_else(|| anyhow!("view {name} has no {key}"))?;
            let from = if Path::new(given).is_absolute() { PathBuf::from(given) } else { source.join(given) };
            let extension = from.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "png".to_string());
            let relative = format!("{folder}/{name}.{extension}");
            let to = target.join(&relative);
            if !same(&from, &to) {
                if !storage::is_file(&from) {
                    bail!("{key} of view {name} is missing: {}", from.display());
                }
                #[cfg(not(target_arch = "wasm32"))]
                let done = link && {
                    let _ = std::fs::remove_file(&to);
                    std::fs::hard_link(&from, &to).is_ok()
                };
                #[cfg(target_arch = "wasm32")]
                let done = {
                    let _ = link;
                    false
                };
                if done {
                    linked += 1;
                } else {
                    let content = storage::read(&from).with_context(|| from.display().to_string())?;
                    bytes += content.len() as u64;
                    storage::write_owned(&to, content)?;
                    copied += 1;
                }
            }
            view[key] = json!(relative);
        }
    }
    let count = views.len();
    if !in_place {
        let points =
            storage::read(source.join("sparse_points.npy")).with_context(|| source.join("sparse_points.npy").display().to_string())?;
        storage::write_owned(target.join("sparse_points.npy"), points)?;
    }
    storage::write(target.join("cameras.json"), serde_json::to_string_pretty(&cameras)? + "\n")?;
    Ok(json!({"views": count, "copied": copied, "linked": linked, "copied_bytes": bytes, "output": target.to_string_lossy()}))
}

const USAGE: &str = "usage: crispmesh-dense inputs --scene FILE --prepared DIR --raw-masks DIR --output DIR [--self-contained [--link]]
       crispmesh-dense inputs --pack DIR [--output DIR] [--link]
--self-contained   copy the prepared photos into the inputs directory and name them by relative path (default: the
                   photos stay where they are and cameras.json holds their absolute paths)
--pack DIR         make an existing inputs directory self-contained, in place or as a copy in --output
--link             hard links instead of copies where the file system allows";

pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    let mut values = std::collections::BTreeMap::new();
    let (mut contained, mut link) = (false, false);
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--self-contained" => contained = true,
            "--link" => link = true,
            "--scene" | "--prepared" | "--raw-masks" | "--output" | "--pack" => {
                values.insert(flag.clone(), rest.next().ok_or_else(|| anyhow!("{flag} needs a value"))?.clone());
            }
            _ => bail!("unknown argument {flag}\n{USAGE}"),
        }
    }
    let get = |name: &str| values.get(name).map(Path::new).ok_or_else(|| anyhow!("{name} is required\n{USAGE}"));
    if let Some(source) = values.get("--pack") {
        println!("{}", pack(Path::new(source), values.get("--output").map(Path::new), link)?);
        return Ok(());
    }
    let mut report = run(get("--scene")?, get("--prepared")?, get("--raw-masks")?, get("--output")?)?;
    if contained {
        report["self_contained"] = pack(get("--output")?, None, link)?;
    }
    println!("{report}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_copies_the_photos_in_and_makes_the_paths_relative() {
        let folder = std::env::temp_dir().join(format!("crispmesh-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("inputs/masks")).unwrap();
        std::fs::create_dir_all(folder.join("prepared")).unwrap();
        std::fs::write(folder.join("prepared/7.png"), b"photo").unwrap();
        std::fs::write(folder.join("inputs/masks/view_7.png"), b"mask").unwrap();
        std::fs::write(folder.join("inputs/sparse_points.npy"), b"points").unwrap();
        let cameras = json!({"scale": {"unit": "mm"}, "views": [{
            "name": "view_7", "source": "a.png", "image": folder.join("prepared/7.png"), "mask": "masks/view_7.png", "width": 4, "height": 3,
        }]});
        std::fs::write(folder.join("inputs/cameras.json"), cameras.to_string()).unwrap();

        // As a copy elsewhere: both files and the points arrive, the rest of cameras.json is kept.
        let report = pack(&folder.join("inputs"), Some(&folder.join("packed")), false).unwrap();
        assert_eq!((report["views"].as_u64(), report["copied"].as_u64(), report["copied_bytes"].as_u64()), (Some(1), Some(2), Some(9)));
        let packed: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("packed/cameras.json")).unwrap()).unwrap();
        assert_eq!(
            (packed["views"][0]["image"].as_str(), packed["views"][0]["mask"].as_str()),
            (Some("images/view_7.png"), Some("masks/view_7.png"))
        );
        assert_eq!((packed["scale"]["unit"].as_str(), packed["views"][0]["width"].as_u64()), (Some("mm"), Some(4)));
        assert_eq!(std::fs::read(folder.join("packed/images/view_7.png")).unwrap(), b"photo");
        assert_eq!(std::fs::read(folder.join("packed/sparse_points.npy")).unwrap(), b"points");

        // In place, with links: only the photo is brought in; a second pass has nothing to do.
        let report = pack(&folder.join("inputs"), None, true).unwrap();
        assert_eq!(report["copied"].as_u64().unwrap() + report["linked"].as_u64().unwrap(), 1);
        assert_eq!(std::fs::read(folder.join("inputs/images/view_7.png")).unwrap(), b"photo");
        let again = pack(&folder.join("inputs"), None, false).unwrap();
        assert_eq!((again["copied"].as_u64(), again["linked"].as_u64()), (Some(0), Some(0)));
        std::fs::remove_file(folder.join("inputs/images/view_7.png")).unwrap();
        assert!(pack(&folder.join("inputs"), None, false).unwrap_err().to_string().contains("missing"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_undistorted_lens_maps_pixels_to_themselves() {
        let lens = Lens { width: 7, height: 5, fx: 100.0, fy: 110.0, cx: 3.2, cy: 2.1, k: [0.0; 3] };
        let (x, y) = lens.undistort_map();
        for i in 0..5 {
            for j in 0..7 {
                assert!((x[i * 7 + j] - j as f32).abs() < 1e-4 && (y[i * 7 + j] - i as f32).abs() < 1e-4);
            }
        }
        // Barrel distortion (k1 < 0) pulls source coordinates towards the principal point.
        let barrel = Lens { k: [-0.2, 0.0, 0.0], fx: 5.0, fy: 5.0, ..lens };
        let (x, _) = barrel.undistort_map();
        assert!(x[0] > 0.0 && x[6] < 6.0 && (x[2 * 7 + 3] - 3.0).abs() < 0.01);
    }

    #[test]
    fn remap_takes_the_nearest_pixel_and_zero_outside() {
        let source = Plane { width: 3, height: 2, data: vec![255u8, 0, 200, 0, 128, 127] };
        let out = remap_mask(&source, &[0.4, 0.5, 1.5, 2.5, -0.6, 1.0], &[0.0, 0.0, 0.0, 0.0, 0.0, 1.2]);
        // 0.5 rounds to 0, 1.5 and 2.5 round to 2; 2.5 -> 2 is inside, -0.6 -> -1 is outside.
        assert_eq!(out, vec![255, 255, 255, 255, 0, 255]);
        assert_eq!(remap_mask(&source, &[2.0, 3.0], &[1.0, 0.0]), vec![0, 0]);
    }

    #[test]
    fn numbers_come_as_strings_or_numbers() {
        assert_eq!(number(&json!("47.95620795")).unwrap(), 47.95620795);
        assert_eq!(number(&json!(3)).unwrap(), 3.0);
        assert!(number(&json!([1])).is_err());
        assert_eq!(numbers(&json!(["1", 2.5]), 2).unwrap(), vec![1.0, 2.5]);
        assert_eq!(file_name("/a/b/capture_0003.png"), "capture_0003.png");
    }
}
