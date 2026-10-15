//! The `sam` mask provider: SAM 2.1 in this process. Reads the staged photos
//! and their coarse masks, writes `work/sam-native/masks/<photo>.png` (and
//! `raw-masks/`, `result.json`), the same files the reference script leaves
//! under `work/sam/`.

use crate::photos::fs::Stored as _;
use std::path::{Path, PathBuf};
use web_time::Instant;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use super::super::masks::MaskProvider;
use super::super::options::Options;
use super::super::providers::{Provider, MASKS_SAM};
use super::super::run::Run;
use super::super::staging::{file_name, open_binary_mask, open_photo, save_mask};
use super::super::util;
use super::backend::{self, BackendOptions, ModelInfo};
use super::prompts::{frozen_prompts, support, Prompt};
use super::{segment, Settings};
use crate::photos::MOST_PHOTOS;

pub const SCHEMA: &str = "crispmesh_sam_native_v1";

/// The default model in the cache, fetched on first use, in builds that can run it.
fn default_model() -> Option<PathBuf> {
    #[cfg(feature = "sam-onnx")]
    return super::fetch::default_model_directory();
    #[cfg(not(feature = "sam-onnx"))]
    None
}

/// Whether a model directory (or the default model, fetched on first use) and the runtime library allow a run;
/// what will run if so. Looks at files only: no network.
pub fn readiness(model: Option<&Path>, runtime: Option<&Path>) -> Result<String, String> {
    // The library a format's backend opens: ONNX Runtime for onnx, libcrispembed-sam2 for gguf.
    let runtime_ready = |format: &str| {
        let (variable, what) = if format == "gguf" {
            ("CRISPEMBED_SAM2_LIB", "libcrispembed-sam2")
        } else {
            ("ORT_DYLIB_PATH", "ONNX Runtime's shared library")
        };
        match runtime.map(Path::to_path_buf).or_else(|| std::env::var_os(variable).map(PathBuf::from)) {
            None => Err(format!("{what} is not given (--sam-runtime FILE or {variable})")),
            Some(library) if !library.stored_file() => Err(format!("{}: {what} not found", library.display())),
            Some(_) => Ok(()),
        }
    };
    let Some(directory) = model else {
        if let Some(reason) = backend::unavailable("onnx") {
            return Err(format!("no model directory (--sam-model DIR or CRISPMESH_SAM_MODEL), and {reason}"));
        }
        let Some(cache) = default_model() else {
            return Err("no model directory (--sam-model DIR) and no cache directory to fetch the default model into".to_string());
        };
        runtime_ready("onnx")?;
        return Ok(format!("sam2.1_hiera_tiny (onnx), from {} (fetched on first use)", cache.display()));
    };
    let model = ModelInfo::read(directory).map_err(|error| format!("{error:#}"))?;
    if let Some(reason) = backend::unavailable(&model.format) {
        return Err(reason);
    }
    runtime_ready(&model.format)?;
    Ok(format!("{} ({})", model.name, model.format))
}

/// Compares the files of a model directory with the SHA-256 its `model.json` records.
pub fn verify_files(model: &ModelInfo) -> anyhow::Result<()> {
    if model.files.is_null() {
        return Ok(()); // a GGUF given as a file: no record to compare with
    }
    for file in [&model.encoder, &model.decoder] {
        let name = file_name(file);
        let Some(expected) = model.files[&name]["sha256"].as_str() else { bail!("model.json records no SHA-256 for {name}") };
        let found = util::sha256_file(file)?;
        if found != expected {
            bail!("{}: SHA-256 {found} differs from the {expected} model.json records (incomplete download?)", file.display());
        }
    }
    Ok(())
}

fn prompt_json(prompt: &Prompt) -> Value {
    json!({"point_xy": prompt.point, "points_xy": prompt.points, "point_labels": prompt.labels, "box_xyxy": prompt.box_xyxy})
}

