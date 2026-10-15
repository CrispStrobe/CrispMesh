//! The network behind the `sam` mask provider, as a small interface: a model
//! (a directory described by `model.json` with ONNX graphs, or one GGUF file)
//! and a backend that turns the prepared image and the prompts into mask
//! logits. Which backends exist is decided at build time by cargo features
//! (`sam-onnx`: ONNX Runtime through the `ort` crate; `sam-ggml`: CrispEmbed's
//! ggml engine through its C API); everything else of the provider is
//! independent of them.

use crate::photos::fs::Stored as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use serde_json::Value;

/// Schema of `model.json`, written by `tools/sam2_export_onnx.py`.
pub const MODEL_SCHEMA: &str = "crispmesh_sam_model_v1";

/// A model directory.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    pub directory: PathBuf,
    pub name: String,
    pub license: String,
    /// `onnx`: two graphs (`encoder`, `decoder`); `gguf`: one file for both (`encoder` = `decoder`).
    pub format: String,
    /// Side of the square network input.
    pub image_size: usize,
    /// Side of the low-resolution mask logits.
    pub mask_size: usize,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    /// The decoder's own fallback from its single mask to the best of the others (`None`: never).
    pub stability: Option<(f32, f32)>,
    /// SHA-256 of the files as `model.json` records them.
    pub files: Value,
}

impl ModelInfo {
    pub fn parse(directory: &Path, description: &Value) -> anyhow::Result<Self> {
        if description["schema"] != MODEL_SCHEMA {
            bail!("model.json: expected schema {MODEL_SCHEMA}");
        }
        let text = |key: &str| description[key].as_str().map(str::to_string).ok_or_else(|| anyhow!("model.json: no {key}"));
        let size = |key: &str| {
            description[key]
                .as_u64()
                .filter(|&v| (16..=4096).contains(&v))
                .map(|v| v as usize)
                .ok_or_else(|| anyhow!("model.json: no {key}"))
        };
        let triple = |key: &str| -> anyhow::Result<[f32; 3]> {
            let values: Vec<f32> = description[key].as_array().into_iter().flatten().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
            values.try_into().map_err(|_| anyhow!("model.json: {key} must be three numbers"))
        };
        let dynamic = &description["dynamic_multimask"];
        let stability = match (dynamic["enabled"].as_bool(), dynamic["stability_delta"].as_f64(), dynamic["stability_threshold"].as_f64()) {
            (Some(true), Some(delta), Some(threshold)) => Some((delta as f32, threshold as f32)),
            _ => None,
        };
        let file = |key: &str| -> anyhow::Result<PathBuf> {
            let name = text(key)?;
            if name.contains(['/', '\\']) || name.starts_with('.') {
                bail!("model.json: {key} must be a file name in the model directory");
            }
            Ok(directory.join(name))
        };
        Ok(ModelInfo {
            directory: directory.to_path_buf(),
            name: text("model")?,
            license: text("license")?,
            format: text("format")?,
            image_size: size("image_size")?,
            mask_size: size("mask_size")?,
            mean: triple("mean")?,
            std: triple("std")?,
            encoder: file("encoder")?,
            decoder: file("decoder")?,
            stability,
            files: description["files"].clone(),
        })
    }

    /// A GGUF file (CrispEmbed's `convert-sam2-to-gguf.py`): SAM 2.1 with its fixed input and normalisation.
    pub fn gguf(file: &Path) -> Self {
        let name = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        ModelInfo {
            directory: file.parent().map(Path::to_path_buf).unwrap_or_default(),
            name,
            license: "Apache-2.0 (SAM 2, Copyright Meta Platforms, Inc. and affiliates)".to_string(),
            format: "gguf".to_string(),
            image_size: 1024,
            mask_size: 256,
            mean: [0.485, 0.456, 0.406],
            std: [0.229, 0.224, 0.225],
            encoder: file.to_path_buf(),
            decoder: file.to_path_buf(),
            stability: Some((0.05, 0.98)),
            files: Value::Null,
        }
    }

    /// Reads `<directory>/model.json` and checks that the files it names are there; a `.gguf` file is a model too.
    pub fn read(directory: &Path) -> anyhow::Result<Self> {
        if directory.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf")) {
            if !directory.stored_file() {
                bail!("{}: no such GGUF file", directory.display());
            }
            return Ok(Self::gguf(directory));
        }
        let path = directory.join("model.json");
        let text = crate::photos::fs::read_to_string(&path)
            .with_context(|| format!("{} (a SAM model directory holds model.json)", path.display()))?;
        let description: Value = serde_json::from_str(&text).with_context(|| path.display().to_string())?;
        let model = Self::parse(directory, &description)?;
        for file in [&model.encoder, &model.decoder] {
            if !file.stored_file() {
                bail!("{}: named in model.json but not found", file.display());
            }
        }
        Ok(model)
    }
}

/// Mask logits of the decoder's output tokens (token 0: the single mask; 1 and up: the alternatives).
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// `scores.len()` maps of `mask_size` x `mask_size`, one after another.
    pub logits: Vec<f32>,
    /// The quality the network predicts for each.
    pub scores: Vec<f32>,
}

/// One loaded network. Used like SAM's image predictor: an image, then any number of prompts for it.
pub trait SamBackend {
    /// What ran, for the report (`onnxruntime 1.22 (cpu)`).
    fn description(&self) -> String;

