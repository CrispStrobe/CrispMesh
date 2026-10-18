//! The declared lens: `load_calibration`, `scale_calibration`,
//! `alicevision_intrinsic_fields` and `calibrated_scene` of `photos_to_inputs.py`.

use std::path::Path;

use anyhow::{anyhow, bail};
use serde_json::{json, Value};

use super::solution::Lens;
use super::util::{self, python_float};

pub const CALIBRATION_SCHEMA: &str = "crispmesh_lens_calibration_v1";

/// A radial k1, k2, k3 lens at the resolution it was calibrated for. The
/// principal point is in the convention where the centre of the top-left pixel is (0, 0).
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    pub width: u32,
    pub height: u32,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub k: [f64; 3],
    pub sensor_width_mm: f64,
}

/// The lens at the photo resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct Scaled {
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    pub k: [f64; 3],
    pub scale: [f64; 2],
}

impl Scaled {
    /// The scaled lens for photos of the given size.
    pub fn lens(&self, width: u32, height: u32) -> Lens {
        Lens { width, height, pixels: [self.fx, self.fy, self.cx, self.cy], k: self.k }
    }

    pub fn to_json(&self) -> Value {
        json!({"fx": self.fx, "fy": self.fy, "cx": self.cx, "cy": self.cy, "k": self.k, "scale": self.scale})
    }
}

/// Reads a lens file. Two spellings are accepted: ours (`schema:
/// crispmesh_lens_calibration_v1`) and the raw 3DLF `rgb_optic.json` (`model:
/// brown5` with a nested `distortion`, whose tangential terms must be zero).
pub fn load_calibration(path: &Path) -> anyhow::Result<Calibration> {
    parse_calibration(&util::read_json(path)?)
}

pub fn parse_calibration(data: &Value) -> anyhow::Result<Calibration> {
    let field = |object: &Value, name: &str| -> anyhow::Result<f64> {
        match object.get(name) {
            None => bail!("calibration is missing {name}"),
            Some(Value::Number(n)) => {
                n.as_f64().filter(|v| v.is_finite()).ok_or_else(|| anyhow!("calibration values must be finite numbers"))
            }
            Some(_) => bail!("calibration values must be finite numbers"),
        }
    };
    let (width, height, k) = if data.get("schema").and_then(Value::as_str) == Some(CALIBRATION_SCHEMA) {
        if data.get("model").and_then(Value::as_str) != Some("radialk3") {
            bail!("calibration model must be radialk3");
        }
        if data.get("principal_point_convention").is_some_and(|v| v.as_str() != Some("pixel_centre")) {
            bail!("principal_point_convention must be pixel_centre");
        }
        (
            field(data, "calibration_width")?,
            field(data, "calibration_height")?,
            [field(data, "k1")?, field(data, "k2")?, field(data, "k3")?],
        )
    } else if data.get("model").and_then(Value::as_str) == Some("brown5") && data.get("distortion").is_some_and(Value::is_object) {
        let distortion = &data["distortion"];
        for name in ["p1", "p2"] {
            if distortion.get(name).is_some_and(|v| v.as_f64() != Some(0.0)) {
                bail!("tangential distortion is not supported; p1 and p2 must be zero");
            }
        }
        (field(data, "width")?, field(data, "height")?, [field(distortion, "k1")?, field(distortion, "k2")?, field(distortion, "k3")?])
    } else {
        bail!("unknown calibration format; expected schema {CALIBRATION_SCHEMA} or a brown5 rgb_optic.json");
    };
    let (fx, fy, cx, cy) = (field(data, "fx")?, field(data, "fy")?, field(data, "cx")?, field(data, "cy")?);
    if width.fract() != 0.0 || height.fract() != 0.0 || width < 1.0 || height < 1.0 || width > 1e9 || height > 1e9 || fx <= 0.0 || fy <= 0.0
    {
        bail!("invalid calibration resolution or focal length");
    }
    let sensor = match data.get("sensor_width_mm") {
        None => 36.0,
        Some(value) => util::number(value).map_err(|_| anyhow!("invalid sensor_width_mm"))?,
    };
    if !sensor.is_finite() || sensor <= 0.0 {
        bail!("invalid sensor_width_mm");
    }
    Ok(Calibration { width: width as u32, height: height as u32, fx, fy, cx, cy, k, sensor_width_mm: sensor })
}