/// Segments every photo of `photos` whose coarse mask is `<coarse>/<photo name>.png`. The photos are
/// `turns` equal runs of one turntable turn each (cameras at different heights); the common prompt box
/// is that of each turn. `watch(done)` is called after every photo and may stop the run.
#[allow(clippy::too_many_arguments)]
pub fn run(
    photos: &Path,
    coarse: &Path,
    output: &Path,
    model: &ModelInfo,
    backend_options: &BackendOptions,
    settings: Settings,
    automatic_cues: bool,
    turns: usize,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    if output.stored() {
        bail!("output exists: {}", output.display());
    }
    let started = Instant::now();
    let mut paths = Vec::new();
    for path in crate::photos::fs::list(photos).with_context(|| photos.display().to_string())? {
        let suffix = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["png", "jpg", "jpeg"].contains(&suffix.as_str()) {
            paths.push(path);
        }
    }
    paths.sort();
    if !(1..=MOST_PHOTOS).contains(&paths.len()) {
        bail!("segmentation input must contain 1..{MOST_PHOTOS} photos");
    }
    let turns = turns.max(1);
    if !paths.len().is_multiple_of(turns) {
        bail!("{} photos do not make {turns} turns of equal length", paths.len());
    }
    // Prompts need every coarse mask first: the box is common to all photos.
    let threads = backend_options.threads.max(1);
    let supports = util::parallel(
        paths.len(),
        threads,
        |index| {
            let path = coarse.join(format!("{}.png", file_name(&paths[index])));
            let mask = open_binary_mask(&path)?;
            Ok((support(&mask, automatic_cues).map_err(|e| anyhow!("{}: {e}", path.display()))?, mask.width, mask.height))
        },
        &mut |_| watch(0),
    )?;
    let (width, height) = (supports[0].1, supports[0].2);
    if supports.iter().any(|s| (s.1, s.2) != (width, height)) {
        bail!("common prompt box requires equal image dimensions");
    }
    let supports: Vec<_> = supports.into_iter().map(|s| s.0).collect();
    let prompts: Vec<Prompt> =
        supports.chunks(supports.len() / turns).flat_map(|turn| frozen_prompts(turn, width, height, automatic_cues)).collect();
    verify_files(model)?;
    let loading = Instant::now();
    let mut network = backend::open(model, backend_options)?;
    let load_seconds = loading.elapsed().as_secs_f64();
    for folder in ["masks", "raw-masks"] {
        crate::photos::fs::create_dir_all(output.join(folder))?;
    }
    let mut rows = Vec::new();
    for (index, (path, prompt)) in paths.iter().zip(&prompts).enumerate() {
        let tick = Instant::now();
        let name = file_name(path);
        let photo = open_photo(path)?;
        if (photo.width(), photo.height()) != (width, height) {
            bail!("{name}: the photo and its coarse mask differ in size");
        }
        let decoded = tick.elapsed().as_secs_f64();
        let result =
            segment(network.as_mut(), model, photo.rgb.as_raw(), width, height, prompt, settings).map_err(|e| anyhow!("{name}: {e}"))?;
        let writing = Instant::now();
        save_mask(&output.join("masks").join(format!("{name}.png")), &result.clean)?;
        save_mask(&output.join("raw-masks").join(format!("{name}.png")), &result.raw)?;
        let mut row = result.metrics;
        row["name"] = json!(name);
        row["prompt"] = prompt_json(prompt);
        row["seconds"] = json!(tick.elapsed().as_secs_f64());
        row["seconds_decode"] = json!(decoded);
        row["seconds_image"] = json!(result.seconds[0]);
        row["seconds_prompts"] = json!(result.seconds[1]);
        row["seconds_write"] = json!(writing.elapsed().as_secs_f64());
        rows.push(row);
        watch(index + 1)?;
    }
    let mean = |key: &str| rows.iter().filter_map(|row| row[key].as_f64()).sum::<f64>() / rows.len() as f64;
    let report = json!({
        "schema": SCHEMA, "status": "complete",
        "backend": network.description(),
        "model": {"name": model.name, "format": model.format, "license": model.license, "directory": model.directory, "files": model.files},
        "configuration": {
            "multimask_output": settings.multimask, "preserve_holes": settings.preserve_holes, "automatic_cues": automatic_cues,
            "accelerator": backend_options.accelerator, "threads": threads,
        },
        "photo_size": [width, height],
        "model_load_seconds": load_seconds,
        "seconds_per_photo": {"all": mean("seconds"), "decode": mean("seconds_decode"), "image": mean("seconds_image"),
                              "prompts": mean("seconds_prompts"), "write": mean("seconds_write")},
        "seconds": started.elapsed().as_secs_f64(),
        "rows": rows,
    });
    util::write_json(&output.join("result.json"), &report, 1)?;
    Ok(report)
}

