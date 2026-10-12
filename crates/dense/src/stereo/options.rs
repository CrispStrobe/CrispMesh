//! Settings resolution for the stereo stage: defaults, then an optional JSON
//! file, then `key=value` overrides, then validation. Mirrors `build`,
//! `_coerce` and `DenseConfig.validate` of `scripts/turntable_mesh/dense_config.py`.

use std::path::Path;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::config::DenseConfig;

fn coerce(name: &str, default: &Value, text: &str) -> anyhow::Result<Value> {
    let number = |item: &str, integer: bool| -> anyhow::Result<Value> {
        let value: f64 = item.trim().parse().map_err(|_| anyhow!("{name} needs a number, got {item:?}"))?;
        Ok(if integer { json!(value.trunc() as i64) } else { json!(value) })
    };
    match default {
        Value::Array(items) => {
            let integer = items.first().map(Value::is_i64).unwrap_or(false);
            let parts: Vec<&str> = text.split([',', ' ']).filter(|p| !p.is_empty()).collect();
            if parts.is_empty() {
                bail!("{name} needs at least one value");
            }
            Ok(Value::Array(parts.into_iter().map(|p| number(p, integer)).collect::<anyhow::Result<_>>()?))
        }
        Value::Bool(_) => match text.to_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(json!(true)),
            "false" | "0" | "no" => Ok(json!(false)),
            _ => bail!("{name} needs true or false"),
        },
        Value::Number(n) if n.is_i64() => {
            Ok(json!(text.trim().parse::<i64>().map_err(|_| anyhow!("{name} needs an integer, got {text:?}"))?))
        }
        _ => number(text, false),
    }
}

/// Defaults, then `config_path` (a `config.json` or a `result.json` with a
/// `configuration` object; unknown keys are ignored), then overrides.
pub fn build(config_path: Option<&Path>, overrides: &[String]) -> anyhow::Result<DenseConfig> {
    let mut values = serde_json::to_value(DenseConfig::default())?;
    let known = values.as_object().cloned().expect("configuration is an object");
    if let Some(path) = config_path {
        let loaded: Value = serde_json::from_str(&crate::storage::read_to_string(path).with_context(|| path.display().to_string())?)
            .with_context(|| path.display().to_string())?;
        let loaded = loaded.get("configuration").cloned().unwrap_or(loaded);
        if let Value::Object(map) = loaded {
            for (key, value) in map {
                if known.contains_key(&key) {
                    values[&key] = value;
                }
            }
        }
    }
    for item in overrides {
        let Some((key, text)) = item.split_once('=') else {
            bail!("unknown setting {item:?}; expected key=value");
        };
        let key = key.trim().replace('-', "_");
        let Some(default) = known.get(&key) else {
            let mut names: Vec<&str> = known.keys().map(String::as_str).collect();
            names.sort_unstable();
            bail!("unknown setting {item:?}; known: {}", names.join(", "));
        };
        values[&key] = coerce(&key, default, text)?;
    }
    let config: DenseConfig = serde_json::from_value(values).context("dense configuration")?;
    validate(&config)?;
    Ok(config)
}