    /// `input`: `3 * image_size * image_size` values, planar, resized and normalised.
    fn set_image(&mut self, input: &[f32]) -> anyhow::Result<()>;

    /// `points` in pixels of the network input (box corners first), `labels` 1 object, 0 background, 2 and 3 box corners.
    fn predict(&mut self, points: &[[f32; 2]], labels: &[i64]) -> anyhow::Result<Prediction>;
}

/// How a backend is to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendOptions {
    pub threads: usize,
    /// `cpu`, or an accelerator the backend knows (`coreml` for ONNX, `gpu` for ggml).
    pub accelerator: String,
    /// The runtime library: ONNX Runtime's shared library, or `libcrispembed-sam2` for a GGUF model.
    pub runtime: Option<PathBuf>,
}

/// The backends compiled into this build, by model format.
pub fn compiled() -> &'static [&'static str] {
    &[
        #[cfg(feature = "sam-onnx")]
        "onnx",
        #[cfg(feature = "sam-ggml")]
        "gguf",
    ]
}

/// Why this build cannot run a model of `format`, if it cannot.
pub fn unavailable(format: &str) -> Option<String> {
    if compiled().contains(&format) {
        return None;
    }
    Some(match format {
        "onnx" => "this build has no ONNX backend: build crispmesh-dense with `--features sam-onnx`".to_string(),
        "gguf" => "this build has no ggml backend: build crispmesh-dense with `--features sam-ggml`".to_string(),
        other => format!("this build has no backend for SAM models of format `{other}`"),
    })
}

/// Loads the model with the backend for its format.
pub fn open(model: &ModelInfo, options: &BackendOptions) -> anyhow::Result<Box<dyn SamBackend>> {
    if let Some(reason) = unavailable(&model.format) {
        bail!("{reason}");
    }
    match model.format.as_str() {
        #[cfg(feature = "sam-onnx")]
        "onnx" => Ok(Box::new(super::onnx::OnnxBackend::open(model, options)?)),
        #[cfg(feature = "sam-ggml")]
        "gguf" => Ok(Box::new(super::ggml::GgmlBackend::open(model, options)?)),
        _ => {
            let _ = options;
            bail!("no backend for SAM models of format `{}`", model.format)
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    pub fn description() -> Value {
        json!({
            "schema": MODEL_SCHEMA, "model": "sam2.1_hiera_tiny", "license": "Apache-2.0", "format": "onnx",
            "image_size": 1024, "mask_size": 256, "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225],
            "encoder": "encoder.onnx", "decoder": "decoder.onnx",
            "dynamic_multimask": {"enabled": true, "stability_delta": 0.05, "stability_threshold": 0.98},
            "files": {"encoder.onnx": {"bytes": 1, "sha256": "00"}},
        })
    }

    #[test]
    fn a_model_directory_is_described_by_model_json() {
        let folder = std::env::temp_dir().join(format!("crispmesh-sam-model-{}", std::process::id()));
        let _ = crate::photos::fs::remove_dir_all(&folder);
        crate::photos::fs::create_dir_all(&folder).unwrap();
        assert!(ModelInfo::read(&folder).unwrap_err().to_string().contains("model.json"));
        crate::photos::fs::write(folder.join("model.json"), description().to_string()).unwrap();
        assert!(ModelInfo::read(&folder).unwrap_err().to_string().contains("encoder.onnx: named in model.json but not found"));
        crate::photos::fs::write(folder.join("encoder.onnx"), b"").unwrap();
        crate::photos::fs::write(folder.join("decoder.onnx"), b"").unwrap();
        let model = ModelInfo::read(&folder).unwrap();
        assert_eq!((model.image_size, model.mask_size, model.stability), (1024, 256, Some((0.05, 0.98))));
        assert_eq!(model.decoder, folder.join("decoder.onnx"));
        let mut wrong = description();
        wrong["encoder"] = json!("../encoder.onnx");
        assert!(ModelInfo::parse(&folder, &wrong).is_err());
        wrong = description();
        wrong["schema"] = json!("other");
        assert!(ModelInfo::parse(&folder, &wrong).is_err());
        // Without a backend in the build, opening says how to get one.
        let options = BackendOptions { threads: 1, accelerator: "cpu".into(), runtime: None };
        if compiled().is_empty() {
            assert!(open(&model, &options).err().unwrap().to_string().contains("--features sam-onnx"));
        }
        assert_eq!(unavailable("gguf").is_none(), compiled().contains(&"gguf"));
        if !compiled().contains(&"gguf") {
            assert!(unavailable("gguf").unwrap().contains("--features sam-ggml"));
        }
        // A GGUF file is a model of its own: SAM 2.1's fixed input, the file as encoder and decoder.
        assert!(ModelInfo::read(&folder.join("missing.gguf")).unwrap_err().to_string().contains("no such GGUF file"));
        crate::photos::fs::write(folder.join("sam2.1-hiera-tiny-f16.gguf"), b"").unwrap();
        let gguf = ModelInfo::read(&folder.join("sam2.1-hiera-tiny-f16.gguf")).unwrap();
        assert_eq!((gguf.format.as_str(), gguf.image_size, gguf.mask_size), ("gguf", 1024, 256));
        assert_eq!((gguf.encoder.clone(), gguf.name.as_str()), (gguf.decoder.clone(), "sam2.1-hiera-tiny-f16"));
        crate::photos::fs::remove_dir_all(&folder).unwrap();
    }
}
