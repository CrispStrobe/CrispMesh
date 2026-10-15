//! Whether a provider can run here: its external programs are looked for at
//! the given locations and asked for their version where they have one.
//!
//! This is the check a host makes before offering a provider (`crispmesh-dense
//! run --describe`, `crate::run::describe_with`); the stage itself checks
//! again when it runs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use web_time::Instant;

use serde_json::{json, Value};

use super::cameras::ALICEVISION_TOOLS;
use super::option_table;

/// Where the external programs are: the tool options of `crispmesh-dense photos`.
#[derive(Clone, Debug, Default)]
pub struct ToolLocations {
    /// AliceVision install prefix or wrapper script.
    pub alicevision: Option<PathBuf>,
    /// The COLMAP executable or a wrapper; `None`: `colmap` on the `PATH`.
    pub colmap: Option<PathBuf>,
    /// Interpreter for `.py` wrappers.
    pub python: Option<String>,
    pub sam_python: Option<String>,
    pub sam_source: Option<PathBuf>,
    pub sam_checkpoint: Option<PathBuf>,
    /// Checkout with `scripts/turntable_mesh/segment.py`.
    pub sam_repository: Option<PathBuf>,
    /// Model directory of the native `sam` provider.
    pub sam_model: Option<PathBuf>,
    /// ONNX Runtime's shared library for it.
    pub sam_runtime: Option<PathBuf>,
}

impl ToolLocations {
    /// From command-line words of the photos stage (`--colmap PATH`, `--alicevision=PATH`, ...; other words
    /// are ignored), then from the options' environment variables. `environment` looks up a variable.
    pub fn from_words(words: &[String], environment: &dyn Fn(&str) -> Option<String>) -> Self {
        let value = |flag: &str| -> Option<String> {
            let long = format!("--{flag}");
            let given = words.iter().enumerate().rev().find_map(|(n, word)| {
                if *word == long {
                    words.get(n + 1).cloned()
                } else {
                    word.strip_prefix(&long).and_then(|rest| rest.strip_prefix('=')).map(String::from)
                }
            });
            let variable = option_table::find(flag).and_then(|option| option.variable);
            given.filter(|v| !v.is_empty()).or_else(|| variable.and_then(environment).filter(|v| !v.is_empty()))
        };
        let path = |flag: &str| value(flag).map(PathBuf::from);
        ToolLocations {
            alicevision: path("alicevision"),
            colmap: path("colmap"),
            python: value("python"),
            sam_python: value("sam-python"),
            sam_source: path("sam-source"),
            sam_checkpoint: path("sam-checkpoint"),
            sam_repository: path("sam-repository"),
            sam_model: path("sam-model"),
            sam_runtime: path("sam-runtime"),
        }
    }

    /// From the environment variables alone.
    pub fn from_environment() -> Self {
        Self::from_words(&[], &|name| std::env::var(name).ok())
    }
}

/// The answer for one provider.
#[derive(Clone, Debug, PartialEq)]
pub struct Availability {
    pub available: bool,
    /// Why not, in a sentence a person can act on.
    pub reason: Option<String>,
    /// Version of the external program, where it could be read; this crate's for a provider without one.
    pub version: Option<String>,
}

impl Availability {
    fn yes(version: Option<String>) -> Self {
        Availability { available: true, reason: None, version }
    }

    fn no(reason: impl Into<String>) -> Self {
        Availability { available: false, reason: Some(reason.into()), version: None }
    }

    pub fn to_json(&self) -> Value {
        json!({"available": self.available, "reason": self.reason, "version": self.version})
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|folder| folder.join(&file)).find(|p| p.is_file()))
}

/// A program given as a path or as a name to be found on the `PATH`.
fn program(text: &str) -> Option<PathBuf> {
    let path = Path::new(text);
    if path.components().count() > 1 || path.is_file() {
        return path.is_file().then(|| path.to_path_buf());
    }
    on_path(text)
}