/// `--calibration fov:DEG`: a pinhole lens whose horizontal field of view is `degrees` across the photo's width
/// (as the photos are given), focal length the same in both directions, principal point in the centre, no
/// distortion. For a camera whose field of view is known (a phone or webcam reports it) but which has no lens file.
pub fn field_of_view_calibration(degrees: f64, width: u32, height: u32) -> anyhow::Result<Calibration> {
    if !(degrees.is_finite() && degrees > 1.0 && degrees < 179.0) {
        bail!("--calibration fov:{degrees}: the field of view must be between 1 and 179 degrees");
    }
    if width == 0 || height == 0 {
        bail!("invalid photo size {width}x{height}");
    }
    let (w, h) = (width as f64, height as f64);
    let focal = w / (2.0 * (degrees.to_radians() / 2.0).tan());
    Ok(Calibration { width, height, fx: focal, fy: focal, cx: (w - 1.0) / 2.0, cy: (h - 1.0) / 2.0, k: [0.0; 3], sensor_width_mm: 36.0 })
}

/// The value of `--calibration fov:DEG`, if `text` is that spelling.
pub fn parse_field_of_view(text: &str) -> Option<anyhow::Result<f64>> {
    let value = text.strip_prefix("fov:")?;
    Some(match value.trim().parse::<f64>() {
        Ok(degrees) if degrees.is_finite() && degrees > 1.0 && degrees < 179.0 => Ok(degrees),
        _ => Err(anyhow!("--calibration fov:{value}: expected the horizontal field of view in degrees, between 1 and 179")),
    })
}

/// Scales a lens declared at one resolution to the photo resolution.
///
/// A pure resize keeps the radial coefficients (they act on normalised
/// coordinates). Focal lengths scale with the image; the principal point is
/// scaled about the image corner, hence the half-pixel terms. The photos must
/// be a resize of the calibration frame, not a crop: the aspect ratios must agree.
pub fn scale_calibration(calibration: &Calibration, width: u32, height: u32) -> anyhow::Result<Scaled> {
    const ASPECT_TOLERANCE: f64 = 0.002;
    let (sx, sy) = (width as f64 / calibration.width as f64, height as f64 / calibration.height as f64);
    if (sx / sy - 1.0).abs() > ASPECT_TOLERANCE {
        bail!(
            "photo {width}x{height} is not a resize of the calibration frame {}x{} (a crop cannot be scaled)",
            calibration.width,
            calibration.height
        );
    }
    Ok(Scaled {
        fx: calibration.fx * sx,
        fy: calibration.fy * sy,
        cx: (calibration.cx + 0.5) * sx - 0.5,
        cy: (calibration.cy + 0.5) * sy - 0.5,
        k: calibration.k,
        scale: [sx, sy],
    })
}

/// The fields of an AliceVision pinhole/radialk3 intrinsic that pin the lens, all locked.
pub fn alicevision_intrinsic_fields(lens: &Lens, sensor_width: f64) -> Value {
    let [fx, fy, cx, cy] = lens.pixels;
    let (width, height) = (lens.width as f64, lens.height as f64);
    let focal = python_float(fy * sensor_width / width);
    json!({
        "focalLength": focal, "initialFocalLength": focal, "pixelRatio": python_float(fy / fx),
        "principalPoint": [python_float(cx - width / 2.0), python_float(cy - height / 2.0)],
        "initializationMode": "calibrated", "distortionInitializationMode": "calibrated",
        "distortionParams": lens.k.map(python_float), "locked": "true", "scaleLocked": "true",
        "offsetLocked": "true", "distortionLocked": "true", "pixelRatioLocked": "true",
    })
}

/// Width and height of the single intrinsic of a scene.
pub fn scene_size(scene: &Value) -> anyhow::Result<(u32, u32)> {
    let intrinsic = scene.get("intrinsics").and_then(|v| v.get(0)).ok_or_else(|| anyhow!("the scene has no intrinsic"))?;
    let side = |name: &str| -> anyhow::Result<u32> {
        let value = util::number(intrinsic.get(name).unwrap_or(&Value::Null))?;
        if value.fract() != 0.0 || !(1.0..=1e9).contains(&value) {
            bail!("invalid image {name} in the scene");
        }
        Ok(value as u32)
    };
    Ok((side("width")?, side("height")?))
}

