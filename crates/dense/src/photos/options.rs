//! Command line and resolved configuration of the photo front stage (`parser`
//! and `resolve` of `photos_to_inputs.py`, reorganised around providers: one
//! for the masks, one for the cameras, each with its own namespaced options).

use crate::photos::fs::Stored as _;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail};
use serde_json::{json, Value};

use super::coarse::Threshold;
use super::option_table;
use super::staging::{list_photos, PHOTO_SUFFIXES};

pub const SCHEMA: &str = "crisp3ds_photos_to_inputs_v1";

/// Default dark-hole cleanup budget for the SAM mask providers (the others: the option table's 0.02).
pub const SAM_HOLE_CLEANUP_BUDGET: f64 = 0.05;

/// Where the object masks come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskChoice {
    /// Dark object on a light backdrop: threshold, largest dark region, hole cleanup. No network.
    Threshold,
    /// Centred foreground against a peripheral colour backdrop.
    Background,
    /// Masks made elsewhere, one per photo; the hole cleanup still runs.
    Import(PathBuf),
    /// SAM 2.1 through `scripts/turntable_mesh/segment.py` in an external interpreter, then the cleanup.
    ExternalSam,
    /// SAM 2.1 in this process (`photos/sam`), then the cleanup.
    Sam,
}

impl MaskChoice {
    pub fn name(&self) -> &'static str {
        match self {
            MaskChoice::Threshold => "threshold",
            MaskChoice::Background => "background",
            MaskChoice::Import(_) => "import",
            MaskChoice::ExternalSam => "external-sam",
            MaskChoice::Sam => "sam",
        }
    }
}

/// Where the camera poses come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraChoice {
    AliceVision,
    Colmap,
    /// A printed marker mat under the object (`photos/markers`).
    Markers,
    /// Our own solver for ordered turntable photos (`photos/turntable`).
    Turntable,
    /// An existing solution: an AliceVision `.sfm` file or a COLMAP model directory.
    Import(PathBuf),
}

