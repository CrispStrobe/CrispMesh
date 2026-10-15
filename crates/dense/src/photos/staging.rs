//! Photo staging: the capture order, `capture_NNNN.png` copies and the coarse
//! masks (`_photos` and `step_coarse` of `photos_to_inputs.py`).

use crate::photos::fs::Stored as _;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use image::ImageDecoder;
use serde_json::{json, Value};

use crate::inputs::Plane;

use super::coarse::{coarse_mask, resolve_envelope, Threshold};
use super::util;

pub const PHOTO_SUFFIXES: [&str; 5] = ["png", "jpg", "jpeg", "tif", "tiff"];

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Number(u128),
    Text(String),
}

/// Sort key that orders `x_2_rgb.png` before `x_10_rgb.png`: runs of digits compare as numbers, the rest without case.
fn natural_key(name: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut run = String::new();
    let mut digits = false;
    for character in name.chars() {
        if character.is_ascii_digit() != digits {
            parts.push(if digits { Part::Number(run.parse().unwrap_or(u128::MAX)) } else { Part::Text(run.to_lowercase()) });
            run.clear();
            digits = !digits;
        }
        run.push(character);
    }
    parts.push(if digits { Part::Number(run.parse().unwrap_or(u128::MAX)) } else { Part::Text(run.to_lowercase()) });
    parts
}

pub fn natural_order(a: &str, b: &str) -> Ordering {
    natural_key(a).cmp(&natural_key(b)).then_with(|| a.cmp(b))
}

pub fn capture_name(index: usize) -> String {
    format!("capture_{index:04}.png")
}