pub struct NativeSam;

fn backend_options(options: &Options) -> BackendOptions {
    BackendOptions { threads: options.threads, accelerator: options.sam.accelerator.clone(), runtime: options.sam.runtime.clone() }
}

impl MaskProvider for NativeSam {
    fn info(&self) -> &'static Provider {
        &MASKS_SAM
    }

    fn check(&self, options: &Options) -> anyhow::Result<()> {
        readiness(options.sam.model.as_deref(), options.sam.runtime.as_deref())
            .map(|_| ())
            .map_err(|reason| anyhow!("--masks sam: {reason} (or choose --masks threshold, which needs none)"))
    }

    fn segment(&self, run: &mut Run, _photo_map: &Value, low: f64, high: f64) -> anyhow::Result<PathBuf> {
        let options = run.options;
        let work = run.out.join("work");
        let target = work.join("sam-native");
        let (directory, low) = match options.sam.model.clone() {
            Some(directory) => (directory, low),
            None => (fetched(run, low, high)?, low + 0.25 * (high - low)),
        };
        let model = ModelInfo::read(&directory)?;
        let settings = Settings { multimask: options.sam.several_candidates, preserve_holes: options.sam.preserve_holes };
        let report =
            run.internal("masks", "sam", options.timeouts.sam, low, high, "Segmenting with SAM 2.1", options.photo_count, |watch| {
                let turns = if options.cameras == crate::photos::options::CameraChoice::Turntable { options.turntable.rings } else { 1 };
                run_into(&work, &target, &model, &backend_options(options), settings, options.sam.automatic_cues, turns, watch)
            })?;
        run.note(
            "sam",
            json!({"backend": report["backend"], "model": report["model"]["name"], "model_load_seconds": report["model_load_seconds"],
                   "seconds_per_photo": report["seconds_per_photo"]}),
        );
        Ok(target.join("masks"))
    }

    fn dropped_warning(&self) -> Option<&'static str> {
        Some("SAM dropped")
    }
}

/// The default model, downloaded into the cache unless it is already there; progress goes to the first quarter.
#[cfg(feature = "sam-onnx")]
fn fetched(run: &mut Run, low: f64, high: f64) -> anyhow::Result<PathBuf> {
    use super::fetch;
    let directory = fetch::default_model_directory().ok_or_else(|| anyhow!("--masks sam: no cache directory; give --sam-model"))?;
    if fetch::present(&directory) {
        return Ok(directory);
    }
    let megabytes = (fetch::total_bytes() >> 20) as usize;
    let message = format!("Downloading the SAM 2.1 model ({megabytes} MB) from {}", fetch::REPOSITORY);
    let timeout = run.options.timeouts.sam;
    run.internal("masks", "sam-download", timeout, low, low + 0.25 * (high - low), &message, megabytes, |watch| {
        fetch::ensure(&directory, &mut |done, _| watch((done >> 20) as usize))
    })?;
    run.note("sam_download", json!({"from": fetch::REPOSITORY, "into": directory, "bytes": fetch::total_bytes()}));
    Ok(directory)
}

#[cfg(not(feature = "sam-onnx"))]
fn fetched(_run: &mut Run, _low: f64, _high: f64) -> anyhow::Result<PathBuf> {
    bail!("--masks sam needs --sam-model in a build without the feature sam-onnx")
}