pub fn validate(c: &DenseConfig) -> anyhow::Result<()> {
    if c.patchmatch && !cfg!(feature = "research-patchmatch") {
        anyhow::bail!("dense configuration: patchmatch is excluded from this build pending patent review; use the default band refinement");
    }
    anyhow::ensure!(!(c.patchmatch && c.slanted_refine), "dense configuration: choose one plane refinement method");
    anyhow::ensure!((1..=16).contains(&c.slanted_iterations), "dense configuration: slanted_iterations must be in 1..16");
    let need = |condition: bool, message: &str| -> anyhow::Result<()> {
        if condition {
            Ok(())
        } else {
            Err(anyhow!("dense configuration: {message}"))
        }
    };
    let nonempty = !c.sizes.is_empty()
        && !c.windows.is_empty()
        && !c.aggregates.is_empty()
        && !c.passes.is_empty()
        && !c.band_first.is_empty()
        && !c.tolerances.is_empty()
        && !c.min_votes.is_empty()
        && c.stretch_percentiles.len() == 2;
    need(nonempty, "per-level lists must not be empty")?;
    need((1..=5).contains(&c.sizes.len()) && c.sizes.iter().all(|s| (32..=4096).contains(s)), "sizes must be 1..5 values in 32..4096")?;
    need(c.sizes.windows(2).all(|w| w[0] <= w[1]), "sizes must increase")?;
    need((0.0..=4.0).contains(&c.match_sharpening), "match_sharpening must be in 0..4")?;
    need((0.0..=1.0).contains(&c.peak_min_margin), "peak_min_margin must be finite and in 0..1")?;
    need((0..=64).contains(&c.patchmatch_iterations), "patchmatch_iterations must be in 0..64")?;
    need((-2.0..=1.0).contains(&c.patchmatch_normal_agreement), "patchmatch_normal_agreement must be in -2..1")?;
    need(2 <= c.best_of && c.best_of <= c.neighbours && c.neighbours <= 16, "need 2 <= best_of <= neighbours <= 16")?;
    need(!c.coherent_band || (c.neighbours <= 8 && c.best_of <= 4), "coherent_band requires neighbours <= 8 and best_of <= 4")?;
    need(0.0 < c.minimum_angle && c.minimum_angle < c.maximum_angle && c.maximum_angle < 90.0, "invalid neighbour angles")?;
    need((16..=512).contains(&c.planes), "planes must be 16..512")?;
    need(c.windows.iter().all(|w| w % 2 == 1 && (3..=31).contains(w)), "windows must be odd, 3..31")?;
    need(c.passes.iter().all(|p| (1..=4).contains(p)), "passes must be 1..4")?;
    need(c.band_first.iter().chain([&c.band_later]).all(|b| (2..=32).contains(b)), "band values must be in 2..32")?;
    need(
        0.0 < c.min_score && c.min_score < 1.0 && 0.0 < c.hull_front_min_score && c.hull_front_min_score < 1.0,
        "scores must be in (0,1)",
    )?;
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
    need((0 <= c.hull_front_level && c.hull_front_level < c.sizes.len() as i64) || !c.hull_front, "hull_front_level outside pyramid")?;
    need((0..=3).contains(&c.fused_passes) && (2..=32).contains(&c.fused_band), "invalid fused pass values")?;
    need((0.0..=3.0).contains(&c.rim_fraction) && c.truncation_voxels >= 1.0, "invalid fusion values")?;
    need((0.0..=10.0).contains(&c.truncation_noise), "truncation_noise must be in 0..10")?;
    need(
        c.cross_check_tolerance > 0.0 && c.cross_check_tolerance <= 0.05 && (1..=16).contains(&c.cross_check_votes),
        "invalid cross_check values",
    )?;
    need((0..=200).contains(&c.mask_grow_close), "mask_grow_close must be 0..200")?;
    need((0..=2).contains(&c.silhouette_first), "silhouette_first must be 0, 1 or 2")?;
    need(
        (0.0..=32.0).contains(&c.mesh_hull_correction) && (0.0..=10.0).contains(&c.mesh_hull_correction_gate),
        "invalid mesh_hull_correction values",
    )?;
    need(c.behind_voxels >= c.truncation_voxels && (0.0..=1.0).contains(&c.behind_weight), "invalid inside vote")?;
    need(0.0 < c.free_weight && c.free_weight <= 1.0, "free_weight must be in (0,1]")?;
    need(c.mesh_smooth > 0.0 && c.mesh_confidence_cap > 0.0 && (0..=100).contains(&c.mesh_taubin_cycles), "invalid mesh values")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> anyhow::Result<DenseConfig> {
        build(None, &items.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn overrides_and_levels() {
        let config = set(&["grid=128", "sizes=64,128", "repair-masks=false", "windows=5", "min_score=0.6"]).unwrap();
        assert_eq!((config.grid, config.sizes.clone(), config.repair_masks), (128, vec![64, 128], false));
        assert_eq!(DenseConfig::level(&config.windows, 3), 5);
        assert_eq!(config.min_score, 0.6);
        assert_eq!(set(&[]).unwrap(), DenseConfig::default());
    }

    #[test]
    fn rejects_unknown_and_invalid() {
        assert!(set(&["coherent_band=true", "neighbours=9"]).is_err());
        assert!(set(&["coherent_band=true", "best_of=5"]).is_err());
        assert!(set(&["coherent_band=true"]).unwrap().coherent_band);
        assert!(set(&["nonsense=1"]).is_err());
        assert!(set(&["windows=4"]).is_err());
        assert!(set(&["best_of=9", "neighbours=4"]).is_err());
        assert!(set(&["repair_masks=maybe"]).is_err());
        assert!(set(&["grid"]).is_err());
        assert!(set(&["peak_min_margin=-0.01"]).is_err());
        assert!(set(&["peak_min_margin=1.01"]).is_err());
        let mut invalid = DenseConfig { peak_min_margin: f64::NAN, ..Default::default() };
        assert!(validate(&invalid).is_err());
        invalid.peak_min_margin = f64::INFINITY;
        assert!(validate(&invalid).is_err());
        assert_eq!(set(&["peak_min_margin=0.02"]).unwrap().peak_min_margin, 0.02);
    }

    #[test]
    fn patchmatch_requires_the_explicit_research_build() {
        let result = set(&["patchmatch=true"]);
        assert_eq!(result.is_ok(), cfg!(feature = "research-patchmatch"));
        if !cfg!(feature = "research-patchmatch") {
            assert!(result.unwrap_err().to_string().contains("excluded from this build"));
        }
    }

    #[test]
    fn reads_a_result_file() {
        let path = std::env::temp_dir().join(format!("crisp3ds-options-{}.json", std::process::id()));
        crate::storage::write(&path, r#"{"configuration": {"grid": 320, "sizes": [256, 512], "unknown": 1}, "views": 73}"#).unwrap();
        let config = build(Some(&path), &["planes=64".to_string()]).unwrap();
        crate::storage::remove_file(&path).unwrap();
        assert_eq!((config.grid, config.sizes, config.planes), (320, vec![256, 512], 64));
    }
}
