//! The stereo stage: port of `scripts/turntable_mesh/multiscale_stereo.py`.

#[cfg(not(target_arch = "wasm32"))]
pub mod capture;
pub mod depth;
pub mod level;
pub mod levels;
pub mod matcher;
pub mod options;
#[cfg(any(test, feature = "research-patchmatch"))]
pub mod patchmatch;
pub mod planes;
pub mod previews;
pub mod run;
pub mod synthetic;

use std::path::PathBuf;

use anyhow::{anyhow, bail};

/// Command line of `crispmesh-dense stereo`, the counterpart of
/// `python -m scripts.turntable_mesh.multiscale_stereo`.
#[derive(Debug, Default)]
pub struct Arguments {
    pub inputs: PathBuf,
    pub output: PathBuf,
    pub config: Option<PathBuf>,
    pub overrides: Vec<String>,
    pub reuse_depths: Option<PathBuf>,
    pub events: Option<PathBuf>,
    pub previews: bool,
    /// Diagnostic: stop after this step (`hull`, `repair`) and write what exists.
    pub only: Option<String>,
}

pub const USAGE: &str = "usage: crispmesh-dense stereo --inputs DIR --output DIR [--config FILE] [--set KEY=VALUE]... \
[--reuse-depths FILE] [--events FILE] [--previews] [--device NAME]";

impl Arguments {
    pub fn parse(arguments: &[String]) -> anyhow::Result<Self> {
        let mut out = Arguments::default();
        let (mut inputs, mut output) = (None, None);
        let mut rest = arguments.iter();
        while let Some(flag) = rest.next() {
            let (flag, inline) = match flag.split_once('=') {
                Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
                _ => (flag.as_str(), None),
            };
            let mut value = || inline.clone().or_else(|| rest.next().cloned()).ok_or_else(|| anyhow!("{flag} needs a value\n{USAGE}"));
            match flag {
                "--inputs" => inputs = Some(PathBuf::from(value()?)),
                "--output" => output = Some(PathBuf::from(value()?)),
                "--config" => out.config = Some(PathBuf::from(value()?)),
                "--set" => out.overrides.push(value()?),
                "--reuse-depths" => out.reuse_depths = Some(PathBuf::from(value()?)),
                "--events" => out.events = Some(PathBuf::from(value()?)),
                "--only" => out.only = Some(value()?),
                // Accepted so the Python driver's command line works unchanged; the adapter is chosen by wgpu.
                "--device" => drop(value()?),
                "--previews" => out.previews = true,
                other => bail!("unknown argument {other}\n{USAGE}"),
            }
        }
        out.inputs = inputs.ok_or_else(|| anyhow!("--inputs is required\n{USAGE}"))?;
        out.output = output.ok_or_else(|| anyhow!("--output is required\n{USAGE}"))?;
        Ok(out)
    }
}

/// Entry point of the `stereo` subcommand.
#[cfg(not(target_arch = "wasm32"))]
pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    let arguments = Arguments::parse(arguments)?;
    let config = options::build(arguments.config.as_deref(), &arguments.overrides)?;
    run::run(&arguments, &config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_python_command_line() {
        let words = "--inputs in --output out --device mps --set grid=96 --set=sizes=64,128 --previews --events run/events.jsonl";
        let parsed = Arguments::parse(&words.split(' ').map(String::from).collect::<Vec<_>>()).unwrap();
        assert_eq!(parsed.inputs, PathBuf::from("in"));
        assert_eq!(parsed.overrides, ["grid=96", "sizes=64,128"]);
        assert!(parsed.previews && parsed.reuse_depths.is_none());
        assert_eq!(parsed.events, Some(PathBuf::from("run/events.jsonl")));
        assert!(Arguments::parse(&["--output".to_string(), "x".to_string()]).is_err());
        assert!(Arguments::parse(&["--bogus".to_string()]).is_err());
    }
}