/// Copy of an uncalibrated cameraInit scene with its single intrinsic replaced by the declared lens.
pub fn calibrated_scene(scene: &Value, lens: &Lens) -> anyhow::Result<Value> {
    let filled = |name: &str| scene.get(name).and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    let intrinsics = scene.get("intrinsics").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    if intrinsics != 1 || filled("poses") || filled("structure") {
        bail!("expected one shared intrinsic and no poses or landmarks before camera recovery");
    }
    let mut scene = scene.clone();
    if scene_size(&scene)? != (lens.width, lens.height) {
        bail!("the scene's photos are not the size the lens was scaled to");
    }
    let intrinsic = &mut scene["intrinsics"][0];
    if intrinsic.get("type").and_then(Value::as_str) != Some("pinhole")
        || intrinsic.get("distortionType").and_then(Value::as_str) != Some("radialk3")
    {
        bail!("cameraInit did not produce a pinhole radialk3 intrinsic");
    }
    let sensor = util::number(intrinsic.get("sensorWidth").unwrap_or(&Value::Null))?;
    let Value::Object(fields) = alicevision_intrinsic_fields(lens, sensor) else { unreachable!() };
    let target = intrinsic.as_object_mut().ok_or_else(|| anyhow!("the intrinsic is not an object"))?;
    target.extend(fields);
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/turntable_mesh/calibrations/3dlf-pro.json");
        util::read_json(Path::new(path)).unwrap()
    }

    #[test]
    fn example_file_scales_to_the_3dlf_photo_size() {
        let calibration = parse_calibration(&example()).unwrap();
        assert_eq!((calibration.width, calibration.height), (583, 385));
        let scaled = scale_calibration(&calibration, 1749, 1155).unwrap();
        // The values the accepted Dragon, Armadillo and Bunny cameras were recovered with.
        assert!((scaled.fx - 2328.2847290039062).abs() < 1e-9);
        assert!((scaled.fy - 2329.8724365234375).abs() < 1e-9);
        assert!((scaled.cx - 874.1592102050781).abs() < 1e-9);
        assert!((scaled.cy - 555.7409973144531).abs() < 1e-9);
        assert_eq!(scaled.k, calibration.k);
        assert_eq!(scaled.scale, [3.0, 3.0]);
    }

    #[test]
    fn same_resolution_is_identity_and_crops_are_refused() {
        let calibration = parse_calibration(&example()).unwrap();
        let same = scale_calibration(&calibration, 583, 385).unwrap();
        assert!((same.cx - calibration.cx).abs() < 1e-12 && (same.fy - calibration.fy).abs() < 1e-12);
        assert!(scale_calibration(&calibration, 1749, 1000).is_err());
    }

    #[test]
    fn raw_dataset_spelling_and_rejections() {
        let ours = example();
        let mut raw = json!({
            "model": "brown5", "width": 583, "height": 385, "fx": ours["fx"], "fy": ours["fy"], "cx": ours["cx"], "cy": ours["cy"],
            "distortion": {"k1": ours["k1"], "k2": ours["k2"], "p1": 0.0, "p2": 0.0, "k3": ours["k3"]},
        });
        assert_eq!(parse_calibration(&raw).unwrap(), parse_calibration(&ours).unwrap());
        raw["distortion"]["p1"] = json!(1e-4);
        assert!(parse_calibration(&raw).unwrap_err().to_string().contains("tangential"));
        let mut text = ours.clone();
        text["fx"] = json!("776");
        assert!(parse_calibration(&text).is_err());
        let mut negative = ours.clone();
        negative["fy"] = json!(-1.0);
        assert!(parse_calibration(&negative).is_err());
        let mut model = ours.clone();
        model["model"] = json!("fisheye");
        assert!(parse_calibration(&model).is_err());
        assert!(parse_calibration(&json!({"fx": 1})).unwrap_err().to_string().contains("unknown calibration format"));
    }

    #[test]
    fn alicevision_fields_round_trip_through_the_audit_convention() {
        let scaled = scale_calibration(&parse_calibration(&example()).unwrap(), 1749, 1155).unwrap();
        let scene = json!({"views": [], "intrinsics": [{"type": "pinhole", "distortionType": "radialk3", "width": "1749",
                                                        "height": "1155", "sensorWidth": "36", "focalLength": "40"}]});
        let calibrated = calibrated_scene(&scene, &scaled.lens(1749, 1155)).unwrap();
        let intrinsic = &calibrated["intrinsics"][0];
        assert_eq!(scene["intrinsics"][0]["focalLength"], "40"); // input untouched
        assert_eq!(intrinsic["locked"], "true");
        let parse = |value: &Value| value.as_str().unwrap().parse::<f64>().unwrap();
        let fy = parse(&intrinsic["focalLength"]) * 1749.0 / 36.0;
        assert!((fy - scaled.fy).abs() < 1e-9);
        assert!((fy / parse(&intrinsic["pixelRatio"]) - scaled.fx).abs() < 1e-9);
        assert!((parse(&intrinsic["principalPoint"][0]) + 1749.0 / 2.0 - scaled.cx).abs() < 1e-9);
        assert!((parse(&intrinsic["principalPoint"][1]) + 1155.0 / 2.0 - scaled.cy).abs() < 1e-9);
        let k: Vec<f64> = intrinsic["distortionParams"].as_array().unwrap().iter().map(parse).collect();
        assert_eq!(k, scaled.k);
        // The strings are what Python's str(float) writes for the 3DLF lens.
        assert_eq!(intrinsic["focalLength"], "47.9562079558855");
        assert_eq!(intrinsic["pixelRatio"], "1.0006819215449696");
        assert_eq!(intrinsic["principalPoint"], json!(["-0.340789794921875", "-21.759002685546875"]));
        assert_eq!(intrinsic["distortionParams"][2], "-3.7169739384093505");
        let mut posed = scene.clone();
        posed["poses"] = json!([{}]);
        assert!(calibrated_scene(&posed, &scaled.lens(1749, 1155)).is_err());
        assert!(calibrated_scene(&scene, &scaled.lens(1749, 1000)).is_err());
    }
}

