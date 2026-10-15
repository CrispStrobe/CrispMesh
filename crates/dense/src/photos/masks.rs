//! Mask providers: from the staged photos (`work/photos/capture_NNNN.png`)
//! and their coarse dark-object masks to one 0/255 mask per photo in the
//! photo's own frame, as `<folder>/capture_NNNN.png.png`. What follows is the
//! same for every provider: dark-hole cleanup, statistics, contact sheet
//! (`run.rs`).

use crate::photos::fs::Stored as _;
use std::path::PathBuf;

use anyhow::{anyhow, bail};
use serde_json::Value;

use super::options::{MaskChoice, Options};
use super::process::ExternalCommand;
use super::providers::{Provider, MASKS_EXTERNAL_SAM, MASKS_IMPORT, MASKS_THRESHOLD};
use super::run::{count_files, Run};
use super::staging::import_masks;

/// Contract of the masks module.
pub trait MaskProvider {
    fn info(&self) -> &'static Provider;

    /// Fails with a clear message when the provider cannot run with these options.
    fn check(&self, options: &Options) -> anyhow::Result<()>;

    /// Produces the masks and returns their folder. Progress goes from `low` to `high` of the `masks` stage.
    fn segment(&self, run: &mut Run, photo_map: &Value, low: f64, high: f64) -> anyhow::Result<PathBuf>;

    /// What to say when the masks leave out much of the coarse dark region.
    fn dropped_warning(&self) -> Option<&'static str>;
}

pub fn mask_provider(options: &Options) -> Box<dyn MaskProvider> {
    match &options.masks {
        MaskChoice::Threshold => Box::new(ThresholdMasks),
        MaskChoice::Background => Box::new(BackgroundMasks),
        MaskChoice::Import(folder) => Box::new(ImportMasks { folder: folder.clone() }),
        MaskChoice::ExternalSam => Box::new(ExternalSam),
        MaskChoice::Sam => Box::new(super::sam::provider::NativeSam),
    }
}

/// The coarse masks are the masks: largest dark region below the threshold.
pub struct ThresholdMasks;

impl MaskProvider for ThresholdMasks {
    fn info(&self) -> &'static Provider {
        &MASKS_THRESHOLD
    }

    fn check(&self, _options: &Options) -> anyhow::Result<()> {
        Ok(())
    }

    fn segment(&self, run: &mut Run, _photo_map: &Value, _low: f64, _high: f64) -> anyhow::Result<PathBuf> {
        Ok(run.out.join("work/coarse-masks"))
    }

    fn dropped_warning(&self) -> Option<&'static str> {
        None
    }
}

pub struct ImportMasks {
    pub folder: PathBuf,
}

impl MaskProvider for ImportMasks {
    fn info(&self) -> &'static Provider {
        &MASKS_IMPORT
    }

    fn check(&self, _options: &Options) -> anyhow::Result<()> {
        if !self.folder.stored_dir() {
            bail!("--masks import:{} is not a directory", self.folder.display());
        }
        Ok(())
    }

    fn segment(&self, run: &mut Run, photo_map: &Value, low: f64, high: f64) -> anyhow::Result<PathBuf> {
        let target = run.out.join("work/imported-masks");
        let (count, small) = (run.options.photo_count, run.options.timeouts.small);
        run.internal("masks", "import-masks", small, low, high, "Reading supplied masks", count, |watch| {
            import_masks(&self.folder, &target, photo_map, watch)
        })?;
        Ok(target)
    }

    fn dropped_warning(&self) -> Option<&'static str> {
        Some("the supplied masks leave out")
    }
}

/// SAM 2.1 through the reference `segment.py`, prompted by the coarse masks.
pub struct ExternalSam;

