//! A camera solution in neutral form, and its readers: an AliceVision `.sfm`
//! scene and a COLMAP model (binary or text). Every camera provider ends with
//! one of these; the gates and the scene writer work on it and know nothing
//! about the program that produced it.
//!
//! Conventions of [`Solution`]: world-to-camera rotation (rows) and camera
//! centre; pixel coordinates with the centre of the top-left pixel at (0, 0)
//! (OpenCV's and AliceVision's convention; COLMAP puts it at (0.5, 0.5), which
//! its reader removes); radial distortion `x_d = x (1 + k1 r^2 + k2 r^4 + k3
//! r^6)` on normalised coordinates.

use crate::photos::fs::Stored as _;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{anyhow, bail, Context};
use serde_json::Value;

use super::staging::file_name;
use super::util;

pub type Vector = [f64; 3];

/// Pinhole lens with three radial coefficients at the photo resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    pub width: u32,
    pub height: u32,
    /// `[fx, fy, cx, cy]`, centre of the top-left pixel at (0, 0).
    pub pixels: [f64; 4],
    pub k: [f64; 3],
}

/// A registered photo.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The producing program's identifier (AliceVision view id, COLMAP image id).
    pub id: String,
    /// File name of the photo.
    pub source: String,
    /// World-to-camera rotation, rows.
    pub rotation: [Vector; 3],
    pub centre: Vector,
    /// `-rotation * centre`.
    pub translation: Vector,
}

/// A sparse point with the pixels it was seen at: `(index into views, [x, y])`.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub position: Vector,
    pub observations: Vec<(usize, [f64; 2])>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    pub lens: Lens,
    pub views: Vec<View>,
    /// Photos the solution lists without a pose.
    pub unregistered: Vec<String>,
    pub landmarks: Vec<Landmark>,
    /// Whether the producing program reports the lens as held fixed (`None`: it does not say).
    pub lens_locked: Option<bool>,
    /// Physical scale of the frame as `(unit, source)`, e.g. `("mm", "markers")`; `None` for an arbitrary scale.
    pub scale: Option<(String, String)>,
    /// Points that locate the object, for the scene's `sparse_points.npy`, where the landmarks do not
    /// (a marker mat's landmarks lie around the object, not on it).
    pub object_points: Option<Vec<Vector>>,
}

/// `-R c` summed in the order NumPy's matrix product gives for `(-rotation) @ centre`.
pub fn translation(rotation: &[Vector; 3], centre: Vector) -> Vector {
    rotation.map(|r| (-r[2]).mul_add(centre[2], (-r[1]).mul_add(centre[1], -r[0] * centre[0])))
}

fn finite(value: &Value) -> anyhow::Result<f64> {
    let number = util::number(value).map_err(|_| anyhow!("invalid native numeric array"))?;
    if !number.is_finite() {
        bail!("nonfinite native camera parameter");
    }
    Ok(number)
}

fn array<const N: usize>(value: &Value) -> anyhow::Result<[f64; N]> {
    let items = value.as_array().filter(|a| a.len() == N).ok_or_else(|| anyhow!("invalid native numeric array"))?;
    let mut out = [0.0; N];
    for (target, item) in out.iter_mut().zip(items) {
        *target = finite(item)?;
    }
    Ok(out)
}