impl CameraChoice {
    pub fn name(&self) -> &'static str {
        match self {
            CameraChoice::AliceVision => "alicevision",
            CameraChoice::Colmap => "colmap",
            CameraChoice::Markers => "markers",
            CameraChoice::Turntable => "turntable",
            CameraChoice::Import(_) => "import",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Timeouts {
    pub small: u64,
    pub sam: u64,
    pub features: u64,
    pub matching: u64,
    pub sfm: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gates {
    pub minimum_registered_fraction: f64,
    pub minimum_observations_per_view: i64,
    pub maximum_view_reprojection_p95_pixels: f64,
    pub minimum_positive_depth_fraction: f64,
    pub maximum_radius_spread_percent: f64,
    pub maximum_out_of_plane_percent: f64,
    pub maximum_angular_gap_deg: f64,
    pub maximum_reversed_steps: i64,
    pub maximum_optical_axis_miss_percent: f64,
    pub duplicate_step_deg: f64,
    /// The photos are one closed turn: the closure gate applies.
    pub full_turn: bool,
    /// The cameras lie on one ring (`--capture turntable`): the ring gates apply. Off for
    /// `--capture orbit`, which keeps the camera audit only.
    pub ring: bool,
    pub maximum_closing_step_ratio: f64,
}

impl Gates {
    pub fn to_json(&self) -> Value {
        json!({
            "minimum_registered_fraction": self.minimum_registered_fraction,
            "minimum_observations_per_view": self.minimum_observations_per_view,
            "maximum_view_reprojection_p95_pixels": self.maximum_view_reprojection_p95_pixels,
            "minimum_positive_depth_fraction": self.minimum_positive_depth_fraction,
            "maximum_radius_spread_percent": self.maximum_radius_spread_percent,
            "maximum_out_of_plane_percent": self.maximum_out_of_plane_percent,
            "maximum_angular_gap_deg": self.maximum_angular_gap_deg,
            "maximum_reversed_steps": self.maximum_reversed_steps,
            "maximum_optical_axis_miss_percent": self.maximum_optical_axis_miss_percent,
            "duplicate_step_deg": self.duplicate_step_deg,
            "full_turn": self.full_turn,
            "ring_gates": self.ring,
            "maximum_closing_step_ratio": self.maximum_closing_step_ratio,
        })
    }
}

/// One AliceVision build: where it is and what its tools need in their environment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tools {
    /// Install prefix, executable wrapper or `.py` wrapper.
    pub location: PathBuf,
    /// Extra library directories after `<prefix>/lib` (prefix form only).
    pub library_path: Vec<PathBuf>,
    /// Extra environment variables (prefix form only).
    pub environment: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AliceVisionOptions {
    pub tools: Option<Tools>,
    pub sensor_database: Option<PathBuf>,
    pub memory_gib: f64,
    pub initial_field_of_view: f64,
    pub describer_types: String,
    pub describer_preset: String,
    pub matching_method: String,
    pub sfm_option: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColmapOptions {
    /// The `colmap` executable, or a wrapper (a `.py` wrapper is run with `--python`).
    pub location: Option<PathBuf>,
    /// Major version of the command line: 3 (`SiftExtraction.*`) or 4 (`FeatureExtraction.*`);
    /// 0 asks the executable for its version when the run starts.
    pub cli: i64,
    /// `exhaustive`, `sequential` or `ring`.
    pub matching: String,
    pub overlap: i64,
    pub use_masks: bool,
    pub max_features: i64,
    pub extractor_option: Vec<String>,
    pub matcher_option: Vec<String>,
    pub mapper_option: Vec<String>,
}

/// Options of the `turntable` camera provider.
#[derive(Debug, Clone, PartialEq)]
pub struct TurntableOptions {
    /// Most features kept per photo.
    pub features: usize,
    /// Every photo is matched with this many successors.
    pub span: usize,
    /// Debug input: features and matches made elsewhere (`crisp3ds_turntable_matches_v1`).
    pub matches: Option<PathBuf>,
    /// Include moving turntable features while retaining object-only scene bounds.
    pub surface: bool,
    /// Longest side, in pixels, of the photos the cameras are solved on (0: the photo resolution).
    pub working_size: usize,
    /// The photos are this many complete turns of equal length, one after the other (camera heights).
    pub rings: usize,
}

/// Options of the `markers` camera provider.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkersOptions {
    /// Description of the printed mat (`crisp3ds_marker_mat_v1`).
    pub mat: Option<PathBuf>,
    /// A photo with fewer decoded markers gets no pose.
    pub minimum_per_photo: usize,
    /// Height over width of the print relative to the design, when measured.
    pub aspect: Option<f64>,
    /// Estimate that number from all photos instead.
    pub aspect_auto: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SamOptions {
    /// RGB backdrop contrast instead of the legacy dark-object prompt seed.
    pub background_prompt: bool,
    pub python: Option<String>,
    pub source: Option<PathBuf>,
    pub checkpoint: Option<PathBuf>,
    pub config: Option<String>,
    pub pythonpath: Vec<String>,
    pub repository: Option<PathBuf>,
    pub device: String,
    pub multimask: bool,
    pub preserve_holes: bool,
    pub automatic_cues: bool,
    /// Model directory of the native provider (`model.json` and the graphs).
    pub model: Option<PathBuf>,
    /// ONNX Runtime's shared library, for the native provider's ONNX backend.
    pub runtime: Option<PathBuf>,
    /// `cpu` or an accelerator of the native provider's backend.
    pub accelerator: String,
    /// The native provider chooses among SAM's three proposals (`--sam-candidates several`) instead of taking its single mask.
    pub several_candidates: bool,
}

/// The resolved options: flags, then environment variables, then defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub photos: PathBuf,
    pub photo_count: usize,
    pub calibration: Option<PathBuf>,
    pub output: PathBuf,
    pub events: Option<PathBuf>,
    pub masks: MaskChoice,
    pub cameras: CameraChoice,
    pub python: Option<String>,
    pub threads: usize,
    pub minimum_free_gib: f64,
    pub keep_intermediates: bool,
    pub stop_after_masks: bool,
    pub envelope: String,
    pub dark_threshold: Threshold,
    /// Share of the way from the object's grey to the threshold that still counts as object core when the
    /// contact shadow is taken out of threshold masks; 0: not done (and never for the prompts of external-sam).
    pub threshold_shadow: f64,
    pub hole_cleanup_budget: f64,
    pub sam: SamOptions,
    pub contrast_gamma: f64,
    pub clahe_clip: f64,
    pub clahe_grid: usize,
    pub alicevision: AliceVisionOptions,
    pub colmap: ColmapOptions,
    pub markers: MarkersOptions,
    pub turntable: TurntableOptions,
    /// The photos do not close a full turn.
    pub open_turn: bool,
    pub random_seed: i64,
    pub timeouts: Timeouts,
    pub gates: Gates,
}

pub const USAGE: &str = "\
usage: crisp3ds-dense photos --photos DIR --calibration JSON --output DIR [--events PATH]
                             [--masks PROVIDER] [--cameras PROVIDER] [options]
       crisp3ds-dense photos --list-providers
       crisp3ds-dense photos --list-options      every option below as JSON (flag, kind, default, choices, meaning)

From a folder of turntable photos to the scene the dense stages read (inputs/:
cameras.json, undistorted images/ and masks/, sparse_points.npy). Masks and
cameras come from interchangeable providers; undistortion, gates and the scene
are this program's. Exit code 0: complete; 2: cameras rejected by a gate (or a
usage error); 1: a step failed. See docs/PHOTOS-TO-INPUTS.md.

--masks background              centred object against a backdrop; RGB/colour contrast, no network
        threshold               dark object on a light backdrop: threshold, largest dark region, contact shadow taken
                                out, hole cleanup; no network, no external program (default)
        import:DIR              masks made elsewhere: one 8-bit PNG per photo, white object, named capture_NNNN.png in
                                capture order or like the photo; the hole cleanup still runs
        external-sam            SAM 2.1 through scripts/turntable_mesh/segment.py in an external Python interpreter
        sam                     SAM 2.1 in this process, from a model directory (builds with the feature sam-onnx)
--cameras colmap                external COLMAP executable, incremental mapper with the declared lens fixed
          alicevision           external AliceVision executables, global SfM with the declared lens locked
          turntable             our own solver for one turn of ordered photos; no external program (default)
          markers               a printed marker mat under the object: exact poses in millimetres, no external program
          import:PATH           an existing solution: AliceVision .sfm file or COLMAP model directory (text or binary)

masks, threshold provider (also makes the prompts of external-sam and the red overlay of the sheet):
  --threshold-level N|otsu        grey level below which a pixel is object (otsu per photo for --masks threshold,
                                  else 70)                                                 [alias --dark-threshold]
  --threshold-envelope SPEC       auto (whole frame) or x0,y0,x1,y1 in pixels, fractions if all <= 1  [alias --envelope]
  --threshold-shadow F            takes the contact shadow out of threshold masks: a mask pixel stays only with object
                                  core (darker than this share of the way from the object's grey to the level) at or
                                  below it in its column (0.25; 0 disables)
  --hole-cleanup-budget F         largest share of the foreground the dark-hole fill may add (0.02; 0.05 for sam and
                                  external-sam), every provider
masks, external-sam provider (flag, then environment variable):
  --sam-python EXE --sam-source DIR --sam-checkpoint FILE [--sam-config NAME] [--sam-pythonpath LIST] --sam-repository DIR
                                  [CRISP3DS_SAM_PYTHON, _SAM_SOURCE, _SAM_CHECKPOINT, _SAM_CONFIG, _SAM_PYTHONPATH,
                                  CRISP3DS_REPOSITORY: the checkout with scripts/turntable_mesh/segment.py]
  --sam-device mps|cpu|cuda (mps)   --[no-]sam-multimask --[no-]sam-preserve-holes --[no-]sam-automatic-cues (on)
masks, sam provider (--[no-]sam-preserve-holes and --[no-]sam-automatic-cues apply to it too):
  --sam-prompt-mask threshold|background (threshold)   coarse seed for either SAM provider; background uses RGB contrast
  --sam-model DIR                 model.json with the ONNX graphs, as tools/sam2_export_onnx.py writes them [CRISP3DS_SAM_MODEL];
                                  default: fetched on first use from huggingface.co/cstr/sam2.1-hiera-tiny-ONNX into the
                                  cache (CRISP3DS_CACHE_DIR, else the platform's) and checked against pinned SHA-256
  --sam-runtime FILE              ONNX Runtime's shared library (libonnxruntime.dylib, .so, onnxruntime.dll), or for a
                                  .gguf model CrispEmbed's libcrispembed-sam2 [CRISP3DS_SAM_RUNTIME, else ORT_DYLIB_PATH
                                  or CRISPEMBED_SAM2_LIB]
  --sam-accelerator cpu|coreml|gpu (cpu)   coreml for ONNX models, gpu for GGUF models
  --sam-candidates single|several (single)   SAM's own single mask, or the best-scored of its three proposals
cameras, every provider:
  --contrast-gamma G (0.5; 1 disables)   --clahe-clip C (2.0; 0 disables)   --clahe-grid N (8)
                                  the images features are detected in and that are undistorted into the scene
  --random-seed N (0)
  --open-turn                     the photos do not close a full turn (default: they do, and the closure gate applies)
  --capture turntable|orbit       turntable (default): one ring of photos, the ring gates apply; orbit: any camera path
                                  (robot arm, hand-held), only the camera audit applies; not with --cameras turntable
cameras, alicevision provider:
  --alicevision PATH              install prefix with bin/aliceVision_* (run directly), or a wrapper script called as
                                  `wrapper TOOL args` [CRISP3DS_ALICEVISION]
  --alicevision-library-path L    extra library directories for a prefix, separated like PATH [CRISP3DS_ALICEVISION_LIBRARY_PATH]
  --alicevision-env K=V           extra variable for the tools of a prefix; repeat [CRISP3DS_ALICEVISION_ENV: K=V,K=V]
  --alicevision-sensor-database FILE   cameraSensors.db [CRISP3DS_ALICEVISION_SENSOR_DB, else <prefix>/share/aliceVision/]
  --alicevision-memory-gib N (4)  --alicevision-initial-field-of-view DEG (45)  --alicevision-describer-types T (sift)
  --alicevision-describer-preset P (normal)  --alicevision-matching-method M (Exhaustive)
  --alicevision-sfm-option ARG    extra token for globalSfM; repeat per token
cameras, colmap provider:
  --colmap PATH                   the colmap executable, or a wrapper called as `wrapper COMMAND args` [CRISP3DS_COLMAP]
  --colmap-matching exhaustive|sequential|ring (exhaustive)   ring: every photo with its --colmap-overlap successors
                                  around the closed turn (matches_importer); sequential: the same without closing the turn
  --colmap-overlap N (10)         --colmap-masks on|off (on: no features outside the masks)
  --colmap-max-features N (8192)  --colmap-cli auto|3|4 (auto: ask the executable; 3: SiftExtraction.* names; 4: FeatureExtraction.*)
  --colmap-extractor-option ARG, --colmap-matcher-option ARG, --colmap-mapper-option ARG   extra tokens; repeat
cameras, turntable provider (our own solver for ordered turntable photos; docs/TURNTABLE-SOLVER.md):
  --turntable-features N (6000)   most features kept per photo
  --turntable-span N (4)          every photo is matched with this many successors
  --turntable-working-size N (2200)   features are found on photos scaled to this longest side when larger;
                                  the cameras are scaled back to the photos (0: the photo resolution)
  --turntable-rings N (1)         the photos are N complete turns of equal length one after the other (cameras at
                                  several heights): every turn is solved, then all are joined and adjusted together
cameras, markers provider (a printed mat under the object; docs/MARKER-MAT.md):
  --markers-mat FILE              description of the printed mat, as `crisp3ds-dense mat` writes it [CRISP3DS_MARKERS_MAT]
  --markers-minimum N (5)         a photo with fewer decoded markers gets no pose
  --markers-aspect auto|X         height over width of the print relative to the design (1); auto estimates it
tools:
  --python EXE                    only for .py wrappers [CRISP3DS_PYTHON]
machine:
  --threads N (2)   --minimum-free-gib N (10, checked before every step)   --keep-intermediates
  --stop-after masks              write the masks and stop; needs no camera provider
deadlines in seconds (the whole process group of an external step is stopped):
  --sam-timeout 1800  --features-timeout 1800  --matching-timeout 1800  --sfm-timeout 900  --small-step-timeout 600
quality gates (a failed gate means exit code 2 and no scene):
  --minimum-registered-fraction 0.8   --minimum-observations-per-view 20   --maximum-view-reprojection-p95 4.0
  --minimum-positive-depth-fraction 0.999   --maximum-radius-spread-percent 5   --maximum-out-of-plane-percent 5
  --maximum-angular-gap-deg 30   --maximum-reversed-steps 0   --maximum-optical-axis-miss-percent 25
  --duplicate-step-deg 0.5 (warning only)
  --maximum-closing-step-ratio 2.5   unless --open-turn: what is left from the last photo to the first, over the median step";

/// Names of the reference command line that live on under a provider's namespace.
const ALIASES: &[(&str, &str)] = &[
    ("dark-threshold", "threshold-level"),
    ("envelope", "threshold-envelope"),
    ("device", "sam-device"),
    ("repository", "sam-repository"),
    ("sensor-database", "alicevision-sensor-database"),
    ("initial-field-of-view", "alicevision-initial-field-of-view"),
    ("describer-types", "alicevision-describer-types"),
    ("describer-preset", "alicevision-describer-preset"),
    ("matching-method", "alicevision-matching-method"),
    ("sfm-option", "alicevision-sfm-option"),
];

fn absolute(path: &str) -> PathBuf {
    crate::photos::fs::absolute(path)
}

fn split_paths(text: &str) -> Vec<String> {
    std::env::split_paths(text).map(|p| p.to_string_lossy().to_string()).filter(|p| !p.is_empty()).collect()
}

fn pair(text: &str) -> anyhow::Result<(String, String)> {
    let (name, value) = text.split_once('=').ok_or_else(|| anyhow!("expected NAME=VALUE, found {text:?}"))?;
    if name.is_empty() {
        bail!("expected NAME=VALUE, found {text:?}");
    }
    Ok((name.to_string(), value.to_string()))
}

/// Parses the command line. `environment` looks up a variable (injected for tests).
pub fn resolve(arguments: &[String], environment: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Options> {
    let mut values: HashMap<String, Vec<String>> = HashMap::new();
    let mut index = 0;
    while index < arguments.len() {
        let word = &arguments[index];
        let Some(flag) = word.strip_prefix("--") else { bail!("unexpected argument {word:?}") };
        let (given, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (flag, None),
        };
        let name = ALIASES.iter().find(|(old, _)| *old == given).map(|(_, new)| *new).unwrap_or(given);
        let switch = option_table::find(name.strip_prefix("no-").unwrap_or(name)).filter(|option| option.kind == "switch");
        let valued = option_table::find(name).filter(|option| option.kind != "switch");
        if let Some(switch) = switch {
            if inline.is_some() {
                bail!("--{given} takes no value");
            }
            values.insert(switch.flag.to_string(), vec![(!name.starts_with("no-")).to_string()]);
        } else if let Some(option) = valued {
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    arguments.get(index).cloned().ok_or_else(|| anyhow!("--{given} needs a value"))?
                }
            };
            let slot = values.entry(name.to_string()).or_default();
            if !option.repeated {
                slot.clear();
            }
            slot.push(value);
        } else {
            bail!("unknown option --{given}");
        }
        index += 1;
    }
    let text = |name: &str| values.get(name).and_then(|v| v.last()).cloned();
    let many = |name: &str| values.get(name).cloned().unwrap_or_default();
    let pick =
        |name: &str, variable: &str| text(name).filter(|v| !v.is_empty()).or_else(|| environment(variable).filter(|v| !v.is_empty()));
    let number = |name: &str| -> anyhow::Result<f64> {
        let value = text(name).unwrap_or_else(|| option_table::default(name).to_string());
        value.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| anyhow!("--{name} needs a number, got {value:?}"))
    };
    let integer = |name: &str| -> anyhow::Result<i64> {
        let value = text(name).unwrap_or_else(|| option_table::default(name).to_string());
        value.trim().parse::<i64>().map_err(|_| anyhow!("--{name} needs an integer, got {value:?}"))
    };
    let switch = |name: &str| text(name).unwrap_or_else(|| option_table::default(name).to_string()) == "true";
    let word = |name: &str| text(name).unwrap_or_else(|| option_table::default(name).to_string());
    let required = |name: &str| text(name).ok_or_else(|| anyhow!("--{name} is required"));
    let seconds = |name: &str| -> anyhow::Result<u64> {
        let value = integer(name)?;
        if value < 1 {
            bail!("--{name} must be at least one second");
        }
        Ok(value as u64)
    };
    // `provider` or `provider:ARGUMENT`.
    let choice = |name: &str| -> (String, Option<String>) {
        let variable = option_table::find(name).and_then(|option| option.variable).unwrap_or_default();
        let value = pick(name, variable).unwrap_or_else(|| option_table::default(name).to_string());
        match value.split_once(':') {
            Some((provider, argument)) => (provider.to_string(), Some(argument.to_string())),
            None => (value, None),
        }
    };

    let photos = absolute(&required("photos")?);
    let output = absolute(&required("output")?);
    if !photos.stored_dir() {
        bail!("--photos must be a directory");
    }
    let photo_count = list_photos(&photos)?.len();
    // Up to 96 photos per turn (several turns with --turntable-rings).
    let turns = integer("turntable-rings")?.clamp(1, 16) as usize;
    if !(3..=96 * turns).contains(&photo_count) {
        let most = 96 * turns;
        bail!("found {photo_count} photos ({}); the camera audit supports 3 to {most}", PHOTO_SUFFIXES.join(", "));
    }
    let stop_after_masks = match text("stop-after").as_deref() {
        None => false,
        Some("masks") => true,
        Some(other) => bail!("--stop-after {other}: only 'masks' is offered"),
    };
    let masks = match choice("masks") {
        (name, None) if name == "threshold" => MaskChoice::Threshold,
        (name, None) if name == "background" => MaskChoice::Background,
        (name, None) if name == "external-sam" => MaskChoice::ExternalSam,
        (name, None) if name == "sam" => MaskChoice::Sam,
        (name, Some(folder)) if name == "import" && !folder.is_empty() => MaskChoice::Import(absolute(&folder)),
        (name, _) if name == "import" => bail!("--masks import needs the folder: import:DIR"),
        (name, _) => bail!("--masks {name}: expected threshold, import:DIR, external-sam or sam (see --list-providers)"),
    };
    if let MaskChoice::Import(folder) = &masks {
        if !folder.stored_dir() {
            bail!("--masks import:{} is not a directory", folder.display());
        }
    }
    let cameras = match choice("cameras") {
        (name, None) if name == "alicevision" => CameraChoice::AliceVision,
        (name, None) if name == "colmap" => CameraChoice::Colmap,
        (name, None) if name == "markers" => CameraChoice::Markers,
        (name, None) if name == "turntable" => CameraChoice::Turntable,
        (name, Some(path)) if name == "import" && !path.is_empty() => CameraChoice::Import(absolute(&path)),
        (name, _) if name == "import" => bail!("--cameras import needs the solution: import:PATH (.sfm file or COLMAP model directory)"),
        (name, _) => bail!("--cameras {name}: expected alicevision, colmap, turntable, markers or import:PATH (see --list-providers)"),
    };
    if let CameraChoice::Import(path) = &cameras {
        if !path.stored() {
            bail!("--cameras import:{} does not exist", path.display());
        }
    }
    let calibration = text("calibration").map(|p| absolute(&p));
    if calibration.is_none() && !stop_after_masks && !matches!(cameras, CameraChoice::Import(_)) {
        bail!("--calibration is required");
    }
    let library_path = pick("alicevision-library-path", "CRISP3DS_ALICEVISION_LIBRARY_PATH").map(|v| split_paths(&v)).unwrap_or_default();
    let mut tool_environment = Vec::new();
    match values.get("alicevision-env") {
        Some(pairs) => {
            for item in pairs {
                tool_environment.push(pair(item)?);
            }
        }
        None => {
            for item in environment("CRISP3DS_ALICEVISION_ENV").unwrap_or_default().split(',').filter(|v| !v.is_empty()) {
                tool_environment.push(pair(item)?);
            }
        }
    }
    let tools = pick("alicevision", "CRISP3DS_ALICEVISION").map(|location| Tools {
        location: absolute(&location),
        library_path: library_path.into_iter().map(PathBuf::from).collect(),
        environment: tool_environment,
    });
    let sam = SamOptions {
        background_prompt: match word("sam-prompt-mask").as_str() {
            "threshold" => false,
            "background" => true,
            value => bail!("invalid --sam-prompt-mask {value:?}; choose threshold or background"),
        },
        python: pick("sam-python", "CRISP3DS_SAM_PYTHON"),
        source: pick("sam-source", "CRISP3DS_SAM_SOURCE").map(|p| absolute(&p)),
        checkpoint: pick("sam-checkpoint", "CRISP3DS_SAM_CHECKPOINT").map(|p| absolute(&p)),
        config: pick("sam-config", "CRISP3DS_SAM_CONFIG"),
        pythonpath: pick("sam-pythonpath", "CRISP3DS_SAM_PYTHONPATH").map(|v| split_paths(&v)).unwrap_or_default(),
        repository: pick("sam-repository", "CRISP3DS_REPOSITORY").map(|p| absolute(&p)),
        device: word("sam-device"),
        multimask: switch("sam-multimask"),
        preserve_holes: switch("sam-preserve-holes"),
        automatic_cues: switch("sam-automatic-cues"),
        model: pick("sam-model", "CRISP3DS_SAM_MODEL").map(|p| absolute(&p)),
        runtime: pick("sam-runtime", "CRISP3DS_SAM_RUNTIME").map(|p| absolute(&p)),
        accelerator: word("sam-accelerator"),
        several_candidates: word("sam-candidates") == "several",
    };
    if !["mps", "cpu", "cuda"].contains(&sam.device.as_str()) {
        bail!("--sam-device {}: expected mps, cpu or cuda", sam.device);
    }
    let threads = integer("threads")?;
    let clahe_grid = integer("clahe-grid")?;
    if !(1..=256).contains(&threads) || !(1..=64).contains(&clahe_grid) {
        bail!("--threads must be 1..256 and --clahe-grid 1..64");
    }
    let turntable = TurntableOptions {
        features: integer("turntable-features")?.clamp(100, 100_000) as usize,
        span: integer("turntable-span")?.clamp(1, 32) as usize,
        matches: text("turntable-matches").map(|p| absolute(&p)),
        surface: word("turntable-region") == "surface",
        working_size: integer("turntable-working-size")?.clamp(0, 100_000) as usize,
        rings: integer("turntable-rings")?.clamp(1, 16) as usize,
    };
    let markers = MarkersOptions {
        mat: pick("markers-mat", "CRISP3DS_MARKERS_MAT").map(|p| absolute(&p)),
        minimum_per_photo: integer("markers-minimum")?.clamp(1, 1000) as usize,
        aspect: match text("markers-aspect").as_deref() {
            None | Some("auto") => None,
            Some(value) => Some(
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|v| (0.9..=1.1).contains(v))
                    .ok_or_else(|| anyhow!("--markers-aspect needs auto or a number near 1"))?,
            ),
        },
        aspect_auto: text("markers-aspect").as_deref() == Some("auto"),
    };
    let colmap = ColmapOptions {
        location: pick("colmap", "CRISP3DS_COLMAP").map(|p| absolute(&p)),
        cli: match word("colmap-cli").as_str() {
            "auto" => 0,
            value => value.parse::<i64>().map_err(|_| anyhow!("--colmap-cli needs auto, 3 or 4, got {value:?}"))?,
        },
        matching: word("colmap-matching"),
        overlap: integer("colmap-overlap")?,
        use_masks: match text("colmap-masks").as_deref() {
            None | Some("on") => true,
            Some("off") => false,
            Some(other) => bail!("--colmap-masks {other}: expected on or off"),
        },
        max_features: integer("colmap-max-features")?,
        extractor_option: many("colmap-extractor-option"),
        matcher_option: many("colmap-matcher-option"),
        mapper_option: many("colmap-mapper-option"),
    };
    if !["exhaustive", "sequential", "ring"].contains(&colmap.matching.as_str()) || ![0, 3, 4].contains(&colmap.cli) || colmap.overlap < 1 {
        bail!("--colmap-matching must be exhaustive, sequential or ring, --colmap-cli auto, 3 or 4, --colmap-overlap at least 1");
    }
    let ring_capture = match word("capture").as_str() {
        "turntable" => true,
        "orbit" => false,
        other => bail!("--capture {other}: expected turntable or orbit"),
    };
    let options = Options {
        photos,
        photo_count,
        calibration,
        output,
        events: text("events").map(|p| absolute(&p)),
        masks: masks.clone(),
        cameras,
        python: pick("python", "CRISP3DS_PYTHON"),
        threads: threads as usize,
        minimum_free_gib: number("minimum-free-gib")?,
        keep_intermediates: switch("keep-intermediates"),
        stop_after_masks,
        envelope: word("threshold-envelope"),
        // As masks the Otsu level did better than a fixed one on the test objects; as SAM prompts the reference level stays.
        dark_threshold: Threshold::parse(
            &text("threshold-level").unwrap_or_else(|| if masks == MaskChoice::Threshold { "otsu" } else { "70" }.into()),
        )?,
        threshold_shadow: if masks == MaskChoice::Threshold { number("threshold-shadow")?.clamp(0.0, 1.0) } else { 0.0 },
        // SAM 2.1 leaves rows of small dark holes inside the object on some views (window-grid pattern, up to about
        // 2.5 % of the mask on the test objects); they are object pixels, and the cleanup fills them.
        hole_cleanup_budget: super::cleanup::validate_budget(match (text("hole-cleanup-budget"), &masks) {
            (None, MaskChoice::Sam | MaskChoice::ExternalSam) => SAM_HOLE_CLEANUP_BUDGET,
            _ => number("hole-cleanup-budget")?,
        })?,
        sam,
        contrast_gamma: number("contrast-gamma")?,
        clahe_clip: number("clahe-clip")?,
        clahe_grid: clahe_grid as usize,
        alicevision: AliceVisionOptions {
            tools,
            sensor_database: pick("alicevision-sensor-database", "CRISP3DS_ALICEVISION_SENSOR_DB").map(PathBuf::from),
            memory_gib: number("alicevision-memory-gib")?,
            initial_field_of_view: number("alicevision-initial-field-of-view")?,
            describer_types: word("alicevision-describer-types"),
            describer_preset: word("alicevision-describer-preset"),
            matching_method: word("alicevision-matching-method"),
            sfm_option: many("alicevision-sfm-option"),
        },
        colmap,
        markers,
        turntable,
        open_turn: switch("open-turn"),
        random_seed: integer("random-seed")?,
        timeouts: Timeouts {
            small: seconds("small-step-timeout")?,
            sam: seconds("sam-timeout")?,
            features: seconds("features-timeout")?,
            matching: seconds("matching-timeout")?,
            sfm: seconds("sfm-timeout")?,
        },
        gates: Gates {
            minimum_registered_fraction: number("minimum-registered-fraction")?,
            minimum_observations_per_view: integer("minimum-observations-per-view")?,
            maximum_view_reprojection_p95_pixels: number("maximum-view-reprojection-p95")?,
            minimum_positive_depth_fraction: number("minimum-positive-depth-fraction")?,
            maximum_radius_spread_percent: number("maximum-radius-spread-percent")?,
            maximum_out_of_plane_percent: number("maximum-out-of-plane-percent")?,
            maximum_angular_gap_deg: number("maximum-angular-gap-deg")?,
            maximum_reversed_steps: integer("maximum-reversed-steps")?,
            maximum_optical_axis_miss_percent: number("maximum-optical-axis-miss-percent")?,
            duplicate_step_deg: number("duplicate-step-deg")?,
            full_turn: !switch("open-turn"),
            ring: ring_capture,
            maximum_closing_step_ratio: number("maximum-closing-step-ratio")?,
        },
    };
    if !options.gates.ring && options.cameras == CameraChoice::Turntable {
        bail!("--capture orbit: the turntable camera provider needs photos taken on a turntable; choose --cameras colmap (or alicevision, markers, import) for other camera paths");
    }
    super::coarse::validate_envelope(&options.envelope)?;
    if let Some(calibration) = &options.calibration {
        super::calibration::load_calibration(calibration)?;
    }
    Ok(options)
}