#[allow(clippy::too_many_arguments)]
fn run_into(
    work: &Path,
    target: &Path,
    model: &ModelInfo,
    backend_options: &BackendOptions,
    settings: Settings,
    automatic_cues: bool,
    turns: usize,
    watch: &mut dyn FnMut(usize) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    run(&work.join("photos"), &work.join("coarse-masks"), target, model, backend_options, settings, automatic_cues, turns, watch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_names_what_is_missing() {
        if backend::compiled().is_empty() {
            assert!(readiness(None, None).unwrap_err().contains("--sam-model"));
        } else {
            // Without a model directory the default model is fetched on first use; the runtime is still needed.
            let library = std::env::temp_dir().join(format!("crispmesh-sam-library-{}", std::process::id()));
            std::fs::write(&library, b"").unwrap();
            assert!(readiness(None, Some(&library)).unwrap().contains("fetched on first use"));
            std::fs::remove_file(&library).unwrap();
        }
        let folder = std::env::temp_dir().join(format!("crispmesh-sam-ready-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        assert!(readiness(Some(&folder), None).unwrap_err().contains("model.json"));
        std::fs::write(folder.join("model.json"), backend::tests::description().to_string()).unwrap();
        std::fs::write(folder.join("encoder.onnx"), b"abc").unwrap();
        std::fs::write(folder.join("decoder.onnx"), b"").unwrap();
        let missing = folder.join("no-such-library");
        let outcome = readiness(Some(&folder), Some(&missing));
        if backend::compiled().contains(&"onnx") {
            assert!(outcome.unwrap_err().contains("shared library not found"));
            assert_eq!(readiness(Some(&folder), Some(&folder.join("model.json"))).unwrap(), "sam2.1_hiera_tiny (onnx)");
        } else {
            assert!(outcome.unwrap_err().contains("--features sam-onnx"));
        }
        // The recorded SHA-256 is compared before a model is loaded.
        let model = ModelInfo::read(&folder).unwrap();
        assert!(verify_files(&model).unwrap_err().to_string().contains("differs from"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// The whole provider on real photos: set `CRISPMESH_SAM_TEST_MODEL` to a model directory,
    /// `CRISPMESH_SAM_TEST_WORK` to the `work` directory of a `photos --keep-intermediates` run
    /// (`photos/`, `coarse-masks/`, and `sam/masks/` from `--masks external-sam` to compare with),
    /// and `ORT_DYLIB_PATH` to ONNX Runtime's shared library; or the model to a `.gguf` file and
    /// `CRISPEMBED_SAM2_LIB` to CrispEmbed's `libcrispembed-sam2` (feature `sam-ggml`).
    #[test]
    fn masks_agree_with_the_reference_route_when_a_model_is_given() {
        let (Some(model), Some(work)) = (std::env::var_os("CRISPMESH_SAM_TEST_MODEL"), std::env::var_os("CRISPMESH_SAM_TEST_WORK")) else {
            eprintln!("skipped: CRISPMESH_SAM_TEST_MODEL and CRISPMESH_SAM_TEST_WORK are not set");
            return;
        };
        let work = PathBuf::from(work);
        let model = ModelInfo::read(Path::new(&model)).unwrap();
        // A handful of photos, with the prompts of the whole set (the box is common to all).
        let all: Vec<PathBuf> = {
            let mut paths: Vec<PathBuf> = std::fs::read_dir(work.join("photos")).unwrap().map(|e| e.unwrap().path()).collect();
            paths.sort();
            paths
        };
        let supports: Vec<_> = all
            .iter()
            .map(|p| support(&open_binary_mask(&work.join("coarse-masks").join(format!("{}.png", file_name(p)))).unwrap(), true).unwrap())
            .collect();
        let first = open_photo(&all[0]).unwrap();
        let (width, height) = (first.width(), first.height());
        let prompts = frozen_prompts(&supports, width, height, true);
        let options = BackendOptions { threads: 4, accelerator: "cpu".into(), runtime: None };
        let mut network = backend::open(&model, &options).unwrap();
        let mut lowest = 1.0f64;
        for index in (0..all.len()).step_by((all.len() / 6).max(1)) {
            let photo = open_photo(&all[index]).unwrap();
            let settings = Settings { multimask: true, preserve_holes: true };
            let ours = segment(network.as_mut(), &model, photo.rgb.as_raw(), width, height, &prompts[index], settings).unwrap();
            let reference = open_binary_mask(&work.join("sam/masks").join(format!("{}.png", file_name(&all[index])))).unwrap();
            let both = ours.clean.data.iter().zip(&reference.data).filter(|(a, b)| **a != 0 && **b != 0).count() as f64;
            let either = ours.clean.data.iter().zip(&reference.data).filter(|(a, b)| **a != 0 || **b != 0).count() as f64;
            lowest = lowest.min(both / either);
        }
        assert!(lowest > 0.99, "lowest IoU with the reference masks {lowest}");
    }
}