/// Converts AliceVision's intrinsic serialization (1.2.11 and later) to a [`Lens`].
pub fn sfm_lens(row: &Value) -> anyhow::Result<Lens> {
    let kind = row.get("distortionType").and_then(Value::as_str);
    if row.get("type").and_then(Value::as_str) != Some("pinhole") || !matches!(kind, Some("none" | "radialk3")) {
        bail!("unsupported camera model");
    }
    let field = |name: &str| -> anyhow::Result<f64> {
        match row.get(name) {
            Some(Value::Bool(_)) => bail!("boolean camera parameter"),
            Some(value) => util::number(value).map_err(|_| anyhow!("invalid camera parameter {name}")),
            None => bail!("missing camera parameter {name}"),
        }
    };
    let (width, height) = (field("width")?, field("height")?);
    if width.fract() != 0.0
        || height.fract() != 0.0
        || !(1.0..=16384.0).contains(&width)
        || !(1.0..=16384.0).contains(&height)
        || width * height > 64e6
    {
        bail!("invalid image dimensions");
    }
    let (sensor, focal, ratio) = (field("sensorWidth")?, field("focalLength")?, field("pixelRatio")?);
    if ![sensor, focal, ratio].iter().all(|v| v.is_finite() && *v > 0.0) {
        bail!("invalid focal length or aspect ratio");
    }
    let fy = focal * width / sensor;
    let fx = fy / ratio;
    let offset: [f64; 2] = array(row.get("principalPoint").unwrap_or(&Value::Null))?;
    let k = if kind == Some("radialk3") { array(row.get("distortionParams").unwrap_or(&Value::Null))? } else { [0.0; 3] };
    let pixels = [fx, fy, width / 2.0 + offset[0], height / 2.0 + offset[1]];
    if !pixels.iter().all(|v| v.is_finite()) {
        bail!("nonfinite pixel intrinsic");
    }
    Ok(Lens { width: width as u32, height: height as u32, pixels, k })
}

fn list<'a>(scene: &'a Value, name: &str) -> &'a [Value] {
    scene.get(name).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn proper(rotation: &[Vector; 3]) -> bool {
    let r = rotation;
    let orthonormal =
        (0..3).all(|i| (0..3).all(|j| ((0..3).map(|n| r[i][n] * r[j][n]).sum::<f64>() - (i == j) as u8 as f64).abs() <= 1e-8));
    let determinant = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1]) - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
    orthonormal && (determinant - 1.0).abs() <= 1e-8
}

/// Parses an AliceVision scene with one shared pinhole intrinsic. The checks
/// and their messages are those of `alicevision_cameras.audit_scene`.
pub fn parse_sfm(scene: &Value) -> anyhow::Result<Solution> {
    let mut version = Vec::new();
    for part in list(scene, "version") {
        version.push(util::number(part)? as i64);
    }
    if version.is_empty() || version < vec![1, 2, 11] {
        bail!("unsupported legacy focal serialization");
    }
    let (views, poses, structure, intrinsics) =
        (list(scene, "views"), list(scene, "poses"), list(scene, "structure"), list(scene, "intrinsics"));
    if !(3..=super::MOST_PHOTOS).contains(&views.len())
        || poses.len() > views.len()
        || structure.len() > 1_000_000
        || !(1..=views.len()).contains(&intrinsics.len())
    {
        bail!("invalid bounded native scene inventory");
    }
    let view_name = |view: &Value| file_name(Path::new(view["path"].as_str().unwrap_or_default()));
    let names: Vec<String> = views.iter().map(view_name).collect();
    if names.iter().collect::<HashSet<_>>().len() != names.len() {
        bail!("camera image inventory mismatch");
    }
    let view_ids: Vec<String> = views.iter().map(|v| util::identifier(&v["viewId"])).collect();
    let pose_map: HashMap<String, &Value> = poses.iter().map(|p| (util::identifier(&p["poseId"]), &p["pose"]["transform"])).collect();
    let intrinsic_ids: HashSet<String> = intrinsics.iter().map(|v| util::identifier(&v["intrinsicId"])).collect();
    if view_ids.iter().collect::<HashSet<_>>().len() != views.len()
        || intrinsic_ids.len() != intrinsics.len()
        || pose_map.len() != poses.len()
    {
        bail!("duplicate native identifier");
    }
    if intrinsics.len() != 1 {
        bail!("expected one shared intrinsic");
    }
    let lens = sfm_lens(&intrinsics[0])?;
    let locked = matches!(intrinsics[0].get("locked"), Some(Value::Bool(true)))
        || intrinsics[0].get("locked").and_then(Value::as_str) == Some("true");
    let (mut solved, mut unregistered, mut index_of) = (Vec::new(), Vec::new(), HashMap::new());
    for ((id, view), name) in view_ids.iter().zip(views).zip(&names) {
        if !intrinsic_ids.contains(&util::identifier(&view["intrinsicId"])) {
            bail!("unknown camera intrinsic");
        }
        let Some(pose) = pose_map.get(&util::identifier(&view["poseId"])) else {
            unregistered.push(name.clone());
            continue;
        };
        // Column-major rotation, as `reshape(3, 3, order="F")` reads it.
        let r: [f64; 9] = array(&pose["rotation"])?;
        let rotation = [[r[0], r[3], r[6]], [r[1], r[4], r[7]], [r[2], r[5], r[8]]];
        let centre: Vector = array(&pose["center"])?;
        if !proper(&rotation) {
            bail!("camera rotation is not proper");
        }
        index_of.insert(id.clone(), solved.len());
        solved.push(View { id: id.clone(), source: name.clone(), rotation, centre, translation: translation(&rotation, centre) });
    }
    let mut landmark_ids = HashSet::new();
    let mut landmarks = Vec::with_capacity(structure.len());
    let mut observation_count = 0usize;
    for point in structure {
        let seen = list(point, "observations");
        if !landmark_ids.insert(util::identifier(&point["landmarkId"])) || seen.len() < 2 {
            bail!("duplicate or untriangulated landmark");
        }
        let mut observations = Vec::with_capacity(seen.len());
        let mut observed = HashSet::new();
        for observation in seen {
            let key = util::identifier(&observation["observationId"]);
            observation_count += 1;
            let view = index_of.get(&key).filter(|_| observation_count <= 16_000_000 && observed.insert(key.clone()));
            let Some(&view) = view else { bail!("invalid bounded landmark observations") };
            observations.push((view, array(&observation["x"])?));
        }
        landmarks.push(Landmark { position: array(&point["X"])?, observations });
    }
    Ok(Solution { lens, views: solved, unregistered, landmarks, lens_locked: Some(locked), scale: None, object_points: None })
}

