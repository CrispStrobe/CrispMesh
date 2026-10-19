//! Options shared by the photo-optimised surfaces (the neural surface and the 2D splats): a quality level that sets
//! speed against detail, and where the scene comes from. Every setting can also be given on its own (`NAME=VALUE`),
//! after the level's values.
//!
//! The photos' resolution follows the object, not the camera: each view is cropped to one common window around the
//! masks and reduced by the whole factor that brings that window closest to, but not above, the level's `crop`
//! width (small objects are never enlarged). The scene is scaled to the object's bounding sphere, so lengths in the
//! settings (hull band, sample spacing) are relative to the object and need no adjusting for its size.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Result};

use super::train::{load_pack, scene_from_inputs, Scene};

/// Speed against detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Bounded real-capture preset: 3,000 steps, 320-pixel crops, 192-cell mesh.
    Capture,
    /// A preview in minutes: fewer steps, photos reduced to about 480 pixels across the object, coarser mesh.
    Fast,
    /// The reference runs' settings (the default).
    Standard,
    /// More photo resolution and a finer mesh; slower and more memory.
    Best,
}

impl FromStr for Quality {
    type Err = anyhow::Error;
    fn from_str(text: &str) -> Result<Self> {
        match text {
            "capture" => Ok(Quality::Capture),
            "fast" => Ok(Quality::Fast),
            "standard" => Ok(Quality::Standard),
            "best" => Ok(Quality::Best),
            _ => bail!("quality must be capture, fast, standard or best, not {text}"),
        }
    }
}

impl Quality {
    pub fn name(self) -> &'static str {
        match self {
            Quality::Capture => "capture",
            Quality::Fast => "fast",
            Quality::Standard => "standard",
            Quality::Best => "best",
        }
    }
}

/// Where a scene comes from and at which resolution.
#[derive(Clone, Debug, Default)]
pub struct Source {
    /// The silhouette mesh (start surface and band) for an inputs directory.
    pub hull_mesh: Option<PathBuf>,
    /// Folders replacing the inputs' original photos or masks.
    pub photos: Option<PathBuf>,
    pub masks: Option<PathBuf>,
    /// Photo reduction; 0: from `crop`.
    pub div: usize,
    /// Target width in pixels of the crop window around the object.
    pub crop: usize,
}

impl Source {
    pub fn for_quality(quality: Quality, best_crop: usize) -> Self {
        let crop = match quality {
            Quality::Capture => 320,
            Quality::Fast => 480,
            Quality::Standard => 720,
            Quality::Best => best_crop,
        };
        Source { crop, ..Default::default() }
    }

    /// Applies one `NAME=VALUE`; false if the name is not a source setting.
    pub fn set(&mut self, name: &str, value: &str) -> Result<bool> {
        match name {
            "hull_mesh" => self.hull_mesh = Some(PathBuf::from(value)),
            "photos" => self.photos = Some(PathBuf::from(value)),
            "masks" => self.masks = Some(PathBuf::from(value)),
            "div" => self.div = whole(name, value)? as usize,
            "crop" => self.crop = whole(name, value)? as usize,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The scene: a pack (`scene.json`) as it is, or an inputs directory (`cameras.json`) with the hull mesh.
    pub fn scene(&self, source: &Path, seed: u64) -> Result<Scene> {
        if crate::storage::exists(source.join("scene.json")) {
            return load_pack(source);
        }
        let hull = self.hull_mesh.as_deref().ok_or_else(|| anyhow!("an inputs directory needs hull_mesh=STL"))?;
        scene_from_inputs(source, self.photos.as_deref(), self.masks.as_deref(), hull, self.div, self.crop, seed)
    }
}

pub fn whole(name: &str, value: &str) -> Result<u32> {
    value.parse::<u32>().map_err(|_| anyhow!("{name} needs a whole number"))
}

pub fn number(name: &str, value: &str) -> Result<f32> {
    value.parse::<f32>().map_err(|_| anyhow!("{name} needs a number"))
}

/// Splits `NAME=VALUE` items; `quality=` first (it sets the others' defaults), the rest in order.
pub fn split(items: &[String]) -> Result<(Quality, Vec<(String, String)>)> {
    let mut quality = Quality::Standard;
    let mut rest = Vec::new();
    for item in items {
        let (name, value) = item.split_once('=').ok_or_else(|| anyhow!("expected NAME=VALUE, got {item}"))?;
        if name == "quality" {
            quality = value.parse()?;
        } else {
            rest.push((name.to_string(), value.to_string()));
        }
    }
    Ok((quality, rest))
}