impl Options {
    /// The record written to `frontend-config.json` and into the report.
    pub fn to_json(&self) -> Value {
        let argument = |path: Option<&PathBuf>| path.map(|p| p.to_string_lossy().to_string());
        let mut record = json!({
            "schema": SCHEMA, "engine": concat!("crisp3ds-dense ", env!("CARGO_PKG_VERSION")),
            "photos": self.photos, "photo_count": self.photo_count, "calibration": self.calibration, "output": self.output,
            "masks_provider": self.masks.name(),
            "masks_import": argument(if let MaskChoice::Import(folder) = &self.masks { Some(folder) } else { None }),
            "cameras_provider": self.cameras.name(),
            "cameras_import": argument(if let CameraChoice::Import(path) = &self.cameras { Some(path) } else { None }),
            "python": self.python, "threads": self.threads, "minimum_free_gib": self.minimum_free_gib,
            "envelope": self.envelope, "dark_threshold": self.dark_threshold.to_json(), "hole_cleanup_budget": self.hole_cleanup_budget,
            "threshold_shadow": self.threshold_shadow,
            "contrast_gamma": self.contrast_gamma, "clahe_clip": self.clahe_clip, "clahe_grid": self.clahe_grid,
            "random_seed": self.random_seed,
            "gates": self.gates.to_json(), "keep_intermediates": self.keep_intermediates, "stop_after_masks": self.stop_after_masks,
        });
        // In parts: one literal of this size exceeds the macro recursion limit.
        let (sam, av, colmap) = (&self.sam, &self.alicevision, &self.colmap);
        let more = json!({
            "sam": {"python": sam.python, "source": sam.source, "checkpoint": sam.checkpoint, "config": sam.config,
                    "pythonpath": sam.pythonpath, "repository": sam.repository, "device": sam.device, "multimask": sam.multimask,
                    "preserve_holes": sam.preserve_holes, "automatic_cues": sam.automatic_cues,
                    "prompt_mask": if sam.background_prompt { "background" } else { "threshold" },
                    "model": sam.model, "runtime": sam.runtime, "accelerator": sam.accelerator,
                    "candidates": if sam.several_candidates { "several" } else { "single" }},
            "alicevision": {
                "location": av.tools.as_ref().map(|t| t.location.clone()),
                "library_path": av.tools.as_ref().map(|t| t.library_path.clone()),
                "environment": av.tools.as_ref().map(|t| t.environment.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>()),
                "sensor_database": av.sensor_database, "memory_gib": av.memory_gib, "initial_field_of_view": av.initial_field_of_view,
                "describer_types": av.describer_types, "describer_preset": av.describer_preset, "matching_method": av.matching_method,
                "sfm_option": av.sfm_option},
            "turntable": {"features": self.turntable.features, "span": self.turntable.span, "matches": self.turntable.matches, "surface": self.turntable.surface, "working_size": self.turntable.working_size, "rings": self.turntable.rings},
            "open_turn": self.open_turn,
            "markers": {"mat": self.markers.mat, "minimum_per_photo": self.markers.minimum_per_photo, "aspect": self.markers.aspect,
                        "aspect_auto": self.markers.aspect_auto},
            "colmap": {"location": colmap.location, "cli": colmap.cli, "matching": colmap.matching, "overlap": colmap.overlap,
                       "masks": colmap.use_masks, "max_features": colmap.max_features, "extractor_option": colmap.extractor_option,
                       "matcher_option": colmap.matcher_option, "mapper_option": colmap.mapper_option},
            "timeouts": {"small": self.timeouts.small, "sam": self.timeouts.sam, "features": self.timeouts.features,
                         "matching": self.timeouts.matching, "sfm": self.timeouts.sfm},
        });
        if let (Some(record), Value::Object(more)) = (record.as_object_mut(), more) {
            record.extend(more);
        }
        record
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    pub fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("crisp3ds-options-{}-{name}", std::process::id()));
        let _ = crate::photos::fs::remove_dir_all(&folder);
        crate::photos::fs::create_dir_all(folder.join("photos")).unwrap();
        for n in [1, 2, 10] {
            crate::photos::fs::write(folder.join(format!("photos/thing_{n}_rgb.png")), b"").unwrap();
        }
        crate::photos::fs::write(folder.join("av.py"), b"").unwrap();
        folder
    }

    pub fn example() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/turntable_mesh/calibrations/3dlf-pro.json").to_string()
    }

    pub fn arguments(folder: &Path, more: &[&str]) -> Vec<String> {
        let mut words: Vec<String> = [
            "--photos",
            &folder.join("photos").to_string_lossy(),
            "--calibration",
            &example(),
            "--output",
            &folder.join("out").to_string_lossy(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        words.extend(more.iter().map(|s| s.to_string()));
        words
    }

    pub fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_and_provider_choices() {
        let folder = scratch("choices");
        let options = resolve(&arguments(&folder, &[]), &none).unwrap();
        assert_eq!((options.photo_count, options.dark_threshold, options.envelope.as_str()), (3, Threshold::Otsu, "auto"));
        assert_eq!((&options.masks, &options.cameras, options.threshold_shadow), (&MaskChoice::Threshold, &CameraChoice::Turntable, 0.25));
        assert!(options.gates.ring);
        let orbit = resolve(&arguments(&folder, &["--capture", "orbit", "--cameras", "colmap"]), &none).unwrap();
        assert!(!orbit.gates.ring && orbit.to_json()["gates"]["ring_gates"] == false);
        let refused = resolve(&arguments(&folder, &["--capture", "orbit"]), &none).unwrap_err().to_string();
        assert!(refused.contains("turntable camera provider"), "{refused}");
        assert!(resolve(&arguments(&folder, &["--capture", "spiral"]), &none).is_err());
        let prompted = resolve(&arguments(&folder, &["--masks", "external-sam"]), &none).unwrap();
        assert_eq!((prompted.dark_threshold, prompted.threshold_shadow), (Threshold::Level(70), 0.0));
        // The SAM providers get a larger hole-cleanup budget unless one is given; the others keep 0.02.
        let native = resolve(&arguments(&folder, &["--masks", "sam"]), &none).unwrap();
        assert_eq!((options.hole_cleanup_budget, prompted.hole_cleanup_budget, native.hole_cleanup_budget), (0.02, 0.05, 0.05));
        assert!(!native.sam.several_candidates);
        assert_eq!(native.to_json()["sam"]["prompt_mask"], "threshold");
        let background = resolve(&arguments(&folder, &["--masks", "sam", "--sam-prompt-mask", "background"]), &none).unwrap();
        assert!(background.sam.background_prompt);
        assert_eq!(background.to_json()["sam"]["prompt_mask"], "background");
        assert!(resolve(&arguments(&folder, &["--masks", "sam", "--sam-prompt-mask", "unknown"]), &none).is_err());
        let given =
            resolve(&arguments(&folder, &["--masks", "sam", "--hole-cleanup-budget", "0.01", "--sam-candidates", "several"]), &none);
        assert_eq!(given.as_ref().map(|o| (o.hole_cleanup_budget, o.sam.several_candidates)).unwrap(), (0.01, true));
        assert_eq!((options.colmap.matching.as_str(), options.colmap.use_masks, options.colmap.cli), ("exhaustive", true, 0));
        let record = options.to_json();
        assert_eq!(record["schema"], SCHEMA);
        assert_eq!((record["masks_provider"].as_str(), record["cameras_provider"].as_str()), (Some("threshold"), Some("turntable")));
        assert_eq!(
            (record["dark_threshold"].as_str(), record["gates"]["minimum_positive_depth_fraction"].as_f64()),
            (Some("otsu"), Some(0.999))
        );
        assert_eq!(record["timeouts"]["sfm"], 900);
        crate::photos::fs::create_dir_all(folder.join("given")).unwrap();
        let given = folder.join("given").to_string_lossy().to_string();
        let variables = |name: &str| match name {
            "CRISP3DS_CAMERAS" => Some("colmap".to_string()),
            "CRISP3DS_COLMAP" => Some("/opt/colmap".to_string()),
            _ => None,
        };
        let more = ["--masks", &format!("import:{given}"), "--colmap-matching", "ring", "--colmap-masks=off"];
        let options = resolve(&arguments(&folder, &more), &variables).unwrap();
        assert_eq!((&options.masks, &options.cameras), (&MaskChoice::Import(PathBuf::from(&given)), &CameraChoice::Colmap));
        assert_eq!(options.colmap.location, Some(std::path::absolute("/opt/colmap").unwrap()));
        assert_eq!((options.colmap.matching.as_str(), options.colmap.use_masks), ("ring", false));
        let options = resolve(&arguments(&folder, &["--cameras", &format!("import:{given}"), "--masks", "external-sam"]), &none).unwrap();
        assert_eq!((&options.masks, &options.cameras), (&MaskChoice::ExternalSam, &CameraChoice::Import(PathBuf::from(&given))));
        crate::photos::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn reference_flag_names_remain_as_aliases() {
        let folder = scratch("aliases");
        let more = [
            "--dark-threshold",
            "otsu",
            "--envelope",
            "0.1,0.1,0.9,0.9",
            "--device",
            "cpu",
            "--threads=3",
            "--no-sam-multimask",
            "--hole-cleanup-budget",
            "0.05",
            "--matching-method",
            "Sequential",
            "--random-seed",
            "7",
            "--sfm-option",
            "--a",
            "--sfm-option=b",
            "--keep-intermediates",
            "--describer-preset",
            "high",
            "--initial-field-of-view",
            "50",
        ];
        let options = resolve(&arguments(&folder, &more), &none).unwrap();
        assert_eq!(
            (options.dark_threshold, options.envelope.as_str(), options.hole_cleanup_budget),
            (Threshold::Otsu, "0.1,0.1,0.9,0.9", 0.05)
        );
        assert_eq!((options.sam.device.as_str(), options.sam.multimask, options.sam.preserve_holes), ("cpu", false, true));
        assert_eq!((options.threads, options.random_seed, options.keep_intermediates), (3, 7, true));
        let av = &options.alicevision;
        assert_eq!((av.matching_method.as_str(), av.describer_preset.as_str(), av.initial_field_of_view), ("Sequential", "high", 50.0));
        assert_eq!(av.sfm_option, ["--a", "b"]);
        crate::photos::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn invalid_requests_are_refused_before_anything_runs() {
        let folder = scratch("invalid");
        let with = |more: &[&str]| resolve(&arguments(&folder, more), &none);
        assert!(with(&[]).is_ok());
        for (more, expected) in [
            (&["--dark-threshold", "300"][..], "--dark-threshold"),
            (&["--envelope", "1,2,3"][..], "envelope"),
            (&["--hole-cleanup-budget", "0.2"][..], "finite"),
            (&["--masks", "magic"][..], "expected threshold, import:DIR, external-sam or sam"),
            (&["--masks", "import"][..], "needs the folder"),
            (&["--masks", "import:/no/such/masks"][..], "is not a directory"),
            (&["--cameras", "meshroom"][..], "expected alicevision, colmap, turntable, markers or import:PATH"),
            (&["--cameras", "import:/no/such.sfm"][..], "does not exist"),
            (&["--colmap-matching", "vocab"][..], "--colmap-matching"),
            (&["--threads", "two"][..], "needs an integer"),
            (&["--bogus", "1"][..], "unknown option --bogus"),
            (&["--stop-after", "cameras"][..], "only 'masks'"),
            (&["--sfm-timeout", "0"][..], "at least one second"),
        ] {
            let error = with(more).unwrap_err().to_string();
            assert!(error.contains(expected), "{more:?}: {error}");
        }
        crate::photos::fs::write(folder.join("bad.json"), "{\"fx\": 1}").unwrap();
        let bad = folder.join("bad.json").to_string_lossy().to_string();
        assert!(with(&["--calibration", &bad]).unwrap_err().to_string().contains("unknown calibration format"));
        crate::photos::fs::create_dir_all(folder.join("two")).unwrap();
        for name in ["a.png", "b.png", "notes.txt"] {
            crate::photos::fs::write(folder.join("two").join(name), b"").unwrap();
        }
        assert!(with(&["--photos", &folder.join("two").to_string_lossy()]).unwrap_err().to_string().contains("found 2 photos"));
        let no_lens: Vec<String> = arguments(&folder, &[]).into_iter().filter(|w| w != "--calibration" && *w != example()).collect();
        assert_eq!(resolve(&no_lens, &none).unwrap_err().to_string(), "--calibration is required");
        crate::photos::fs::remove_dir_all(&folder).unwrap();
    }
}