pub fn read_sfm(path: &Path) -> anyhow::Result<Solution> {
    parse_sfm(&util::read_json(path)?).with_context(|| path.display().to_string())
}

/// World-to-camera rotation of a unit quaternion `(w, x, y, z)`, normalised first.
pub fn quaternion_rotation(q: [f64; 4]) -> [Vector; 3] {
    let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let [w, x, y, z] = q.map(|v| v / norm);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/// The COLMAP camera models a `radialk3` lens can represent, by name and by the id of the binary format.
fn colmap_model(name_or_id: &str) -> anyhow::Result<(&'static str, usize)> {
    Ok(match name_or_id {
        "SIMPLE_PINHOLE" | "0" => ("SIMPLE_PINHOLE", 3),
        "PINHOLE" | "1" => ("PINHOLE", 4),
        "SIMPLE_RADIAL" | "2" => ("SIMPLE_RADIAL", 4),
        "RADIAL" | "3" => ("RADIAL", 5),
        "OPENCV" | "4" => ("OPENCV", 8),
        "FULL_OPENCV" | "6" => ("FULL_OPENCV", 12),
        other => bail!("COLMAP camera model {other} cannot be expressed as a pinhole lens with radial k1, k2, k3"),
    })
}

/// COLMAP camera parameters as a [`Lens`]. COLMAP's principal point counts from the image corner.
pub fn colmap_lens(model: &str, width: u64, height: u64, p: &[f64]) -> anyhow::Result<Lens> {
    let (fx, fy, cx, cy, k, zero): (f64, f64, f64, f64, [f64; 3], &[f64]) = match model {
        "SIMPLE_PINHOLE" => (p[0], p[0], p[1], p[2], [0.0; 3], &[]),
        "PINHOLE" => (p[0], p[1], p[2], p[3], [0.0; 3], &[]),
        "SIMPLE_RADIAL" => (p[0], p[0], p[1], p[2], [p[3], 0.0, 0.0], &[]),
        "RADIAL" => (p[0], p[0], p[1], p[2], [p[3], p[4], 0.0], &[]),
        "OPENCV" => (p[0], p[1], p[2], p[3], [p[4], p[5], 0.0], &p[6..8]),
        "FULL_OPENCV" => (p[0], p[1], p[2], p[3], [p[4], p[5], p[8]], &p[6..8]),
        other => bail!("unsupported COLMAP camera model {other}"),
    };
    let rational = if model == "FULL_OPENCV" { &p[9..12] } else { &[][..] };
    if zero.iter().chain(rational).any(|v| *v != 0.0) {
        bail!("the COLMAP camera has tangential or rational distortion terms; only radial k1, k2, k3 are supported");
    }
    if !(1..=16384).contains(&width)
        || !(1..=16384).contains(&height)
        || !(fx > 0.0 && fy > 0.0)
        || ![fx, fy, cx, cy].iter().chain(&k).all(|v| v.is_finite())
    {
        bail!("invalid COLMAP camera");
    }
    Ok(Lens { width: width as u32, height: height as u32, pixels: [fx, fy, cx - 0.5, cy - 0.5], k })
}

/// The `FULL_OPENCV` parameter list COLMAP takes for a [`Lens`]: `fx, fy, cx, cy, k1, k2, p1, p2, k3, k4, k5, k6`.
pub fn colmap_parameters(lens: &Lens) -> [f64; 12] {
    let [fx, fy, cx, cy] = lens.pixels;
    [fx, fy, cx + 0.5, cy + 0.5, lens.k[0], lens.k[1], 0.0, 0.0, lens.k[2], 0.0, 0.0, 0.0]
}

struct ColmapImage {
    id: u64,
    q: [f64; 4],
    t: Vector,
    camera: u64,
    name: String,
    /// `(x, y, point id)` for the keypoints that belong to a point.
    points: Vec<(f64, f64, u64)>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> anyhow::Result<[u8; N]> {
        let slice = self.bytes.get(self.at..self.at + N).ok_or_else(|| anyhow!("truncated COLMAP file"))?;
        self.at += N;
        Ok(slice.try_into().expect("length checked"))
    }
    fn u32(&mut self) -> anyhow::Result<u64> {
        Ok(u32::from_le_bytes(self.take()?) as u64)
    }
    fn u64(&mut self) -> anyhow::Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn f64(&mut self) -> anyhow::Result<f64> {
        Ok(f64::from_le_bytes(self.take()?))
    }
    fn count(&mut self, limit: u64) -> anyhow::Result<usize> {
        let count = self.u64()?;
        if count > limit {
            bail!("implausible count {count} in a COLMAP file");
        }
        Ok(count as usize)
    }
}

type ColmapCameras = HashMap<u64, Lens>;

fn colmap_binary(folder: &Path) -> anyhow::Result<(ColmapCameras, Vec<ColmapImage>, HashMap<u64, Vector>)> {
    let read = |name: &str| crate::photos::fs::read(folder.join(name)).with_context(|| folder.join(name).display().to_string());
    let (bytes, mut cameras) = (read("cameras.bin")?, HashMap::new());
    let mut reader = Reader { bytes: &bytes, at: 0 };
    for _ in 0..reader.count(1 << 20)? {
        let (id, model) = (reader.u32()?, reader.u32()?);
        let (name, parameters) = colmap_model(&model.to_string())?;
        let (width, height) = (reader.u64()?, reader.u64()?);
        let p: Vec<f64> = (0..parameters).map(|_| reader.f64()).collect::<anyhow::Result<_>>()?;
        cameras.insert(id, colmap_lens(name, width, height, &p)?);
    }
    let (bytes, mut images) = (read("images.bin")?, Vec::new());
    let mut reader = Reader { bytes: &bytes, at: 0 };
    for _ in 0..reader.count(1 << 20)? {
        let id = reader.u32()?;
        let q = [reader.f64()?, reader.f64()?, reader.f64()?, reader.f64()?];
        let t = [reader.f64()?, reader.f64()?, reader.f64()?];
        let camera = reader.u32()?;
        let mut name = Vec::new();
        loop {
            let [byte] = reader.take::<1>()?;
            if byte == 0 {
                break;
            }
            name.push(byte);
        }
        let mut points = Vec::new();
        for _ in 0..reader.count(1 << 26)? {
            let (x, y, point) = (reader.f64()?, reader.f64()?, reader.u64()?);
            if point != u64::MAX {
                points.push((x, y, point));
            }
        }
        images.push(ColmapImage { id, q, t, camera, name: String::from_utf8_lossy(&name).to_string(), points });
    }
    let (bytes, mut points) = (read("points3D.bin")?, HashMap::new());
    let mut reader = Reader { bytes: &bytes, at: 0 };
    for _ in 0..reader.count(1 << 26)? {
        let id = reader.u64()?;
        let position = [reader.f64()?, reader.f64()?, reader.f64()?];
        reader.take::<3>()?;
        reader.f64()?;
        let track = reader.count(1 << 26)?;
        reader.at += track * 8;
        points.insert(id, position);
    }
    Ok((cameras, images, points))
}

fn colmap_text(folder: &Path) -> anyhow::Result<(ColmapCameras, Vec<ColmapImage>, HashMap<u64, Vector>)> {
    let read = |name: &str| crate::photos::fs::read_to_string(folder.join(name)).with_context(|| folder.join(name).display().to_string());
    let number = |text: &str| text.parse::<f64>().map_err(|_| anyhow!("not a number in a COLMAP text file: {text:?}"));
    let integer = |text: &str| text.parse::<i64>().map_err(|_| anyhow!("not an integer in a COLMAP text file: {text:?}"));
    let mut cameras = HashMap::new();
    for line in read("cameras.txt")?.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            bail!("malformed cameras.txt line: {line}");
        }
        let (name, parameters) = colmap_model(fields[1])?;
        let p: Vec<f64> = fields[4..].iter().map(|f| number(f)).collect::<anyhow::Result<_>>()?;
        if p.len() != parameters {
            bail!("cameras.txt: {name} needs {parameters} parameters");
        }
        cameras.insert(integer(fields[0])? as u64, colmap_lens(name, integer(fields[2])? as u64, integer(fields[3])? as u64, &p)?);
    }
    let mut images = Vec::new();
    let text = read("images.txt")?;
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.splitn(10, ' ').collect();
        if fields.len() < 10 {
            bail!("malformed images.txt line: {line}");
        }
        let values: Vec<f64> = fields[1..8].iter().map(|f| number(f)).collect::<anyhow::Result<_>>()?;
        let keypoints: Vec<&str> = lines.next().unwrap_or_default().split_whitespace().collect();
        let mut points = Vec::new();
        for triple in keypoints.chunks(3).filter(|c| c.len() == 3) {
            let point = integer(triple[2])?;
            if point >= 0 {
                points.push((number(triple[0])?, number(triple[1])?, point as u64));
            }
        }
        images.push(ColmapImage {
            id: integer(fields[0])? as u64,
            q: [values[0], values[1], values[2], values[3]],
            t: [values[4], values[5], values[6]],
            camera: integer(fields[8])? as u64,
            name: fields[9].trim().to_string(),
            points,
        });
    }
    let mut points = HashMap::new();
    for line in read("points3D.txt")?.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            bail!("malformed points3D.txt line: {line}");
        }
        points.insert(integer(fields[0])? as u64, [number(fields[1])?, number(fields[2])?, number(fields[3])?]);
    }
    Ok((cameras, images, points))
}

