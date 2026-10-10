//! Building and checking a configuration the way `dense_config.build` does:
//! defaults, then an optional JSON file, then `key=value` overrides, then validation.
//!
//! Nothing here is specific to surface extraction; it lives in this module only because the
//! `mesh` command was the first to need it.

use std::path::Path;

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::config::DenseConfig;

/// The `mesh_*` settings in the order of `dense_config.py`, as `result.json` records them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeshSettings {
    pub mesh_smooth: f64,
    pub mesh_fill_sigmas: Vec<f64>,
    pub mesh_final_smooth: f64,
    pub mesh_minimum_weight: f64,
    pub mesh_confidence_cap: f64,
    pub mesh_taubin_cycles: i64,
    pub mesh_flat_base: bool,
    pub mesh_base_margin: f64,
}

impl From<&DenseConfig> for MeshSettings {
    fn from(config: &DenseConfig) -> Self {
        MeshSettings {
            mesh_smooth: config.mesh_smooth,
            mesh_fill_sigmas: config.mesh_fill_sigmas.clone(),
            mesh_final_smooth: config.mesh_final_smooth,
            mesh_minimum_weight: config.mesh_minimum_weight,
            mesh_confidence_cap: config.mesh_confidence_cap,
            mesh_taubin_cycles: config.mesh_taubin_cycles,
            mesh_flat_base: config.mesh_flat_base,
            mesh_base_margin: config.mesh_base_margin,
        }
    }
}

fn number(text: &str, key: &str) -> Result<f64> {
    text.trim().parse::<f64>().map_err(|_| anyhow!("{key} needs a number, found {text:?}"))
}

fn integer(text: &str, key: &str, through_float: bool) -> Result<Value> {
    // Python: `int(float(v))` for list entries, `int(v)` for a single value.
    let value = if through_float {
        number(text, key)?.trunc() as i64
    } else {
        text.trim().parse::<i64>().map_err(|_| anyhow!("{key} needs an integer, found {text:?}"))?
    };
    Ok(Value::from(value))
}

/// One override as text, converted to the kind of the setting's default.
fn coerce(key: &str, default: &Value, text: &str) -> Result<Value> {
    Ok(match default {
        Value::Bool(_) => match text.to_lowercase().as_str() {
            "true" | "1" | "yes" => Value::Bool(true),
            "false" | "0" | "no" => Value::Bool(false),
            _ => bail!("{key} needs true or false"),
        },
        Value::Array(entries) => {
            let integers = entries.first().is_some_and(|e| e.is_i64() || e.is_u64());
            let parts = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|part| !part.is_empty());
            let values = parts.map(|part| if integers { integer(part, key, true) } else { number(part, key).map(Value::from) });
            Value::Array(values.collect::<Result<Vec<Value>>>()?)
        }
        Value::Number(n) if n.is_i64() || n.is_u64() => integer(text, key, false)?,
        _ => Value::from(number(text, key)?),
    })
}

/// Defaults, then `config_path` (a `config.json` or a `result.json` with a `configuration`
/// object), then `overrides` (`key=value`, dashes allowed in the key), then `validate`.
pub fn build(config_path: Option<&Path>, overrides: &[String]) -> Result<DenseConfig> {
    let config = match config_path {
        Some(path) => DenseConfig::load(path).with_context(|| format!("cannot load configuration {}", path.display()))?,
        None => DenseConfig::default(),
    };
    let mut record = serde_json::to_value(&config)?;
    let fields = record.as_object_mut().expect("a configuration serialises to an object");
    for item in overrides {
        let (key, text) = item.split_once('=').unwrap_or((item, ""));
        let key = key.trim().replace('-', "_");
        let known = fields.get(&key).filter(|_| item.contains('='));
        let Some(default) = known else {
            let names: Vec<&str> = fields.keys().map(String::as_str).collect();
            bail!("unknown setting {item:?}; known: {}", names.join(", "));
        };
        let value = coerce(&key, default, text)?;
        fields.insert(key, value);
    }
    let config: DenseConfig = serde_json::from_value(record)?;
    validate(&config)?;
    Ok(config)
}