#[cfg(test)]
mod field_of_view_tests {
    use super::*;

    #[test]
    fn field_of_view_lens() {
        // 90 degrees across 1000 px: focal 500 px; principal point at the centre of the pixel grid.
        let c = field_of_view_calibration(90.0, 1000, 750).unwrap();
        assert!((c.fx - 500.0).abs() < 1e-9 && c.fx == c.fy);
        assert_eq!((c.cx, c.cy, c.k), (499.5, 374.5, [0.0; 3]));
        // Back from the focal length: the 3DLF lens (fx 2328.28 at 1749 px wide) is about 41.2 degrees across.
        let fx: f64 = 776.0949096679688 * 3.0;
        let fov = 2.0 * (1749.0 / 2.0 / fx).atan().to_degrees();
        let c = field_of_view_calibration(fov, 1749, 1155).unwrap();
        assert!((c.fx - fx).abs() < 1e-6);
        // Scaling to the same size keeps it.
        let s = scale_calibration(&c, 1749, 1155).unwrap();
        assert_eq!((s.fx, s.cx, s.cy), (c.fx, c.cx, c.cy));
        assert!(field_of_view_calibration(0.5, 10, 10).is_err());
        assert!(field_of_view_calibration(180.0, 10, 10).is_err());
    }

    #[test]
    fn field_of_view_spelling() {
        assert!(parse_field_of_view("auto").is_none());
        assert!(parse_field_of_view("lens.json").is_none());
        assert_eq!(parse_field_of_view("fov:63.5").unwrap().unwrap(), 63.5);
        for bad in ["fov:", "fov:wide", "fov:0", "fov:180", "fov:NaN", "fov:-5"] {
            assert!(parse_field_of_view(bad).unwrap().is_err(), "{bad}");
        }
    }
}

/// The first lens of `--calibration auto` for photos of `width` x `height`: the focal length from the 35 mm
/// equivalent focal length when the photo carries one (by the diagonal, as the equivalent is defined), else from
/// `field_of_view` degrees across the longer side; principal point in the centre, no distortion. Also says which.
pub fn guessed_lens(width: u32, height: u32, focal_35mm: Option<f64>, field_of_view: f64) -> (Lens, &'static str) {
    let (w, h) = (width as f64, height as f64);
    let (focal, source) = match focal_35mm.filter(|f| (8.0..=1200.0).contains(f)) {
        Some(f) => (f * w.hypot(h) / 36f64.hypot(24.0), "exif_35mm_equivalent"),
        None => (w.max(h) / (2.0 * (field_of_view.to_radians() / 2.0).tan()), "field_of_view_guess"),
    };
    (Lens { width, height, pixels: [focal, focal, (w - 1.0) / 2.0, (h - 1.0) / 2.0], k: [0.0; 3] }, source)
}