/// The command `photos_to_inputs.py` builds for `segment.py`.
pub fn sam_command(options: &Options) -> anyhow::Result<ExternalCommand> {
    let sam = &options.sam;
    let work = options.output.join("work");
    let text = |path: PathBuf| path.to_string_lossy().to_string();
    let need = |value: Option<String>, name: &str| value.ok_or_else(|| anyhow!("--masks external-sam needs {name}"));
    let repository = need(sam.repository.clone().map(text), "--sam-repository")?;
    let mut command: Vec<String> = vec![
        need(sam.python.clone(), "--sam-python")?,
        "-m".into(),
        "scripts.turntable_mesh.segment".into(),
        "--images".into(),
        text(work.join("photos")),
        "--coarse-masks".into(),
        text(work.join("coarse-masks")),
        "--output".into(),
        text(work.join("sam")),
        "--source".into(),
        need(sam.source.clone().map(text), "--sam-source")?,
        "--checkpoint".into(),
        need(sam.checkpoint.clone().map(text), "--sam-checkpoint")?,
        "--device".into(),
        sam.device.clone(),
        "--views".into(),
        options.photo_count.to_string(),
    ];
    if let Some(config) = &sam.config {
        command.extend(["--model-config".to_string(), config.clone()]);
    }
    for (flag, on) in [("--multimask", sam.multimask), ("--preserve-holes", sam.preserve_holes), ("--automatic-cues", sam.automatic_cues)] {
        if on {
            command.push(flag.to_string());
        }
    }
    let pythonpath = std::env::join_paths(std::iter::once(repository).chain(sam.pythonpath.iter().cloned()))
        .map_err(|e| anyhow!("--sam-pythonpath: {e}"))?;
    let environment = vec![
        ("PYTHONPATH".to_string(), pythonpath.to_string_lossy().to_string()),
        ("PYTORCH_ENABLE_MPS_FALLBACK".to_string(), "0".to_string()),
    ];
    Ok(ExternalCommand { name: "sam".to_string(), command, environment })
}

impl MaskProvider for ExternalSam {
    fn info(&self) -> &'static Provider {
        &MASKS_EXTERNAL_SAM
    }

    fn check(&self, options: &Options) -> anyhow::Result<()> {
        let sam = &options.sam;
        let missing: Vec<&str> = [
            ("--sam-python / CRISPMESH_SAM_PYTHON", sam.python.is_some()),
            ("--sam-source / CRISPMESH_SAM_SOURCE", sam.source.is_some()),
            ("--sam-checkpoint / CRISPMESH_SAM_CHECKPOINT", sam.checkpoint.is_some()),
            ("--sam-repository / CRISPMESH_REPOSITORY (the checkout with scripts/turntable_mesh/segment.py)", sam.repository.is_some()),
        ]
        .iter()
        .filter(|(_, given)| !given)
        .map(|(name, _)| *name)
        .collect();
        if !missing.is_empty() {
            bail!("missing tool locations: {} (or choose --masks threshold, which needs none)", missing.join("; "));
        }
        Ok(())
    }

    fn segment(&self, run: &mut Run, _photo_map: &Value, low: f64, high: f64) -> anyhow::Result<PathBuf> {
        let options = run.options;
        let produced = run.out.join("work/sam/masks");
        let count = options.photo_count;
        let counter = || count_files(&produced, "png") as f64 / count as f64;
        let command = sam_command(options)?;
        let directory = options.sam.repository.clone().unwrap_or_else(|| run.out.to_path_buf());
        run.external("masks", &command, &directory, options.timeouts.sam, low, high, "Segmenting with SAM 2.1", Some(&counter))?;
        Ok(produced)
    }

    fn dropped_warning(&self) -> Option<&'static str> {
        Some("SAM dropped")
    }
}

