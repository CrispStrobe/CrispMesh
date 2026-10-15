//! Camera providers: programs or files that turn the photos (as contrast
//! images in `work/contrast/`, with their masks) and the declared lens into a
//! [`Solution`]. What follows a provider is the same for all of them: audit,
//! ring gates, undistortion and the scene (`run.rs`).
//!
//! - `alicevision`: the commands `photos_to_inputs.py` builds (feature
//!   extraction, matching, global SfM with a locked lens), for an install
//!   prefix called directly or for a wrapper script;
//! - `colmap`: feature extraction with one shared camera fixed to the declared
//!   lens, matching, incremental mapper with fixed intrinsics; the model is
//!   read from `cameras`/`images`/`points3D`;
//! - `import`: an existing `.sfm` file or COLMAP model.

use crate::photos::fs::Stored as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde_json::Value;

use super::calibration::{calibrated_scene, scene_size};
use super::options::{CameraChoice, Options, Tools};
use super::process::ExternalCommand;
use super::providers::{Provider, CAMERAS_ALICEVISION, CAMERAS_COLMAP, CAMERAS_IMPORT};
use super::run::Run;
use super::solution::{adopt_capture_names, colmap_parameters, read_any, read_colmap, read_sfm, Lens, Solution};
use super::staging::capture_name;
use super::util;

/// Contract of the cameras module: photos, lens calibration and masks in;
/// poses of the registered photos, sparse points and the lens model used out.
pub trait CameraProvider {
    fn info(&self) -> &'static Provider;

    /// Fails with a clear message when the provider cannot run with these options (a missing external program).
    fn check(&self, options: &Options) -> anyhow::Result<()>;

    /// Recovers the cameras. `lens` is the declared lens at the photo
    /// resolution, when one was declared. Progress goes from 0.04 to 0.82 of
    /// the `cameras` stage. View sources of the result are `capture_NNNN.png`.
    fn recover(&self, run: &mut Run, lens: Option<&Lens>) -> anyhow::Result<Solution>;

    /// Bulky work files below the output directory that the scene does not need.
    fn intermediates(&self) -> &'static [&'static str];

    /// Whether the provider reads the staged photos (`work/photos`); when not, they are
    /// deleted as soon as the contrast images are written (unless intermediates are kept),
    /// which in a browser keeps them out of memory while cameras are recovered.
    fn reads_staged_photos(&self) -> bool {
        false
    }
}

pub fn camera_provider(options: &Options) -> Box<dyn CameraProvider> {
    match &options.cameras {
        CameraChoice::AliceVision => Box::new(AliceVision),
        CameraChoice::Colmap => Box::new(Colmap),
        CameraChoice::Markers => Box::new(super::markers::provider::Markers),
        CameraChoice::Turntable => Box::new(super::turntable::provider::Turntable),
        CameraChoice::Import(path) => Box::new(Import { path: path.clone() }),
    }
}