/// Standard output and error of a short command, or `None` when it cannot be started or takes longer than five seconds.
fn ask(command: &[String]) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut child =
        Command::new(&command[0]).args(&command[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    Some(format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}

/// `3.9.1` from `COLMAP 3.9.1 -- Structure-from-Motion ...`.
pub fn colmap_version(banner: &str) -> Option<String> {
    let rest = banner.split("COLMAP").nth(1)?.trim_start();
    let version: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let version = version.trim_end_matches('.');
    version.chars().next().is_some_and(|c| c.is_ascii_digit()).then(|| version.to_string())
}

/// The version in a library's file name: `3.4` from `libaliceVision_camera.3.4.dylib`, `3.4.0` from `libaliceVision_camera.so.3.4.0`.
pub fn library_version(file_name: &str) -> Option<String> {
    if !file_name.contains("aliceVision_") {
        return None;
    }
    let mut best: Option<String> = None;
    let mut current = String::new();
    for c in file_name.chars().chain(std::iter::once('/')) {
        if c.is_ascii_digit() || (c == '.' && !current.is_empty()) {
            current.push(c);
        } else {
            let found = current.trim_end_matches('.');
            if found.contains('.') && best.as_ref().is_none_or(|b| found.len() > b.len()) {
                best = Some(found.to_string());
            }
            current.clear();
        }
    }
    best
}

/// AliceVision has no `--version`: the version is read from the names of its libraries.
fn alicevision_version(prefix: &Path) -> Option<String> {
    let mut best: Option<String> = None;
    for folder in ["lib", "lib64", "bin"] {
        let Ok(entries) = std::fs::read_dir(prefix.join(folder)) else { continue };
        for entry in entries.flatten() {
            if let Some(version) = library_version(&entry.file_name().to_string_lossy()) {
                if best.as_ref().is_none_or(|b| version.len() > b.len()) {
                    best = Some(version);
                }
            }
        }
    }
    best
}

fn alicevision(tools: &ToolLocations) -> Availability {
    let Some(location) = &tools.alicevision else {
        return Availability::no("AliceVision is an external program: give its install prefix or a wrapper script (--alicevision)");
    };
    if location.is_dir() {
        let absent: Vec<String> = ALICEVISION_TOOLS
            .iter()
            .map(|tool| format!("aliceVision_{tool}{}", std::env::consts::EXE_SUFFIX))
            .filter(|file| !location.join("bin").join(file).is_file())
            .collect();
        if !absent.is_empty() {
            return Availability::no(format!("{}: no bin/{} there", location.display(), absent.join(", bin/")));
        }
        return Availability::yes(alicevision_version(location));
    }
    if !location.is_file() {
        return Availability::no(format!("{}: not found", location.display()));
    }
    if location.extension().is_some_and(|e| e == "py") && tools.python.as_deref().and_then(program).is_none() {
        return Availability::no(format!("{} is a Python wrapper: give the interpreter (--python)", location.display()));
    }
    // A wrapper: what it starts is not known here.
    Availability::yes(None)
}

fn colmap(tools: &ToolLocations) -> Availability {
    let command: Vec<String> = match &tools.colmap {
        Some(location) if !location.is_file() => return Availability::no(format!("{}: not found", location.display())),
        Some(location) if location.extension().is_some_and(|e| e == "py") => match tools.python.as_deref().and_then(program) {
            Some(python) => vec![python.to_string_lossy().to_string(), location.to_string_lossy().to_string()],
            None => return Availability::no(format!("{} is a Python wrapper: give the interpreter (--python)", location.display())),
        },
        Some(location) => vec![location.to_string_lossy().to_string()],
        None => match on_path("colmap") {
            Some(found) => vec![found.to_string_lossy().to_string()],
            None => return Availability::no("COLMAP is an external program: no colmap on the PATH; give the executable (--colmap)"),
        },
    };
    let mut asking = command;
    asking.push("help".to_string());
    Availability::yes(ask(&asking).as_deref().and_then(colmap_version))
}

fn external_sam(tools: &ToolLocations) -> Availability {
    let mut missing = Vec::new();
    match tools.sam_python.as_deref() {
        None => missing.push("an interpreter with PyTorch (--sam-python)".to_string()),
        Some(python) if program(python).is_none() => missing.push(format!("{python}: no such interpreter")),
        Some(_) => {}
    }
    match &tools.sam_source {
        None => missing.push("the SAM 2 source (--sam-source)".to_string()),
        Some(source) if !source.is_dir() => missing.push(format!("{}: not a directory", source.display())),
        Some(_) => {}
    }
    match &tools.sam_checkpoint {
        None => missing.push("a checkpoint (--sam-checkpoint)".to_string()),
        Some(checkpoint) if !checkpoint.is_file() => missing.push(format!("{}: not found", checkpoint.display())),
        Some(_) => {}
    }
    match &tools.sam_repository {
        None => missing.push("the checkout with scripts/turntable_mesh/segment.py (--sam-repository)".to_string()),
        Some(repository) if !repository.join("scripts/turntable_mesh/segment.py").is_file() => {
            missing.push(format!("{}: no scripts/turntable_mesh/segment.py there", repository.display()));
        }
        Some(_) => {}
    }
    if missing.is_empty() {
        // The interpreter is not started: importing PyTorch takes seconds.
        Availability::yes(None)
    } else {
        Availability::no(format!("SAM 2.1 runs in an external Python: missing {}", missing.join("; ")))
    }
}

/// Whether the provider `name` of `module` (`masks` or `cameras`) can run with these tool locations.
/// Providers without an external program are available and report this crate's version.
pub fn check(module: &str, name: &str, tools: &ToolLocations) -> Availability {
    match (module, name) {
        ("cameras", "alicevision") => alicevision(tools),
        ("cameras", "colmap") => colmap(tools),
        ("masks", "external-sam") => external_sam(tools),
        ("masks", "sam") => match super::sam::provider::readiness(tools.sam_model.as_deref(), tools.sam_runtime.as_deref()) {
            Ok(model) => Availability::yes(Some(model)),
            Err(reason) => Availability::no(format!("SAM 2.1 in this process: {reason}")),
        },
        _ if super::providers::PROVIDERS.iter().any(|p| p.module == module && p.name == name) => {
            Availability::yes(Some(env!("CARGO_PKG_VERSION").to_string()))
        }
        _ => Availability::no(format!("no {module} provider named {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_banners_and_library_names() {
        assert_eq!(colmap_version("COLMAP 3.9.1 -- Structure-from-Motion and Multi-View Stereo\n").as_deref(), Some("3.9.1"));
        assert_eq!(colmap_version("\nCOLMAP 4.0 (Commit abc)").as_deref(), Some("4.0"));
        assert_eq!(colmap_version("usage: something else"), None);
        assert_eq!(library_version("libaliceVision_camera.3.4.dylib").as_deref(), Some("3.4"));
        assert_eq!(library_version("libaliceVision_camera.so.3.4.0").as_deref(), Some("3.4.0"));
        assert_eq!(library_version("libaliceVision_camera.dylib"), None);
        assert_eq!(library_version("libboost_system.1.85.dylib"), None);
    }

    #[test]
    fn locations_come_from_words_then_from_the_environment() {
        let words: Vec<String> =
            ["--masks", "threshold", "--colmap", "/opt/colmap", "--sam-python=/venv/python", "--threads", "4"].map(String::from).to_vec();
        let environment = |name: &str| match name {
            "CRISPMESH_COLMAP" => Some("/elsewhere".to_string()),
            "CRISPMESH_ALICEVISION" => Some("/opt/av".to_string()),
            _ => None,
        };
        let tools = ToolLocations::from_words(&words, &environment);
        assert_eq!(tools.colmap.as_deref(), Some(Path::new("/opt/colmap")));
        assert_eq!(tools.alicevision.as_deref(), Some(Path::new("/opt/av")));
        assert_eq!((tools.sam_python.as_deref(), tools.python.as_deref()), (Some("/venv/python"), None));
    }

    #[test]
    fn missing_programs_are_named_and_native_providers_are_available() {
        let folder = std::env::temp_dir().join(format!("crispmesh-availability-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("prefix/bin")).unwrap();
        std::fs::create_dir_all(folder.join("prefix/lib")).unwrap();
        let none = ToolLocations::default();
        assert_eq!(check("cameras", "turntable", &none), Availability::yes(Some(env!("CARGO_PKG_VERSION").to_string())));
        assert!(check("masks", "threshold", &none).available && check("cameras", "import", &none).available);
        assert!(!check("cameras", "nothing", &none).available);
        let unset = check("cameras", "alicevision", &none);
        assert!(!unset.available && unset.reason.unwrap().contains("--alicevision"));
        let sam = check("masks", "external-sam", &none);
        assert!(!sam.available && sam.reason.unwrap().contains("--sam-checkpoint"));

        let mut tools = ToolLocations { alicevision: Some(folder.join("prefix")), ..Default::default() };
        let partial = check("cameras", "alicevision", &tools);
        assert!(!partial.available && partial.reason.unwrap().contains("aliceVision_cameraInit"));
        for tool in ALICEVISION_TOOLS {
            std::fs::write(folder.join("prefix/bin").join(format!("aliceVision_{tool}{}", std::env::consts::EXE_SUFFIX)), b"").unwrap();
        }
        std::fs::write(folder.join("prefix/lib/libaliceVision_sfm.so.3.4.0"), b"").unwrap();
        std::fs::write(folder.join("prefix/lib/libaliceVision_sfm.so.3"), b"").unwrap();
        assert_eq!(check("cameras", "alicevision", &tools), Availability::yes(Some("3.4.0".to_string())));

        tools.colmap = Some(folder.join("no-colmap"));
        let absent = check("cameras", "colmap", &tools);
        assert!(!absent.available && absent.reason.unwrap().contains("not found"));
        std::fs::write(folder.join("colmap.py"), b"").unwrap();
        tools.colmap = Some(folder.join("colmap.py"));
        assert!(check("cameras", "colmap", &tools).reason.unwrap().contains("--python"));
        assert_eq!(check("cameras", "colmap", &tools).to_json()["available"], false);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