/// Reads a COLMAP model directory (`cameras`, `images`, `points3D` as `.bin`
/// or `.txt`). All images must share one camera. `photos` lists the photos
/// that were given to COLMAP, so that the ones without a pose are known.
pub fn read_colmap(folder: &Path, photos: &[String]) -> anyhow::Result<Solution> {
    let (cameras, mut images, points) = if folder.join("images.bin").stored_file() { colmap_binary(folder)? } else { colmap_text(folder)? };
    images.sort_by_key(|image| image.id);
    let used: HashSet<u64> = images.iter().map(|image| image.camera).collect();
    if images.is_empty() || used.len() != 1 {
        bail!(
            "{}: expected registered images sharing one camera, found {} images and {} cameras",
            folder.display(),
            images.len(),
            used.len()
        );
    }
    let lens = *cameras.get(&images[0].camera).ok_or_else(|| anyhow!("images refer to a camera that cameras does not list"))?;
    let mut index: HashMap<u64, usize> = HashMap::new();
    let mut landmarks: Vec<Landmark> = Vec::new();
    let mut views = Vec::new();
    for (n, image) in images.iter().enumerate() {
        let rotation = quaternion_rotation(image.q);
        if !proper(&rotation) || !image.t.iter().all(|v| v.is_finite()) {
            bail!("camera rotation is not proper");
        }
        let (r, t) = (&rotation, image.t);
        let centre = [0, 1, 2].map(|axis| -(r[0][axis] * t[0] + r[1][axis] * t[1] + r[2][axis] * t[2]));
        views.push(View { id: image.id.to_string(), source: file_name(Path::new(&image.name)), rotation, centre, translation: t });
        for &(x, y, point) in &image.points {
            let Some(&position) = points.get(&point) else { bail!("image {} observes a point that points3D does not list", image.name) };
            let slot = *index.entry(point).or_insert_with(|| {
                landmarks.push(Landmark { position, observations: Vec::new() });
                landmarks.len() - 1
            });
            landmarks[slot].observations.push((n, [x - 0.5, y - 0.5]));
        }
    }
    let registered: HashSet<&String> = views.iter().map(|v| &v.source).collect();
    let unregistered = photos.iter().filter(|name| !registered.contains(name)).cloned().collect();
    Ok(Solution { lens, views, unregistered, landmarks, lens_locked: None, scale: None, object_points: None })
}