/// The checks of `DenseConfig.validate` in `dense_config.py`, with the same messages.
pub fn validate(c: &DenseConfig) -> Result<()> {
    fn need(condition: bool, message: &str) -> Result<()> {
        ensure!(condition, "dense configuration: {message}");
        Ok(())
    }
    need((1..=5).contains(&c.sizes.len()) && c.sizes.iter().all(|s| (32..=4096).contains(s)), "sizes must be 1..5 values in 32..4096")?;
    need(c.sizes.windows(2).all(|pair| pair[0] <= pair[1]), "sizes must increase")?;
    need(2 <= c.best_of && c.best_of <= c.neighbours && c.neighbours <= 16, "need 2 <= best_of <= neighbours <= 16")?;
    need(0.0 < c.minimum_angle && c.minimum_angle < c.maximum_angle && c.maximum_angle < 90.0, "invalid neighbour angles")?;
    need((16..=512).contains(&c.planes), "planes must be 16..512")?;
    need(c.windows.iter().all(|w| w % 2 == 1 && (3..=31).contains(w)), "windows must be odd, 3..31")?;
    need(c.passes.iter().all(|p| (1..=4).contains(p)), "passes must be 1..4")?;
    need(c.band_first.iter().chain([&c.band_later]).all(|b| (2..=32).contains(b)), "band values must be in 2..32")?;
    let score = |s: f64| 0.0 < s && s < 1.0;
    need(score(c.min_score) && score(c.hull_front_min_score), "scores must be in (0,1)")?;
    need(c.min_variance > 0.0 && 0.0 < c.window_fill && c.window_fill <= 1.0, "invalid texture gates")?;
    need(c.tolerances.iter().all(|&t| 0.0 < t && t < 0.1), "tolerances must be in (0,0.1)")?;
    need(c.min_votes.iter().all(|&v| 1 <= v && v <= c.vote_neighbours) && c.vote_neighbours <= 32, "invalid votes")?;
    need((64..=1024).contains(&c.grid), "grid must be 64..1024")?;
    need(
        (0..=16).contains(&c.hull_dilate) && (0..=64).contains(&c.hull_allowed) && (0..=64).contains(&c.repair_loose),
        "invalid hull tolerances",
    )?;
    need(c.repair_loose == 0 || c.repair_loose >= c.hull_allowed, "repair_loose must be 0 or at least hull_allowed")?;
    need((1..=4).contains(&c.repair_rounds), "repair_rounds must be 1..4")?;
    need((0 <= c.hull_front_level && (c.hull_front_level as usize) < c.sizes.len()) || !c.hull_front, "hull_front_level outside pyramid")?;
    need((0..=3).contains(&c.fused_passes) && (2..=32).contains(&c.fused_band), "invalid fused pass values")?;
    need((0.0..=3.0).contains(&c.rim_fraction) && c.truncation_voxels >= 1.0, "invalid fusion values")?;
    need((0.0..=10.0).contains(&c.truncation_noise), "truncation_noise must be in 0..10")?;
    need(c.behind_voxels >= c.truncation_voxels && (0.0..=1.0).contains(&c.behind_weight), "invalid inside vote")?;
    need(0.0 < c.free_weight && c.free_weight <= 1.0, "free_weight must be in (0,1]")?;
    need(c.mesh_smooth > 0.0 && c.mesh_confidence_cap > 0.0 && (0..=100).contains(&c.mesh_taubin_cycles), "invalid mesh values")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> Result<DenseConfig> {
        build(None, &items.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn defaults_are_valid_and_overrides_are_typed() {
        assert_eq!(build(None, &[]).unwrap(), DenseConfig::default());
        let c = set(&[
            "mesh_taubin_cycles=0",
            "mesh-fill-sigmas=1.5, 3",
            "sizes=256 512.0",
            "mesh_flat_base=No",
            "grid = 320",
            "mesh_smooth=2",
        ])
        .unwrap();
        assert_eq!(c.mesh_taubin_cycles, 0);
        assert_eq!(c.mesh_fill_sigmas, vec![1.5, 3.0]);
        assert_eq!(c.sizes, vec![256, 512]);
        assert!(!c.mesh_flat_base);
        assert_eq!(c.grid, 320);
        assert_eq!(c.mesh_smooth, 2.0);
    }

    #[test]
    fn bad_overrides_are_refused() {
        for items in [
            &["nonsense=1"][..],
            &["grid"],
            &["mesh_flat_base=maybe"],
            &["grid=3.5"],
            &["mesh_smooth=wide"],
            &["mesh_smooth=0"],
            &["grid=32"],
        ] {
            assert!(set(items).is_err(), "{items:?} should be refused");
        }
        assert!(set(&["mesh_taubin_cycles=101"]).unwrap_err().to_string().contains("invalid mesh values"));
    }

    #[test]
    fn a_result_file_configures_a_run() {
        let path = std::env::temp_dir().join(format!("crisp3ds-settings-{}.json", std::process::id()));
        crate::storage::write(&path, r#"{"closed": true, "configuration": {"mesh_smooth": 1.5, "mesh_fill_sigmas": [3.0]}}"#).unwrap();
        let c = build(Some(&path), &["mesh_smooth=1.25".to_string()]).unwrap();
        crate::storage::remove_file(&path).unwrap();
        assert_eq!((c.mesh_smooth, c.mesh_fill_sigmas.clone(), c.grid), (1.25, vec![3.0], 400));
    }

    #[test]
    fn mesh_settings_cover_every_mesh_field() {
        let all = serde_json::to_value(DenseConfig::default()).unwrap();
        let ours = serde_json::to_value(MeshSettings::from(&DenseConfig::default())).unwrap();
        let expected: Vec<(&String, &Value)> = all
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.starts_with("mesh_") && !crate::config::NATIVE_ONLY.contains(&key.as_str()))
            .collect();
        let found: Vec<(&String, &Value)> = ours.as_object().unwrap().iter().collect();
        assert_eq!(found, expected);
        let text = serde_json::to_string(&MeshSettings::from(&DenseConfig::default())).unwrap();
        assert!(text.starts_with(r#"{"mesh_smooth":1.0,"mesh_fill_sigmas":[2.0,4.0],"mesh_final_smooth":0.6,"#), "{text}");
    }
}