/// The 35 mm equivalent focal length (EXIF tag 0xA405) of a JPEG or PNG's EXIF block, if it has one.
pub fn exif_focal_35mm(bytes: &[u8]) -> Option<f64> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let exif = reader.into_decoder().ok()?.exif_metadata().ok()??;
    focal_35mm_of_tiff(&exif)
}

/// Reads tag 0xA405 from the EXIF sub-directory of a TIFF-structured EXIF block.
fn focal_35mm_of_tiff(tiff: &[u8]) -> Option<f64> {
    let tiff = tiff.strip_prefix(b"Exif\0\0").unwrap_or(tiff);
    let little = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    // The value field of `tag` in the directory at `offset`: (type, count, position of the value field).
    let find = |offset: usize, tag: u16| -> Option<(u16, u32, usize)> {
        let entries = u16_at(offset)? as usize;
        (0..entries.min(512))
            .map(|i| offset + 2 + 12 * i)
            .find(|&e| u16_at(e) == Some(tag))
            .and_then(|e| Some((u16_at(e + 2)?, u32_at(e + 4)?, e + 8)))
    };
    let first = u32_at(4)? as usize;
    let (_, _, pointer) = find(first, 0x8769)?;
    let (kind, count, value) = find(u32_at(pointer)? as usize, 0xA405)?;
    // SHORT (3), sometimes LONG (4); one value, stored in the entry itself.
    let focal = match (kind, count) {
        (3, 1) => u16_at(value)? as f64,
        (4, 1) => u32_at(value)? as f64,
        _ => return None,
    };
    (focal > 0.0).then_some(focal)
}

#[cfg(test)]
mod lens_guess_tests {
    use super::*;

    /// A little-endian TIFF block with IFD0 -> EXIF IFD -> FocalLengthIn35mmFilm = 26 (an iPhone main camera).
    fn exif_block(little: bool, focal: u16) -> Vec<u8> {
        let mut out = Vec::new();
        let u16b = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        out.extend(if little { b"II" } else { b"MM" });
        out.extend(u16b(42));
        out.extend(u32b(8));
        // IFD0 at 8: one entry, the EXIF pointer to 26.
        out.extend(u16b(1));
        out.extend(u16b(0x8769));
        out.extend(u16b(4));
        out.extend(u32b(1));
        out.extend(u32b(26));
        out.extend(u32b(0));
        // EXIF IFD at 26: one SHORT entry.
        out.extend(u16b(1));
        out.extend(u16b(0xA405));
        out.extend(u16b(3));
        out.extend(u32b(1));
        out.extend(u16b(focal));
        out.extend([0, 0]);
        out.extend(u32b(0));
        out
    }

    #[test]
    fn reads_the_35mm_focal_length_in_both_byte_orders() {
        assert_eq!(focal_35mm_of_tiff(&exif_block(true, 26)), Some(26.0));
        assert_eq!(focal_35mm_of_tiff(&exif_block(false, 26)), Some(26.0));
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend(exif_block(true, 50));
        assert_eq!(focal_35mm_of_tiff(&prefixed), Some(50.0));
        assert_eq!(focal_35mm_of_tiff(b"II*\0\x08\0\0\0\0\0"), None);
        assert_eq!(focal_35mm_of_tiff(b"junk"), None);
    }

    #[test]
    fn guesses_a_centred_lens_without_distortion() {
        // 26 mm equivalent on a 4032 x 3024 photo: about 3 100 px (an iPhone's main camera).
        let (lens, source) = guessed_lens(4032, 3024, Some(26.0), 65.0);
        assert_eq!(source, "exif_35mm_equivalent");
        assert!((lens.pixels[0] - 26.0 * 5040.0 / 43.2666).abs() < 1.0, "{}", lens.pixels[0]);
        assert_eq!(lens.pixels[2..], [2015.5, 1511.5]);
        assert_eq!(lens.k, [0.0; 3]);
        // No EXIF: 90 degrees across the longer side is half that side.
        let (lens, source) = guessed_lens(1000, 800, None, 90.0);
        assert_eq!(source, "field_of_view_guess");
        assert!((lens.pixels[0] - 500.0).abs() < 1e-9);
    }
}
