//! ONNX Runtime backend of the `sam` provider (cargo feature `sam-onnx`),
//! through the `ort` crate. The runtime is a shared library loaded when the
//! first model is opened (`--sam-runtime`, else `ORT_DYLIB_PATH`), so that
//! building this crate downloads and links nothing.

use crate::photos::fs::Stored as _;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{anyhow, bail};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use super::backend::{BackendOptions, ModelInfo, Prediction, SamBackend};

/// The outcome of loading the runtime library: it happens once per process.
static RUNTIME: OnceLock<Result<String, String>> = OnceLock::new();

fn runtime(library: Option<&PathBuf>) -> anyhow::Result<String> {
    let loaded = RUNTIME.get_or_init(|| {
        let path = match library {
            Some(path) => path.clone(),
            None => match std::env::var_os("ORT_DYLIB_PATH") {
                Some(path) => PathBuf::from(path),
                None => return Err("ONNX Runtime's shared library is not given (--sam-runtime FILE or ORT_DYLIB_PATH)".to_string()),
            },
        };
        if !path.stored_file() {
            return Err(format!("{}: ONNX Runtime's shared library not found", path.display()));
        }
        match ort::init_from(&path) {
            Ok(environment) => {
                environment.with_name("crispmesh-sam").commit();
                Ok(format!("{} ({})", ort::info().lines().next().unwrap_or("ONNX Runtime"), path.display()))
            }
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    });
    loaded.clone().map_err(|reason| anyhow!("{reason}"))
}

pub struct OnnxBackend {
    encoder: Session,
    decoder: Session,
    image_size: usize,
    mask_size: usize,
    /// Encoder outputs of the current image with their shapes: image_embed, high_res_0, high_res_1.
    features: Option<[(Vec<i64>, Vec<f32>); 3]>,
    description: String,
}

fn session(path: &std::path::Path, options: &BackendOptions) -> anyhow::Result<Session> {
    let fail = |what: &str, error: String| anyhow!("{}: {what}: {error}", path.display());
    let mut builder = Session::builder()
        .map_err(|e| fail("session", e.to_string()))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| fail("optimisation level", e.to_string()))?
        .with_intra_threads(options.threads.max(1))
        .map_err(|e| fail("threads", e.to_string()))?;
    match options.accelerator.as_str() {
        "cpu" => {}
        "coreml" => {
            // Sub-graphs CoreML cannot take stay on the CPU provider.
            let provider = ort::ep::CoreML::default().with_model_format(ort::ep::coreml::ModelFormat::MLProgram).build().error_on_failure();
            builder = builder.with_execution_providers([provider]).map_err(|e| fail("CoreML execution provider", e.to_string()))?;
        }
        other => bail!("--sam-accelerator {other}: the ONNX backend offers cpu and coreml"),
    }
    builder.commit_from_file(path).map_err(|e| fail("loading the graph", e.to_string()))
}

impl OnnxBackend {
    pub fn open(model: &ModelInfo, options: &BackendOptions) -> anyhow::Result<Self> {
        let runtime = runtime(options.runtime.as_ref())?;
        Ok(OnnxBackend {
            encoder: session(&model.encoder, options)?,
            decoder: session(&model.decoder, options)?,
            image_size: model.image_size,
            mask_size: model.mask_size,
            features: None,
            description: format!("{runtime}, {}, {} threads", options.accelerator, options.threads.max(1)),
        })
    }
}

impl SamBackend for OnnxBackend {
    fn description(&self) -> String {
        self.description.clone()
    }

    fn set_image(&mut self, input: &[f32]) -> anyhow::Result<()> {
        let side = self.image_size;
        if input.len() != 3 * side * side {
            bail!("the network input must hold 3 x {side} x {side} values");
        }
        self.features = None;
        let image = TensorRef::from_array_view(([1usize, 3, side, side], input)).map_err(|e| anyhow!("encoder input: {e}"))?;
        let outputs = self.encoder.run(ort::inputs!["image" => image]).map_err(|e| anyhow!("SAM image encoder: {e}"))?;
        let take = |name: &str| -> anyhow::Result<(Vec<i64>, Vec<f32>)> {
            let (shape, data) = outputs[name].try_extract_tensor::<f32>().map_err(|e| anyhow!("encoder output {name}: {e}"))?;
            Ok((shape.iter().copied().collect(), data.to_vec()))
        };
        self.features = Some([take("image_embed")?, take("high_res_0")?, take("high_res_1")?]);
        Ok(())
    }

    fn predict(&mut self, points: &[[f32; 2]], labels: &[i64]) -> anyhow::Result<Prediction> {
        let Some(features) = &self.features else { bail!("no image set") };
        if points.is_empty() || points.len() != labels.len() {
            bail!("prompts need one label per point");
        }
        let flat: Vec<f32> = points.iter().flatten().copied().collect();
        let tensor = |index: usize| {
            TensorRef::from_array_view((features[index].0.clone(), features[index].1.as_slice())).map_err(|e| anyhow!("decoder input: {e}"))
        };
        let coords = TensorRef::from_array_view(([1usize, points.len(), 2], flat.as_slice())).map_err(|e| anyhow!("decoder input: {e}"))?;
        let labels = TensorRef::from_array_view(([1usize, labels.len()], labels)).map_err(|e| anyhow!("decoder input: {e}"))?;
        let outputs = self
            .decoder
            .run(ort::inputs![
                "image_embed" => tensor(0)?, "high_res_0" => tensor(1)?, "high_res_1" => tensor(2)?,
                "point_coords" => coords, "point_labels" => labels,
            ])
            .map_err(|e| anyhow!("SAM mask decoder: {e}"))?;
        let (_, logits) = outputs["mask_logits"].try_extract_tensor::<f32>().map_err(|e| anyhow!("decoder output mask_logits: {e}"))?;
        let (_, scores) = outputs["iou"].try_extract_tensor::<f32>().map_err(|e| anyhow!("decoder output iou: {e}"))?;
        if scores.is_empty() || logits.len() != scores.len() * self.mask_size * self.mask_size {
            bail!("the decoder returned {} logits for {} masks of side {}", logits.len(), scores.len(), self.mask_size);
        }
        Ok(Prediction { logits: logits.to_vec(), scores: scores.to_vec() })
    }
}