/// Reads a solution by what the path is: a COLMAP model directory or an AliceVision `.sfm` file.
pub fn read_any(path: &Path, photos: &[String]) -> anyhow::Result<Solution> {
    if path.stored_dir() {
        let nested = ["", "sparse/0", "0"]
            .iter()
            .map(|sub| path.join(sub))
            .find(|dir| dir.join("images.bin").stored_file() || dir.join("images.txt").stored_file());
        return read_colmap(
            &nested.ok_or_else(|| anyhow!("{}: no COLMAP model (images.bin or images.txt) there", path.display()))?,
            photos,
        );
    }
    read_sfm(path)
}

/// Renames the views of an imported solution to the staged capture names. A
/// solution may name its photos like the originals or already `capture_NNNN.png`.
pub fn adopt_capture_names(solution: &mut Solution, photo_map: &Value) -> anyhow::Result<()> {
    let rows = photo_map["photos"].as_array().ok_or_else(|| anyhow!("photo-map.json has no photos"))?;
    let capture_of = |source: &str| -> Option<String> {
        rows.iter()
            .find(|row| row["source"] == source || row["capture"] == source)
            .and_then(|row| row["capture"].as_str())
            .map(String::from)
    };
    for view in &mut solution.views {
        view.source = capture_of(&view.source)
            .ok_or_else(|| anyhow!("the imported solution has a view {} that is not among the photos", view.source))?;
    }
    let listed: std::collections::HashSet<&String> = solution.views.iter().map(|v| &v.source).collect();
    if listed.len() != solution.views.len() {
        bail!("the imported solution lists a photo twice");
    }
    solution.unregistered = rows
        .iter()
        .filter_map(|row| row["capture"].as_str())
        .filter(|name| !listed.contains(&name.to_string()))
        .map(String::from)
        .collect();
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small model: three cameras on the x axis looking along +z, four points, FULL_OPENCV lens.
    pub fn colmap_text_model(folder: &Path) {
        crate::photos::fs::create_dir_all(folder).unwrap();
        crate::photos::fs::write(
            folder.join("cameras.txt"),
            "# Camera list\n1 FULL_OPENCV 100 80 90 100 52.5 39.5 -0.1 0.02 0 0 0.003 0 0 0\n",
        )
        .unwrap();
        let (fx, fy, cx, cy, k) = (90.0, 100.0, 52.5, 39.5, [-0.1, 0.02, 0.003]);
        let points = [[0.1, 0.05, 2.0], [-0.2, 0.1, 2.5], [0.3, -0.1, 3.0], [0.0, 0.0, 2.2]];
        let mut images = String::from("# Image list\n");
        for (i, name) in ["capture_0000.png", "capture_0001.png", "capture_0002.png"].iter().enumerate() {
            let c = [0.1 * i as f64, 0.0, 0.0];
            images += &format!("{} 1 0 0 0 {} 0 0 1 {name}\n", i + 1, -c[0]);
            for (id, p) in points.iter().enumerate() {
                let n = [(p[0] - c[0]) / p[2], p[1] / p[2]];
                let rr = n[0] * n[0] + n[1] * n[1];
                let gain = 1.0 + k[0] * rr + k[1] * rr * rr + k[2] * rr * rr * rr;
                images += &format!("{} {} {} ", fx * n[0] * gain + cx, fy * n[1] * gain + cy, id + 7);
            }
            images += "33.0 44.0 -1\n";
        }
        crate::photos::fs::write(folder.join("images.txt"), images).unwrap();
        let mut text = String::from("# 3D point list\n");
        for (id, p) in points.iter().enumerate() {
            text += &format!("{} {} {} {} 10 20 30 0.5 1 0 2 0 3 0\n", id + 7, p[0], p[1], p[2]);
        }
        crate::photos::fs::write(folder.join("points3D.txt"), text).unwrap();
    }

    /// The binary form of a text model, written field by field as COLMAP's `WriteBinary` does.
    pub fn colmap_binary_from_text(text: &Path, binary: &Path) {
        crate::photos::fs::create_dir_all(binary).unwrap();
        let mut cameras = 1u64.to_le_bytes().to_vec();
        cameras.extend(1u32.to_le_bytes());
        cameras.extend(6i32.to_le_bytes());
        cameras.extend(100u64.to_le_bytes());
        cameras.extend(80u64.to_le_bytes());
        for value in [90.0, 100.0, 52.5, 39.5, -0.1, 0.02, 0.0, 0.0, 0.003, 0.0, 0.0, 0.0f64] {
            cameras.extend(value.to_le_bytes());
        }
        crate::photos::fs::write(binary.join("cameras.bin"), cameras).unwrap();
        let listing = crate::photos::fs::read_to_string(text.join("images.txt")).unwrap();
        let lines: Vec<&str> = listing.lines().filter(|l| !l.starts_with('#')).collect();
        let mut images = ((lines.len() / 2) as u64).to_le_bytes().to_vec();
        for pair in lines.chunks(2) {
            let fields: Vec<&str> = pair[0].split(' ').collect();
            images.extend(fields[0].parse::<u32>().unwrap().to_le_bytes());
            for value in &fields[1..8] {
                images.extend(value.parse::<f64>().unwrap().to_le_bytes());
            }
            images.extend(fields[8].parse::<u32>().unwrap().to_le_bytes());
            images.extend(fields[9].as_bytes());
            images.push(0);
            let keypoints: Vec<&str> = pair[1].split_whitespace().collect();
            images.extend(((keypoints.len() / 3) as u64).to_le_bytes());
            for triple in keypoints.chunks(3) {
                images.extend(triple[0].parse::<f64>().unwrap().to_le_bytes());
                images.extend(triple[1].parse::<f64>().unwrap().to_le_bytes());
                images.extend((triple[2].parse::<i64>().unwrap() as u64).to_le_bytes());
            }
        }
        crate::photos::fs::write(binary.join("images.bin"), images).unwrap();
        let listing = crate::photos::fs::read_to_string(text.join("points3D.txt")).unwrap();
        let lines: Vec<&str> = listing.lines().filter(|l| !l.starts_with('#')).collect();
        let mut points = (lines.len() as u64).to_le_bytes().to_vec();
        for line in lines {
            let fields: Vec<&str> = line.split(' ').collect();
            points.extend(fields[0].parse::<u64>().unwrap().to_le_bytes());
            for value in &fields[1..4] {
                points.extend(value.parse::<f64>().unwrap().to_le_bytes());
            }
            points.extend([10u8, 20, 30]);
            points.extend(0.5f64.to_le_bytes());
            let track = &fields[8..];
            points.extend(((track.len() / 2) as u64).to_le_bytes());
            for value in track {
                points.extend(value.parse::<u32>().unwrap().to_le_bytes());
            }
        }
        crate::photos::fs::write(binary.join("points3D.bin"), points).unwrap();
    }

    #[test]
    fn colmap_text_and_binary_models_convert_conventions() {
        let root = std::env::temp_dir().join(format!("crispmesh-colmap-{}", std::process::id()));
        let _ = crate::photos::fs::remove_dir_all(&root);
        colmap_text_model(&root.join("text"));
        colmap_binary_from_text(&root.join("text"), &root.join("sparse/0"));
        let photos: Vec<String> = (0..4).map(|n| format!("capture_{n:04}.png")).collect();
        let text = read_colmap(&root.join("text"), &photos).unwrap();
        let binary = read_any(&root, &photos).unwrap();
        assert_eq!(text, binary);
        assert_eq!(text.lens, Lens { width: 100, height: 80, pixels: [90.0, 100.0, 52.0, 39.0], k: [-0.1, 0.02, 0.003] });
        assert_eq!(colmap_parameters(&text.lens), [90.0, 100.0, 52.5, 39.5, -0.1, 0.02, 0.0, 0.0, 0.003, 0.0, 0.0, 0.0]);
        assert_eq!(text.unregistered, ["capture_0003.png"]);
        assert_eq!((text.views.len(), text.landmarks.len()), (3, 4));
        assert_eq!((text.views[2].id.as_str(), text.views[2].source.as_str()), ("3", "capture_0002.png"));
        assert_eq!(text.views[2].centre, [0.2, 0.0, 0.0]);
        // Every observation reprojects exactly through the converted lens.
        for landmark in &text.landmarks {
            assert_eq!(landmark.observations.len(), 3);
            for &(view, pixel) in &landmark.observations {
                let c = text.views[view].centre;
                let n = [(landmark.position[0] - c[0]) / landmark.position[2], landmark.position[1] / landmark.position[2]];
                let rr = n[0] * n[0] + n[1] * n[1];
                let gain = 1.0 + text.lens.k[0] * rr + text.lens.k[1] * rr * rr + text.lens.k[2] * rr * rr * rr;
                assert!((text.lens.pixels[0] * n[0] * gain + text.lens.pixels[2] - pixel[0]).abs() < 1e-9);
                assert!((text.lens.pixels[1] * n[1] * gain + text.lens.pixels[3] - pixel[1]).abs() < 1e-9);
            }
        }
        crate::photos::fs::write(root.join("text/cameras.txt"), "1 OPENCV 100 80 90 100 52.5 39.5 -0.1 0.02 0.001 0\n").unwrap();
        assert!(read_colmap(&root.join("text"), &photos).unwrap_err().to_string().contains("tangential"));
        crate::photos::fs::write(root.join("text/cameras.txt"), "1 OPENCV_FISHEYE 100 80 90 100 52.5 39.5 0 0 0 0\n").unwrap();
        assert!(read_colmap(&root.join("text"), &photos).is_err());
        crate::photos::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn quaternions_follow_colmap() {
        // 90 degrees about z: (w, x, y, z) = (cos 45, 0, 0, sin 45) maps x to y.
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let r = quaternion_rotation([h, 0.0, 0.0, h]);
        let x = [r[0][0], r[1][0], r[2][0]];
        assert!((x[0]).abs() < 1e-15 && (x[1] - 1.0).abs() < 1e-15 && x[2].abs() < 1e-15);
        assert!(proper(&quaternion_rotation([0.3, -0.4, 0.5, 0.7])));
        assert_eq!(translation(&[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [1.0, 2.0, 3.0]), [-1.0, -2.0, -3.0]);
    }
}