/// Background separation is written by staging, beside the original photos.
pub struct BackgroundMasks;
impl MaskProvider for BackgroundMasks {
    fn info(&self) -> &'static Provider {
        &super::providers::MASKS_BACKGROUND
    }
    fn check(&self, _: &Options) -> anyhow::Result<()> {
        Ok(())
    }
    fn segment(&self, run: &mut Run, _: &Value, _: f64, _: f64) -> anyhow::Result<PathBuf> {
        Ok(run.out.join("work/coarse-masks"))
    }
    fn dropped_warning(&self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::options::resolve;
    use crate::photos::options::tests::{arguments, scratch};
    use crate::photos::staging::capture_name;

    #[test]
    fn sam_command_is_the_reference_command() {
        let folder = scratch("sam");
        let variables = |name: &str| match name {
            "CRISPMESH_SAM_PYTHON" => Some("/sam/python".to_string()),
            "CRISPMESH_SAM_SOURCE" => Some("/sam/source".to_string()),
            "CRISPMESH_REPOSITORY" => Some("/repo".to_string()),
            _ => None,
        };
        let more = [
            "--masks",
            "external-sam",
            "--device",
            "cpu",
            "--no-sam-multimask",
            "--sam-config",
            "configs/x.yaml",
            "--sam-pythonpath",
            "/a:/b",
        ];
        let options = resolve(&arguments(&folder, &more), &variables).unwrap();
        let provider = mask_provider(&options);
        assert_eq!(provider.info().name, "external-sam");
        assert_eq!(
            provider.check(&options).unwrap_err().to_string(),
            "missing tool locations: --sam-checkpoint / CRISPMESH_SAM_CHECKPOINT (or choose --masks threshold, which needs none)"
        );
        let options = resolve(&arguments(&folder, &[&more[..], &["--sam-checkpoint", "/sam/tiny.pt"]].concat()), &variables).unwrap();
        provider.check(&options).unwrap();
        let sam = sam_command(&options).unwrap();
        assert_eq!(sam.command[..3], ["/sam/python", "-m", "scripts.turntable_mesh.segment"]);
        let after = |flag: &str| sam.command[sam.command.iter().position(|w| w == flag).unwrap() + 1].clone();
        assert_eq!(
            (after("--device").as_str(), after("--views").as_str(), after("--model-config").as_str()),
            ("cpu", "3", "configs/x.yaml")
        );
        assert_eq!(PathBuf::from(after("--coarse-masks")), folder.join("out/work/coarse-masks"));
        assert_eq!(sam.command[sam.command.len() - 2..], ["--preserve-holes", "--automatic-cues"]);
        if cfg!(unix) {
            assert_eq!(sam.environment[0], ("PYTHONPATH".to_string(), "/repo:/a:/b".to_string()));
        }
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn imported_masks_are_found_by_capture_or_photo_name() {
        let root = std::env::temp_dir().join(format!("crispmesh-import-masks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("given")).unwrap();
        let mask = |level: u8| image::GrayImage::from_fn(8, 6, |x, _| image::Luma([if x < 4 { level } else { 0 }]));
        mask(255).save(root.join("given/capture_0000.png")).unwrap();
        mask(200).save(root.join("given/thing_2.jpg.png")).unwrap();
        mask(128).save(root.join("given/thing_10.png")).unwrap();
        let map = serde_json::json!({"width": 8, "height": 6, "photos": [{"source": "thing_1.jpg"}, {"source": "thing_2.jpg"}, {"source": "thing_10.jpg"}]});
        let mut seen = 0;
        import_masks(&root.join("given"), &root.join("out"), &map, &mut |done| {
            seen = done;
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 3);
        for index in 0..3 {
            let read = crate::photos::staging::open_binary_mask(&root.join("out").join(format!("{}.png", capture_name(index)))).unwrap();
            assert_eq!(read.data.iter().filter(|&&m| m == 1).count(), 24);
        }
        mask(100).save(root.join("given/thing_10.png")).unwrap();
        assert!(import_masks(&root.join("given"), &root.join("out2"), &map, &mut |_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("mask is empty"));
        std::fs::remove_file(root.join("given/thing_10.png")).unwrap();
        assert!(import_masks(&root.join("given"), &root.join("out3"), &map, &mut |_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("no mask for photo thing_10.jpg"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