fn suffix(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// The photos of a folder in capture order (natural order of the file names).
pub fn list_photos(folder: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut photos = Vec::new();
    for path in crate::photos::fs::list(folder).with_context(|| folder.display().to_string())? {
        if PHOTO_SUFFIXES.contains(&suffix(&path).as_str()) && path.stored_file() {
            photos.push(path);
        }
    }
    photos.sort_by(|a, b| natural_order(&file_name(a), &file_name(b)));
    Ok(photos)
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// A decoded photo: upright 8-bit RGB as `ImageOps.exif_transpose(image).convert("RGB")`
/// gives it, and whether the file needed no rotation.
pub struct Photo {
    pub rgb: image::RgbImage,
    pub upright: bool,
}

impl Photo {
    pub fn width(&self) -> usize {
        self.rgb.width() as usize
    }

    pub fn height(&self) -> usize {
        self.rgb.height() as usize
    }

    /// Pillow's `convert("L")`.
    pub fn gray(&self) -> Plane<u8> {
        let data = self.rgb.pixels().map(|p| util::luma(p.0[0], p.0[1], p.0[2])).collect();
        Plane { width: self.width(), height: self.height(), data }
    }
}

/// Decodes a PNG or JPEG by content, as Pillow does. 16-bit files are reduced
/// by the decoder, which Pillow does differently; TIFF is not built in.
pub fn open_photo(path: &Path) -> anyhow::Result<Photo> {
    let context = || format!("{} (the native stage reads PNG and JPEG photos)", path.display());
    let bytes = crate::photos::fs::read(path).with_context(context)?;
    decode_photo(path, &bytes)
}

/// [`open_photo`] of bytes already read; `path` names them in errors.
pub fn decode_photo(path: &Path, bytes: &[u8]) -> anyhow::Result<Photo> {
    let context = || format!("{} (the native stage reads PNG and JPEG photos)", path.display());
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().with_context(context)?;
    let mut decoder = reader.into_decoder().with_context(context)?;
    let orientation = decoder.orientation().with_context(context)?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).with_context(context)?;
    decoded.apply_orientation(orientation);
    Ok(Photo { rgb: decoded.to_rgb8(), upright: orientation == image::metadata::Orientation::NoTransforms })
}

/// A 0/255 mask file as a 0/1 plane; anything else in the file is an error.
pub fn open_binary_mask(path: &Path) -> anyhow::Result<Plane<u8>> {
    let decoded = crate::photos::fs::open_image(path).with_context(|| path.display().to_string())?;
    let (width, height) = (decoded.width() as usize, decoded.height() as usize);
    let values = match decoded {
        image::DynamicImage::ImageLuma8(gray) => gray.into_raw(),
        _ => bail!("binary single-channel masks required: {}", path.display()),
    };
    if values.iter().any(|&v| v != 0 && v != 255) {
        bail!("binary single-channel masks required: {}", path.display());
    }
    Ok(Plane { width, height, data: values.into_iter().map(|v| (v != 0) as u8).collect() })
}

pub fn save_mask(path: &Path, mask: &Plane<u8>) -> anyhow::Result<()> {
    util::save_gray(path, mask.width, mask.height, mask.data.iter().map(|&m| if m != 0 { 255 } else { 0 }).collect())
}

/// Masks made elsewhere, brought to the names and form the cleanup reads.
pub fn import_masks(source: &Path, target: &Path, map: &Value, watch: &mut dyn FnMut(usize) -> anyhow::Result<()>) -> anyhow::Result<()> {
    crate::photos::fs::create_dir_all(target)?;
    let (width, height) = (map["width"].as_u64().unwrap_or(0) as u32, map["height"].as_u64().unwrap_or(0) as u32);
    for (index, row) in map["photos"].as_array().ok_or_else(|| anyhow!("photo-map.json has no photos"))?.iter().enumerate() {
        let (capture, original) = (capture_name(index), row["source"].as_str().unwrap_or_default());
        let stem = Path::new(original).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let candidates =
            [capture.clone(), format!("{capture}.png"), original.to_string(), format!("{original}.png"), format!("{stem}.png")];
        let found =
            candidates.iter().map(|name| source.join(name)).find(|path| path.extension().is_some_and(|e| e == "png") && path.stored_file());
        let Some(path) = found else {
            bail!("masks import:{}: no mask for photo {original} (looked for {})", source.display(), candidates.join(", "));
        };
        let gray = crate::photos::fs::open_image(&path).with_context(|| path.display().to_string())?.to_luma8();
        if (gray.width(), gray.height()) != (width, height) {
            bail!("{}: mask is {}x{}, the photos are {width}x{height}", path.display(), gray.width(), gray.height());
        }
        let data: Vec<u8> = gray.into_raw().into_iter().map(|v| (v > 127) as u8).collect();
        if !data.contains(&1) {
            bail!("{}: mask is empty", path.display());
        }
        save_mask(&target.join(format!("{capture}.png")), &Plane { width: width as usize, height: height as usize, data })?;
        watch(index + 1)?;
    }
    Ok(())
}

/// Stages the photos as `work/photos/capture_NNNN.png` (PNG files byte for
/// byte, others decoded once) and writes `work/coarse-masks/<name>.png` and
/// `photo-map.json`. Returns the photo map.
pub fn step_coarse(
    out: &Path,
    photos: &[PathBuf],
    envelope: &str,
    threshold: Threshold,
    threads: usize,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    step_coarse_with(out, photos, envelope, threshold, threads, None, watch)
}

/// What may be done to a photo's grey values before its dark region is taken (the photo itself is not
/// changed). It may return the grey level to use where the threshold was left to Otsu's method.
pub type Preparation = dyn Fn(&mut Plane<u8>) -> anyhow::Result<Option<u32>> + Send + Sync;

/// [`step_coarse`] with a preparation of the grey image, e.g. hiding the dark markers of a mat.
pub fn step_coarse_with(
    out: &Path,
    photos: &[PathBuf],
    envelope: &str,
    threshold: Threshold,
    threads: usize,
    prepare: Option<&Preparation>,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    step_coarse_shadow(out, photos, envelope, threshold, 0.0, threads, prepare, watch)
}

/// [`step_coarse_with`] that also takes the contact shadow out of every mask (`coarse::drop_shadow`
/// with this `shadow` fraction; 0 leaves the masks as they are).
#[allow(clippy::too_many_arguments)]
pub fn step_coarse_shadow(
    out: &Path,
    photos: &[PathBuf],
    envelope: &str,
    threshold: Threshold,
    shadow: f64,
    threads: usize,
    prepare: Option<&Preparation>,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    step_coarse_mode(out, photos, envelope, threshold, shadow, threads, prepare, false, watch)
}

pub fn step_coarse_background(
    out: &Path,
    photos: &[PathBuf],
    envelope: &str,
    threads: usize,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    step_coarse_mode(out, photos, envelope, Threshold::Level(70), 0.0, threads, None, true, watch)
}

#[allow(clippy::too_many_arguments)]
fn step_coarse_mode(
    out: &Path,
    photos: &[PathBuf],
    envelope: &str,
    threshold: Threshold,
    shadow: f64,
    threads: usize,
    prepare: Option<&Preparation>,
    background: bool,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    let (staged, masks) = (out.join("work/photos"), out.join("work/coarse-masks"));
    crate::photos::fs::create_dir_all(&staged)?;
    crate::photos::fs::create_dir_all(&masks)?;
    // The photo is read apart from the rest: in a browser with threads the host's photos can only
    // be read on the calling thread (`util::parallel_with_input`).
    let rows = util::parallel_with_input(
        photos.len(),
        threads,
        |index| {
            let source = &photos[index];
            crate::photos::fs::read(source).with_context(|| format!("{} (the native stage reads PNG and JPEG photos)", source.display()))
        },
        |index, bytes| {
            let source = &photos[index];
            let name = capture_name(index);
            let photo = decode_photo(source, &bytes)?;
            let byte_exact = suffix(source) == "png" && photo.upright;
            if byte_exact {
                crate::photos::fs::write(staged.join(&name), bytes).with_context(|| source.display().to_string())?;
            } else {
                crate::photos::fs::save_image(&photo.rgb, staged.join(&name))?;
            }
            let (width, height) = (photo.width(), photo.height());
            let window = resolve_envelope(envelope, width, height)?;
            let mut gray = photo.gray();
            let mut threshold = threshold;
            if let Some(prepare) = prepare {
                let level = prepare(&mut gray).map_err(|e| anyhow!("{}: {e}", file_name(source)))?;
                if let (Threshold::Otsu, Some(level)) = (threshold, level) {
                    threshold = Threshold::Level(level);
                }
            }
            let (mut mask, info) =
                (if background { super::background::mask(&photo.rgb, window) } else { coarse_mask(&gray, threshold, window) })
                    .map_err(|e| anyhow!("{}: {e}", file_name(source)))?;
            let shadow_pixels = super::coarse::drop_shadow(&gray, &mut mask, info.threshold, shadow);
            save_mask(&masks.join(format!("{name}.png")), &mask)?;
            let row = json!({
                "capture": name, "source": file_name(source), "byte_exact_copy": byte_exact,
                "threshold": info.threshold, "foreground_pixels": info.foreground_pixels - shadow_pixels,
                "shadow_pixels_removed": shadow_pixels,
                "other_dark_pixels_in_envelope": info.other_dark_pixels_in_envelope,
                "dark_pixels_outside_envelope": info.dark_pixels_outside_envelope,
                "bbox_xyxy": info.bbox_xyxy, "touches_envelope": info.touches_envelope,
            });
            Ok((row, width, height, window))
        },
        watch,
    )?;
    let Some(&(_, width, height, window)) = rows.first() else { bail!("no photos") };
    for ((_, w, h, _), source) in rows.iter().zip(photos) {
        if (*w, *h) != (width, height) {
            bail!("{} is ({w}, {h}); all photos must share one size (({width}, {height}))", file_name(source));
        }
    }
    let report = json!({
        "width": width, "height": height, "envelope_xyxy": window,
        "photos": rows.into_iter().map(|row| row.0).collect::<Vec<_>>(),
    });
    util::write_json(&out.join("photo-map.json"), &report, 1)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_photo_order() {
        let mut names = vec!["x_10_rgb.png", "x_2_rgb.png", "x_1_rgb.png", "X_11_rgb.png"];
        names.sort_by(|a, b| natural_order(a, b));
        assert_eq!(names, ["x_1_rgb.png", "x_2_rgb.png", "x_10_rgb.png", "X_11_rgb.png"]);
        assert_eq!(capture_name(7), "capture_0007.png");
        let mut names = vec!["9.png", "10.png", "a.png", "1b.png"];
        names.sort_by(|a, b| natural_order(a, b));
        assert_eq!(names, ["1b.png", "9.png", "10.png", "a.png"]);
    }

    #[test]
    fn stages_photos_and_writes_coarse_masks() {
        let folder = std::env::temp_dir().join(format!("crispmesh-staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("photos")).unwrap();
        for n in [2usize, 10, 1] {
            let mut photo = image::RgbImage::from_pixel(40, 30, image::Rgb([220, 220, 220]));
            for y in 10..20 {
                for x in 5 + n as u32..25 {
                    photo.put_pixel(x, y, image::Rgb([20, 30, 40]));
                }
            }
            photo.save(folder.join(format!("photos/thing_{n}_rgb.png"))).unwrap();
        }
        std::fs::write(folder.join("photos/notes.txt"), "not a photo").unwrap();
        let photos = list_photos(&folder.join("photos")).unwrap();
        assert_eq!(photos.iter().map(|p| file_name(p)).collect::<Vec<_>>(), ["thing_1_rgb.png", "thing_2_rgb.png", "thing_10_rgb.png"]);
        let out = folder.join("out");
        let map = step_coarse(&out, &photos, "auto", Threshold::Level(70), 2, &mut |_| Ok(())).unwrap();
        assert_eq!((map["width"].as_u64(), map["height"].as_u64()), (Some(40), Some(30)));
        assert_eq!(map["photos"][2]["source"], "thing_10_rgb.png");
        assert_eq!(map["photos"][2]["foreground_pixels"], 100);
        assert_eq!(map["photos"][0]["bbox_xyxy"], json!([6, 10, 25, 20]));
        assert_eq!(map["photos"][0]["byte_exact_copy"], true);
        assert_eq!(std::fs::read(out.join("work/photos/capture_0001.png")).unwrap(), std::fs::read(&photos[1]).unwrap());
        let mask = open_binary_mask(&out.join("work/coarse-masks/capture_0002.png.png")).unwrap();
        assert_eq!(mask.data.iter().filter(|&&m| m == 1).count(), 100);
        assert_eq!(util::read_json(&out.join("photo-map.json")).unwrap(), map);
        // A photo of another size is refused.
        image::RgbImage::from_pixel(41, 30, image::Rgb([0, 0, 0])).save(folder.join("photos/thing_11_rgb.png")).unwrap();
        let photos = list_photos(&folder.join("photos")).unwrap();
        let error = step_coarse(&folder.join("out2"), &photos, "auto", Threshold::Level(70), 2, &mut |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains("all photos must share one size"), "{error}");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