fn text(path: PathBuf) -> String {
    path.to_string_lossy().to_string()
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const EXTERNAL_HINT: &str = "is an external program this stage calls";

// ---------------------------------------------------------------------------------------------------------------------
// AliceVision
// ---------------------------------------------------------------------------------------------------------------------

pub struct AliceVision;

pub const ALICEVISION_TOOLS: [&str; 5] = ["cameraInit", "featureExtraction", "imageMatching", "featureMatching", "globalSfM"];

fn alicevision_executable(prefix: &Path, tool: &str) -> PathBuf {
    prefix.join("bin").join(format!("aliceVision_{tool}{}", std::env::consts::EXE_SUFFIX))
}

/// `(command, extra environment)` for one AliceVision tool.
///
/// A prefix gets `ALICEVISION_ROOT` and the library path (`<prefix>/lib`, the
/// extra directories, then whatever the variable already held) under the names
/// the dynamic loaders of macOS and Linux read, and `<prefix>/bin` and
/// `<prefix>/lib` in front of `PATH` on Windows: what the wrapper scripts of a
/// local build do, without an interpreter in between.
#[allow(clippy::type_complexity)]
pub fn alicevision_command(
    tools: &Tools,
    tool: &str,
    arguments: &[String],
    python: Option<&str>,
    environment: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<(Vec<String>, Vec<(String, String)>)> {
    let location = tools.location.to_string_lossy().to_string();
    let mut command;
    let mut extra = Vec::new();
    if tools.location.stored_dir() {
        command = vec![text(alicevision_executable(&tools.location, tool))];
        extra.push(("ALICEVISION_ROOT".to_string(), location));
        let mut libraries = vec![tools.location.join("lib")];
        libraries.extend(tools.library_path.iter().cloned());
        let join = |first: &[PathBuf], name: &str| -> anyhow::Result<String> {
            let inherited = environment(name).filter(|v| !v.is_empty());
            let all = first.iter().cloned().chain(inherited.iter().flat_map(std::env::split_paths));
            Ok(std::env::join_paths(all).map_err(|e| anyhow!("library path: {e}"))?.to_string_lossy().to_string())
        };
        for name in ["DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"] {
            extra.push((name.to_string(), join(&libraries, name)?));
        }
        if cfg!(windows) {
            let mut first = vec![tools.location.join("bin")];
            first.extend(libraries.iter().cloned());
            extra.push(("PATH".to_string(), join(&first, "PATH")?));
        }
        extra.extend(tools.environment.iter().cloned());
    } else if tools.location.extension().is_some_and(|e| e == "py") {
        let python = python
            .ok_or_else(|| anyhow!("{location} is a Python wrapper; give --python / CRISPMESH_PYTHON, or an install prefix instead"))?;
        command = vec![python.to_string(), location, tool.to_string()];
    } else {
        command = vec![location, tool.to_string()];
    }
    command.extend(arguments.iter().cloned());
    Ok((command, extra))
}

/// `cameraSensors.db`: the explicit file, else below the prefix (for a wrapper: below `prefix/` next to it).
pub fn sensor_database(location: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(explicit) = explicit {
        return explicit.to_path_buf();
    }
    let prefix = if location.stored_dir() { location.to_path_buf() } else { location.parent().unwrap_or(Path::new(".")).join("prefix") };
    prefix.join("share").join("aliceVision").join("cameraSensors.db")
}

/// The AliceVision commands of a run, in the order they run.
pub fn alicevision_commands(options: &Options, environment: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Vec<ExternalCommand>> {
    let av = &options.alicevision;
    let tools = av.tools.as_ref().ok_or_else(|| anyhow!("missing tool locations: --alicevision / CRISPMESH_ALICEVISION"))?;
    let out = &options.output;
    let (work, sfm) = (out.join("work"), out.join("sfm"));
    let threads = options.threads.to_string();
    let resource =
        words(&["--maxCoresAvailable", &threads, "--maxMemoryAvailable", &((av.memory_gib * (1u64 << 30) as f64) as i64).to_string()]);
    let mut commands = Vec::new();
    let mut add = |name: &str, tool: &str, arguments: Vec<String>| -> anyhow::Result<()> {
        let arguments: Vec<String> = arguments.into_iter().chain(resource.iter().cloned()).collect();
        let (command, mut extra) = alicevision_command(tools, tool, &arguments, options.python.as_deref(), environment)?;
        // The tools thread through OpenMP; keep the BLAS underneath single-threaded.
        extra.extend([("OPENBLAS_NUM_THREADS".to_string(), "1".to_string()), ("VECLIB_MAXIMUM_THREADS".to_string(), "1".to_string())]);
        commands.push(ExternalCommand { name: name.to_string(), command, environment: extra });
        Ok(())
    };
    let (features, matches) = (text(work.join("features")), text(work.join("matches")));
    let database = text(sensor_database(&tools.location, av.sensor_database.as_deref()));
    let uncalibrated = text(sfm.join("cameraInit-uncalibrated.sfm"));
    let initial = text(sfm.join("cameraInit.sfm"));
    let (pairs, last) = (text(sfm.join("pairs.txt")), text(sfm.join("final.sfm")));
    let seed = options.random_seed.to_string();
    add(
        "cameraInit-uncalibrated",
        "cameraInit",
        words(&[
            "--imageFolder",
            &text(work.join("contrast")),
            "--output",
            &uncalibrated,
            "--sensorDatabase",
            &database,
            "--defaultFieldOfView",
            &util::python_float(av.initial_field_of_view),
            "--groupCameraFallback",
            "global",
        ]),
    )?;
    add(
        "featureExtraction",
        "featureExtraction",
        words(&[
            "--input",
            &uncalibrated,
            "--output",
            &features,
            "--describerTypes",
            &av.describer_types,
            "--describerPreset",
            &av.describer_preset,
            "--forceCpuExtraction",
            "true",
            "--masksFolder",
            &text(out.join("masks")),
            "--maskExtension",
            "png",
            "--maskInvert",
            "false",
            "--maxThreads",
            &threads,
        ]),
    )?;
    add(
        "cameraInit",
        "cameraInit",
        words(&[
            "--input",
            &text(sfm.join("calibrated-input.sfm")),
            "--output",
            &initial,
            "--sensorDatabase",
            &database,
            "--groupCameraFallback",
            "global",
        ]),
    )?;
    add(
        "imageMatching",
        "imageMatching",
        words(&["--input", &initial, "--featuresFolders", &features, "--output", &pairs, "--method", &av.matching_method]),
    )?;
    add(
        "featureMatching",
        "featureMatching",
        words(&[
            "--input",
            &initial,
            "--featuresFolders",
            &features,
            "--imagePairsList",
            &pairs,
            "--output",
            &matches,
            "--describerTypes",
            &av.describer_types,
            "--randomSeed",
            &seed,
        ]),
    )?;
    let mut global = words(&[
        "--input",
        &initial,
        "--featuresFolders",
        &features,
        "--matchesFolders",
        &matches,
        "--output",
        &last,
        "--outputViewsAndPoses",
        &text(sfm.join("views.sfm")),
        "--extraInfoFolder",
        &text(sfm.join("extra")),
        "--lockAllIntrinsics",
        "true",
        "--randomSeed",
        &seed,
    ]);
    global.extend(av.sfm_option.iter().cloned());
    add("globalSfM", "globalSfM", global)?;
    Ok(commands)
}

fn process_environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl CameraProvider for AliceVision {
    fn info(&self) -> &'static Provider {
        &CAMERAS_ALICEVISION
    }

    fn check(&self, options: &Options) -> anyhow::Result<()> {
        let Some(tools) = &options.alicevision.tools else { bail!("missing tool locations: --alicevision / CRISPMESH_ALICEVISION") };
        let location = &tools.location;
        let hint = format!("AliceVision {EXTERNAL_HINT}; give its install prefix or a wrapper script (docs/PHOTOS-TO-INPUTS.md)");
        if location.stored_dir() {
            let absent: Vec<String> = ALICEVISION_TOOLS
                .iter()
                .filter(|tool| !alicevision_executable(location, tool).stored_file())
                .map(|tool| format!("aliceVision_{tool}"))
                .collect();
            if !absent.is_empty() {
                bail!("--alicevision {}: no bin/{} there. {hint}", location.display(), absent.join(", bin/"));
            }
        } else if !location.stored_file() {
            bail!("--alicevision {}: not found. {hint}", location.display());
        } else if location.extension().is_some_and(|e| e == "py") && options.python.is_none() {
            bail!(
                "--alicevision {} is a Python wrapper; give --python / CRISPMESH_PYTHON, or an install prefix instead",
                location.display()
            );
        }
        Ok(())
    }

    fn recover(&self, run: &mut Run, lens: Option<&Lens>) -> anyhow::Result<Solution> {
        let lens = lens.ok_or_else(|| anyhow!("the alicevision provider needs the declared lens (--calibration)"))?;
        let (options, out) = (run.options, run.out);
        for name in ["sfm/extra", "work/matches", "work/features"] {
            crate::photos::fs::create_dir_all(out.join(name))?;
        }
        let commands = alicevision_commands(options, &process_environment)?;
        let command = |name: &str| commands.iter().find(|c| c.name == name).expect("command built above");
        let (count, timeouts) = (options.photo_count, &options.timeouts);
        run.external("cameras", command("cameraInit-uncalibrated"), out, timeouts.small, 0.04, 0.05, "Listing views", None)?;
        let features = out.join("work/features");
        let extracted = || super::run::count_files(&features, "feat") as f64 / count as f64;
        run.external("cameras", command("featureExtraction"), out, timeouts.features, 0.05, 0.30, "Detecting features", Some(&extracted))?;
        let scene = util::read_json(&out.join("sfm/cameraInit-uncalibrated.sfm"))?;
        if scene_size(&scene)? != (lens.width, lens.height) {
            bail!("cameraInit lists the photos at another size than they were staged at");
        }
        util::write_json(&out.join("sfm/calibrated-input.sfm"), &calibrated_scene(&scene, lens)?, 2)?;
        run.external("cameras", command("cameraInit"), out, timeouts.small, 0.30, 0.31, "Applying the declared lens", None)?;
        run.external("cameras", command("imageMatching"), out, timeouts.small, 0.31, 0.32, "Choosing image pairs", None)?;
        run.external("cameras", command("featureMatching"), out, timeouts.matching, 0.32, 0.60, "Matching features", None)?;
        run.external("cameras", command("globalSfM"), out, timeouts.sfm, 0.60, 0.82, "Recovering cameras (global SfM, locked lens)", None)?;
        read_sfm(&out.join("sfm/final.sfm"))
    }

    fn intermediates(&self) -> &'static [&'static str] {
        &["work/features", "work/matches"]
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// COLMAP
// ---------------------------------------------------------------------------------------------------------------------

pub struct Colmap;

fn colmap_program(options: &Options) -> anyhow::Result<Vec<String>> {
    match &options.colmap.location {
        Some(location) if location.extension().is_some_and(|e| e == "py") => {
            let python = options.python.clone().ok_or_else(|| {
                anyhow!("--colmap {} is a Python wrapper; give --python / CRISPMESH_PYTHON, or the executable", location.display())
            })?;
            Ok(vec![python, text(location.clone())])
        }
        Some(location) => Ok(vec![text(location.clone())]),
        None => Ok(vec![format!("colmap{}", std::env::consts::EXE_SUFFIX)]),
    }
}

/// The photos of the closed turn paired with their `overlap` successors, one `name name` pair per line.
pub fn ring_pairs(count: usize, overlap: usize) -> String {
    let mut seen = std::collections::BTreeSet::new();
    for first in 0..count {
        for step in 1..=overlap.min(count.saturating_sub(1)) {
            let second = (first + step) % count;
            if first != second {
                seen.insert((first.min(second), first.max(second)));
            }
        }
    }
    seen.into_iter().map(|(a, b)| format!("{} {}\n", capture_name(a), capture_name(b))).collect()
}

/// Major version in the banner COLMAP prints with its help (`COLMAP 3.9.1 -- Structure-from-Motion ...`).
pub fn colmap_major(banner: &str) -> Option<i64> {
    let rest = banner.split("COLMAP").nth(1)?.trim_start();
    let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
    number.parse().ok()
}

/// Asks the executable for its version; `None` when it does not say (a wrapper, say).
fn detect_colmap_major(options: &Options) -> Option<i64> {
    let program = colmap_program(options).ok()?;
    let output = std::process::Command::new(&program[0]).args(&program[1..]).arg("help").output().ok()?;
    colmap_major(&String::from_utf8_lossy(&output.stdout)).or_else(|| colmap_major(&String::from_utf8_lossy(&output.stderr)))
}

/// The COLMAP commands of a run: `feature_extractor`, a matcher and `mapper`.
///
/// The shared camera is `FULL_OPENCV` with the declared lens as its
/// parameters (`p1 = p2 = k4 = k5 = k6 = 0`, which is exactly the radial k1,
/// k2, k3 model; COLMAP's principal point counts from the image corner, hence
/// half a pixel more than ours), and the mapper refines neither focal length
/// nor principal point nor distortion.
pub fn colmap_commands(options: &Options, lens: &Lens) -> anyhow::Result<Vec<ExternalCommand>> {
    let colmap = &options.colmap;
    let out = &options.output;
    let work = out.join("sfm/colmap");
    let (database, images) = (text(work.join("database.db")), text(out.join("work/contrast")));
    let threads = options.threads.to_string();
    let program = colmap_program(options)?;
    let parameters = colmap_parameters(lens).iter().map(|v| util::python_float(*v)).collect::<Vec<_>>().join(",");
    // COLMAP 4 moved the options every feature type shares out of the SIFT groups.
    let (extraction, matching) =
        if colmap.cli >= 4 { ("FeatureExtraction", "FeatureMatching") } else { ("SiftExtraction", "SiftMatching") }; // 0 (not asked) is treated as 3
    let step = |name: &str, command: &str, arguments: Vec<String>, more: &[String]| ExternalCommand {
        name: name.to_string(),
        command: program.iter().cloned().chain([command.to_string()]).chain(arguments).chain(more.iter().cloned()).collect(),
        environment: Vec::new(),
    };
    let mut extractor = words(&[
        "--database_path",
        &database,
        "--image_path",
        &images,
        "--ImageReader.single_camera",
        "1",
        "--ImageReader.camera_model",
        "FULL_OPENCV",
        "--ImageReader.camera_params",
        &parameters,
        &format!("--{extraction}.use_gpu"),
        "0",
        &format!("--{extraction}.num_threads"),
        &threads,
        "--SiftExtraction.max_num_features",
        &colmap.max_features.to_string(),
    ]);
    if colmap.use_masks {
        // One mask per photo, named like the photo plus `.png`; no features where it is black.
        extractor.extend(words(&["--ImageReader.mask_path", &text(out.join("work/mask-cleanup/masks"))]));
    }
    let shared =
        words(&["--database_path", &database, &format!("--{matching}.use_gpu"), "0", &format!("--{matching}.num_threads"), &threads]);
    let overlap = colmap.overlap.to_string();
    let (matcher, extra) = match colmap.matching.as_str() {
        "exhaustive" => ("exhaustive_matcher", vec![]),
        "sequential" => (
            "sequential_matcher",
            words(&[
                "--SequentialMatching.overlap",
                &overlap,
                "--SequentialMatching.quadratic_overlap",
                "0",
                "--SequentialMatching.loop_detection",
                "0",
            ]),
        ),
        _ => ("matches_importer", words(&["--match_list_path", &text(work.join("pairs.txt")), "--match_type", "pairs"])),
    };
    // The mapper refuses to register photos whose camera has a distortion coefficient above
    // `max_extra_param` (1 by default), taking it for a diverged estimate. A declared lens is not
    // one: the 3DLF lens has k3 = -3.7.
    let extra_bound = lens.k.iter().fold(1f64, |bound, k| bound.max(2.0 * k.abs())).ceil();
    let mapper = words(&[
        "--database_path",
        &database,
        "--image_path",
        &images,
        "--output_path",
        &text(work.join("sparse")),
        "--Mapper.ba_refine_focal_length",
        "0",
        "--Mapper.ba_refine_principal_point",
        "0",
        "--Mapper.ba_refine_extra_params",
        "0",
        "--Mapper.multiple_models",
        "0",
        "--Mapper.max_extra_param",
        &util::python_float(extra_bound),
        "--Mapper.num_threads",
        &threads,
    ]);
    Ok(vec![
        step("colmap-feature_extractor", "feature_extractor", extractor, &colmap.extractor_option),
        step(&format!("colmap-{matcher}"), matcher, shared.into_iter().chain(extra).collect(), &colmap.matcher_option),
        step("colmap-mapper", "mapper", mapper, &colmap.mapper_option),
    ])
}

/// The model directory below `sparse/` with the most registered images.
fn largest_model(sparse: &Path, photos: &[String]) -> anyhow::Result<(PathBuf, Solution)> {
    let mut best: Option<(PathBuf, Solution)> = None;
    let mut folders: Vec<PathBuf> = crate::photos::fs::list(sparse).with_context(|| sparse.display().to_string())?;
    folders.sort();
    for folder in folders.into_iter().filter(|f| f.join("images.bin").stored_file() || f.join("images.txt").stored_file()) {
        let solution = read_colmap(&folder, photos)?;
        if best.as_ref().is_none_or(|(_, held)| solution.views.len() > held.views.len()) {
            best = Some((folder, solution));
        }
    }
    best.ok_or_else(|| anyhow!("the COLMAP mapper wrote no model: no photo pair gave an initial reconstruction"))
}

impl CameraProvider for Colmap {
    fn info(&self) -> &'static Provider {
        &CAMERAS_COLMAP
    }

    fn check(&self, options: &Options) -> anyhow::Result<()> {
        let hint =
            format!("COLMAP {EXTERNAL_HINT}; give the executable or a wrapper with --colmap / CRISPMESH_COLMAP (docs/PHOTOS-TO-INPUTS.md)");
        match &options.colmap.location {
            Some(location) if !location.stored_file() => bail!("--colmap {}: not found. {hint}", location.display()),
            Some(_) => colmap_program(options).map(|_| ()),
            None => {
                let name = format!("colmap{}", std::env::consts::EXE_SUFFIX);
                let found = std::env::var_os("PATH")
                    .is_some_and(|path| std::env::split_paths(&path).any(|folder| folder.join(&name).stored_file()));
                if !found {
                    bail!("no colmap on the PATH. {hint}");
                }
                Ok(())
            }
        }
    }

    fn recover(&self, run: &mut Run, lens: Option<&Lens>) -> anyhow::Result<Solution> {
        let lens = lens.ok_or_else(|| anyhow!("the colmap provider needs the declared lens (--calibration)"))?;
        let (options, out) = (run.options, run.out);
        let work = out.join("sfm/colmap");
        crate::photos::fs::create_dir_all(work.join("sparse"))?;
        let count = options.photo_count;
        crate::photos::fs::write(work.join("pairs.txt"), ring_pairs(count, options.colmap.overlap as usize))?;
        // The option names of the feature steps changed with COLMAP 4; ask the executable unless told.
        let mut versioned = options.clone();
        if versioned.colmap.cli == 0 {
            let found = detect_colmap_major(options);
            run.note("colmap_major_version", found.map(Value::from).unwrap_or(Value::Null));
            versioned.colmap.cli = found.unwrap_or(3);
        }
        let commands = colmap_commands(&versioned, lens)?;
        let timeouts = &options.timeouts;
        run.external("cameras", &commands[0], out, timeouts.features, 0.04, 0.30, "Detecting features (COLMAP)", None)?;
        run.external("cameras", &commands[1], out, timeouts.matching, 0.30, 0.60, "Matching features (COLMAP)", None)?;
        run.external("cameras", &commands[2], out, timeouts.sfm, 0.60, 0.82, "Recovering cameras (COLMAP mapper, fixed lens)", None)?;
        let photos: Vec<String> = (0..count).map(capture_name).collect();
        let (folder, solution) = largest_model(&work.join("sparse"), &photos)?;
        run.note("colmap_model", Value::String(text(folder)));
        Ok(solution)
    }

    fn intermediates(&self) -> &'static [&'static str] {
        &["sfm/colmap/database.db"]
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------------------------------

pub struct Import {
    pub path: PathBuf,
}

impl CameraProvider for Import {
    fn info(&self) -> &'static Provider {
        &CAMERAS_IMPORT
    }

    fn check(&self, _options: &Options) -> anyhow::Result<()> {
        if !self.path.stored() {
            bail!("--cameras import:{} does not exist", self.path.display());
        }
        Ok(())
    }

    fn recover(&self, run: &mut Run, lens: Option<&Lens>) -> anyhow::Result<Solution> {
        let out = run.out;
        let path = self.path.clone();
        let small = run.options.timeouts.small;
        let mut solution = run.internal("cameras", "import-cameras", small, 0.04, 0.82, "Reading the supplied cameras", 1, |_| {
            let photo_map = util::read_json(&out.join("photo-map.json"))?;
            let photos: Vec<String> =
                photo_map["photos"].as_array().into_iter().flatten().filter_map(|row| row["source"].as_str()).map(String::from).collect();
            let mut solution = read_any(&path, &photos)?;
            adopt_capture_names(&mut solution, &photo_map)?;
            Ok(solution)
        })?;
        if let Some(lens) = lens {
            if (solution.lens.width, solution.lens.height) != (lens.width, lens.height) {
                bail!(
                    "the imported cameras are for {}x{} photos, these are {}x{}",
                    solution.lens.width,
                    solution.lens.height,
                    lens.width,
                    lens.height
                );
            }
        }
        // An imported lens is taken as it is; whether it was held fixed is not this program's to say.
        solution.lens_locked = None;
        Ok(solution)
    }

    fn intermediates(&self) -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::options::resolve;
    use crate::photos::options::tests::{arguments, none, scratch};

    fn after<'a>(command: &'a [String], flag: &str) -> &'a str {
        &command[command.iter().position(|w| w == flag).unwrap_or_else(|| panic!("{flag} missing in {command:?}")) + 1]
    }

    fn named<'a>(commands: &'a [ExternalCommand], name: &str) -> &'a ExternalCommand {
        commands.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no command {name}"))
    }

    #[test]
    fn wrapper_and_prefix_forms() {
        let folder = scratch("forms");
        let wrapper = Tools { location: folder.join("av.py"), ..Tools::default() };
        let arguments = ["--input".to_string(), "3".to_string()];
        let (command, extra) = alicevision_command(&wrapper, "globalSfM", &arguments, Some("/sci/python"), &none).unwrap();
        assert_eq!(command, ["/sci/python", &folder.join("av.py").to_string_lossy(), "globalSfM", "--input", "3"]);
        assert!(extra.is_empty());
        assert!(alicevision_command(&wrapper, "globalSfM", &arguments, None, &none).unwrap_err().to_string().contains("--python"));
        let plain = Tools { location: folder.join("av"), ..Tools::default() };
        assert_eq!(
            alicevision_command(&plain, "cameraInit", &[], None, &none).unwrap().0,
            [&*folder.join("av").to_string_lossy(), "cameraInit"]
        );
        let prefix = Tools {
            location: folder.clone(),
            library_path: vec![PathBuf::from("/opt/extra/lib")],
            environment: vec![("ACPP_VISIBILITY_MASK".to_string(), "omp".to_string())],
        };
        let inherited = |name: &str| (name == "DYLD_LIBRARY_PATH").then(|| "/already/there".to_string());
        let (command, extra) = alicevision_command(&prefix, "cameraInit", &["-x".to_string()], None, &inherited).unwrap();
        assert_eq!(command, [&*alicevision_executable(&folder, "cameraInit").to_string_lossy(), "-x"]);
        let value = |name: &str| extra.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(value("ALICEVISION_ROOT"), folder.to_string_lossy());
        let expected: Vec<PathBuf> = vec![folder.join("lib"), "/opt/extra/lib".into(), "/already/there".into()];
        assert_eq!(std::env::split_paths(&value("DYLD_LIBRARY_PATH")).collect::<Vec<_>>(), expected);
        assert_eq!(std::env::split_paths(&value("LD_LIBRARY_PATH")).collect::<Vec<_>>(), expected[..2]);
        assert_eq!(value("ACPP_VISIBILITY_MASK"), "omp");
        assert_eq!(sensor_database(&folder, None), folder.join("share/aliceVision/cameraSensors.db"));
        assert_eq!(sensor_database(&folder.join("av.py"), None), folder.join("prefix/share/aliceVision/cameraSensors.db"));
        assert_eq!(sensor_database(&folder, Some(Path::new("/else/db"))), Path::new("/else/db"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn alicevision_defaults_reproduce_the_hand_driven_commands() {
        let folder = scratch("av-defaults");
        let wrapper = folder.join("av.py").to_string_lossy().to_string();
        let more = [
            "--alicevision",
            &wrapper,
            "--python",
            "/sci/python",
            "--sfm-option",
            "--a",
            "--sfm-option=b",
            "--threads",
            "3",
            "--random-seed",
            "7",
        ];
        let options = resolve(&arguments(&folder, &more), &none).unwrap();
        AliceVision.check(&options).unwrap();
        let out = folder.join("out");
        let commands = alicevision_commands(&options, &none).unwrap();
        assert_eq!(
            commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["cameraInit-uncalibrated", "featureExtraction", "cameraInit", "imageMatching", "featureMatching", "globalSfM"]
        );
        let listing = &named(&commands, "cameraInit-uncalibrated").command;
        assert_eq!(listing[..3], ["/sci/python".to_string(), wrapper.clone(), "cameraInit".to_string()]);
        assert_eq!(PathBuf::from(after(listing, "--imageFolder")), out.join("work/contrast"));
        assert_eq!(after(listing, "--defaultFieldOfView"), "45.0");
        assert_eq!(PathBuf::from(after(listing, "--sensorDatabase")), folder.join("prefix/share/aliceVision/cameraSensors.db"));
        let features = &named(&commands, "featureExtraction").command;
        for (flag, value) in [
            ("--describerTypes", "sift".to_string()),
            ("--describerPreset", "normal".to_string()),
            ("--forceCpuExtraction", "true".to_string()),
            ("--masksFolder", text(out.join("masks"))),
            ("--maskExtension", "png".to_string()),
            ("--maxThreads", "3".to_string()),
            ("--maxCoresAvailable", "3".to_string()),
            ("--maxMemoryAvailable", (4u64 << 30).to_string()),
        ] {
            assert_eq!(after(features, flag), value, "{flag}");
        }
        assert_eq!(PathBuf::from(after(&named(&commands, "cameraInit").command, "--input")), out.join("sfm/calibrated-input.sfm"));
        assert_eq!(after(&named(&commands, "imageMatching").command, "--method"), "Exhaustive");
        assert_eq!(PathBuf::from(after(&named(&commands, "featureMatching").command, "--imagePairsList")), out.join("sfm/pairs.txt"));
        assert_eq!(after(&named(&commands, "featureMatching").command, "--randomSeed"), "7");
        let sfm = &named(&commands, "globalSfM").command;
        assert_eq!((after(sfm, "--lockAllIntrinsics"), after(sfm, "--randomSeed")), ("true", "7"));
        assert_eq!(PathBuf::from(after(sfm, "--output")), out.join("sfm/final.sfm"));
        assert_eq!(sfm[sfm.len() - 6..sfm.len() - 4], ["--a", "b"]);
        let blas = named(&commands, "globalSfM").environment.iter().find(|(k, _)| k == "OPENBLAS_NUM_THREADS").unwrap();
        assert_eq!(blas.1, "1");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn missing_external_programs_are_named() {
        let folder = scratch("missing");
        let wrapper = folder.join("av.py").to_string_lossy().to_string();
        let check = |more: &[&str]| -> String {
            let all: Vec<&str> = ["--cameras", "alicevision"].iter().chain(more).copied().collect();
            let options = resolve(&arguments(&folder, &all), &none).unwrap();
            camera_provider(&options).check(&options).err().map(|e| e.to_string()).unwrap_or_default()
        };
        assert_eq!(check(&[]), "missing tool locations: --alicevision / CRISPMESH_ALICEVISION");
        assert!(check(&["--alicevision", "/no/such/prefix"]).contains("not found. AliceVision is an external program"));
        std::fs::create_dir_all(folder.join("prefix/bin")).unwrap();
        let prefix = folder.join("prefix").to_string_lossy().to_string();
        assert!(check(&["--alicevision", &prefix]).contains("no bin/aliceVision_cameraInit, bin/aliceVision_featureExtraction"));
        assert!(check(&["--alicevision", &wrapper]).contains("Python wrapper"));
        assert_eq!(check(&["--alicevision", &wrapper, "--python", "/sci/python"]), "");
        assert!(check(&["--cameras", "colmap", "--colmap", "/no/such/colmap"]).contains("not found. COLMAP is an external program"));
        assert!(check(&["--cameras", "colmap", "--colmap", &wrapper]).contains("Python wrapper"));
        assert_eq!(check(&["--cameras", "colmap", "--colmap", &wrapper, "--python", "/sci/python"]), "");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn colmap_version_is_read_from_its_banner() {
        assert_eq!(
            colmap_major("COLMAP 3.9.1 -- Structure-from-Motion and Multi-View Stereo\n(Commit Unknown on Unknown without CUDA)"),
            Some(3)
        );
        assert_eq!(colmap_major("\nCOLMAP 4.0.2 (Commit abc)"), Some(4));
        assert_eq!(colmap_major("usage: something else"), None);
    }

    #[test]
    fn colmap_commands_fix_the_declared_lens() {
        let folder = scratch("colmap");
        let out = folder.join("out");
        let lens = Lens { width: 1749, height: 1155, pixels: [2328.25, 2329.5, 874.125, 555.75], k: [-0.17, 0.41, -3.7] };
        let program = text(std::path::absolute("/opt/colmap").unwrap());
        let options = resolve(&arguments(&folder, &["--cameras", "colmap", "--colmap", &program, "--threads", "4"]), &none).unwrap();
        let commands = colmap_commands(&options, &lens).unwrap();
        assert_eq!(
            commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["colmap-feature_extractor", "colmap-exhaustive_matcher", "colmap-mapper"]
        );
        let extractor = &commands[0].command;
        assert_eq!(extractor[..2], [program.as_str(), "feature_extractor"]);
        assert_eq!(PathBuf::from(after(extractor, "--database_path")), out.join("sfm/colmap/database.db"));
        assert_eq!(PathBuf::from(after(extractor, "--image_path")), out.join("work/contrast"));
        assert_eq!((after(extractor, "--ImageReader.single_camera"), after(extractor, "--ImageReader.camera_model")), ("1", "FULL_OPENCV"));
        // fx, fy, cx + 0.5, cy + 0.5, k1, k2, p1, p2, k3, k4, k5, k6
        assert_eq!(after(extractor, "--ImageReader.camera_params"), "2328.25,2329.5,874.625,556.25,-0.17,0.41,0.0,0.0,-3.7,0.0,0.0,0.0");
        assert_eq!(PathBuf::from(after(extractor, "--ImageReader.mask_path")), out.join("work/mask-cleanup/masks"));
        assert_eq!((after(extractor, "--SiftExtraction.use_gpu"), after(extractor, "--SiftExtraction.num_threads")), ("0", "4"));
        assert_eq!(after(&commands[1].command, "--SiftMatching.use_gpu"), "0");
        let mapper = &commands[2].command;
        for flag in ["--Mapper.ba_refine_focal_length", "--Mapper.ba_refine_principal_point", "--Mapper.ba_refine_extra_params"] {
            assert_eq!(after(mapper, flag), "0", "{flag}");
        }
        assert_eq!(PathBuf::from(after(mapper, "--output_path")), out.join("sfm/colmap/sparse"));
        assert_eq!(after(mapper, "--Mapper.max_extra_param"), "8.0"); // twice the largest coefficient, 3.7, rounded up
        let wrapper = folder.join("av.py").to_string_lossy().to_string();
        let more = [
            "--cameras",
            "colmap",
            "--colmap",
            &wrapper,
            "--python",
            "/sci/python",
            "--colmap-matching",
            "ring",
            "--colmap-masks",
            "off",
            "--colmap-cli",
            "4",
            "--colmap-mapper-option",
            "--Mapper.min_num_matches",
            "--colmap-mapper-option",
            "10",
        ];
        let options = resolve(&arguments(&folder, &more), &none).unwrap();
        let commands = colmap_commands(&options, &lens).unwrap();
        assert_eq!(commands[0].command[..3], ["/sci/python".to_string(), wrapper.clone(), "feature_extractor".to_string()]);
        assert!(!commands[0].command.contains(&"--ImageReader.mask_path".to_string()));
        assert_eq!(after(&commands[0].command, "--FeatureExtraction.use_gpu"), "0");
        assert_eq!(commands[1].name, "colmap-matches_importer");
        assert_eq!(after(&commands[1].command, "--match_type"), "pairs");
        assert_eq!(PathBuf::from(after(&commands[1].command, "--match_list_path")), out.join("sfm/colmap/pairs.txt"));
        assert_eq!(commands[2].command[commands[2].command.len() - 2..], ["--Mapper.min_num_matches", "10"]);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn ring_pairs_close_the_turn() {
        assert_eq!(ring_pairs(4, 1), "capture_0000.png capture_0001.png\ncapture_0000.png capture_0003.png\ncapture_0001.png capture_0002.png\ncapture_0002.png capture_0003.png\n");
        assert_eq!(ring_pairs(5, 10).lines().count(), 10); // every pair once
        assert_eq!(ring_pairs(73, 10).lines().count(), 730);
        assert!(ring_pairs(1, 3).is_empty());
    }

    #[test]
    fn imported_views_take_the_capture_names() {
        let root = std::env::temp_dir().join(format!("crispmesh-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        crate::photos::solution::tests::colmap_text_model(&root);
        let photos: Vec<String> = ["b_2.png", "b_10.png", "capture_0002.png", "extra.png"].map(String::from).to_vec();
        let mut solution = read_any(&root, &photos).unwrap();
        // The model names its images capture_0000..2; pretend two of them carried original names.
        solution.views[0].source = "b_2.png".into();
        solution.views[1].source = "b_10.png".into();
        let map = serde_json::json!({"photos": [
            {"capture": "capture_0000.png", "source": "b_2.png"}, {"capture": "capture_0001.png", "source": "b_10.png"},
            {"capture": "capture_0002.png", "source": "b_11.png"}, {"capture": "capture_0003.png", "source": "b_12.png"}]});
        adopt_capture_names(&mut solution, &map).unwrap();
        assert_eq!(
            solution.views.iter().map(|v| v.source.as_str()).collect::<Vec<_>>(),
            ["capture_0000.png", "capture_0001.png", "capture_0002.png"]
        );
        assert_eq!(solution.unregistered, ["capture_0003.png"]);
        solution.views[2].source = "stranger.png".into();
        assert!(adopt_capture_names(&mut solution, &map).unwrap_err().to_string().contains("not among the photos"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
