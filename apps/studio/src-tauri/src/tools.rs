//! The photos start as the app offers it: the crate's description of providers, their
//! options, and whether each can run here, shaped for the form; and how the external
//! programs behind some providers (AliceVision, COLMAP, SAM 2.1 in a Python interpreter)
//! are handed to the crate.
//!
//! Everything about providers comes from `crates/dense` (`run::describe_with`,
//! `photos::availability`, `photos::option_table`). What this module adds is the app's
//! own rule: the web view never names a program or a place. Where the programs are comes
//! from the saved settings (or the environment variables the crate reads); a start request
//! may only choose providers and give values to their tuning options.

use std::path::PathBuf;

use crispmesh_dense::photos::{availability, option_table};
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::Tools;

/// Kinds of the option table that name a place or a program.
const PLACE_KINDS: [&str; 4] = ["path", "directory", "executable", "path_list"];

/// What the app's own editions prefer when it can run: both are computed inside the app.
const PREFERRED: [(&str, &str); 2] = [("masks", "threshold"), ("cameras", "turntable")];

const CONTAINED: &str = "This edition of the app cannot start other programs (App Sandbox, or a phone), so providers that need one are not available here.";

/// True inside the macOS App Sandbox.
pub fn sandboxed() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// True where the app cannot start programs from the disk: the App Sandbox and phones.
pub fn contained() -> bool {
    cfg!(any(target_os = "ios", target_os = "android")) || sandboxed()
}

/// Whether an option of the photos stage may not come from a start request: it says where
/// something is or what to run, or it is one of the stage's own arguments.
pub fn reserved(name: &str) -> bool {
    let name = name.trim_start_matches("--");
    let name = name.strip_prefix("no-").filter(|rest| option_table::find(rest).is_some()).unwrap_or(name);
    match option_table::find(name) {
        Some(option) => {
            PLACE_KINDS.contains(&option.kind)
                || option.kind == "provider"
                || option.scope == "run"
                || option.scope == "tools"
                // Environment for an external program: part of what is run.
                || option.flag == "alicevision-env"
        }
        // Not an option of the table: arguments of the command line itself.
        None => matches!(name, "list-providers" | "describe" | "help"),
    }
}

/// Refuses option words from a start request that name a place or a program.
pub fn check_tokens(tokens: &[String]) -> Result<(), String> {
    for token in tokens {
        if token.contains('\0') {
            return Err("photo_options: invalid character".into());
        }
        if !token.starts_with("--") {
            continue;
        }
        let name = token.split('=').next().unwrap_or_default();
        if reserved(name) {
            return Err(format!("photo_options: {name} cannot be set by a run request; it comes from the form's own fields and the Tools settings"));
        }
    }
    Ok(())
}

/// The tool locations from the settings as words of the photos command line: those the
/// providers named need, or all of them with `None`.
pub fn location_words(providers: Option<(&str, &str)>, tools: &Tools, repo: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut push = |flag: &str, value: &str| {
        if !value.is_empty() {
            words.push(flag.to_string());
            words.push(value.to_string());
        }
    };
    let wanted = |module: &str, name: &str| match providers {
        None => true,
        Some((masks, cameras)) => (module == "masks" && masks == name) || (module == "cameras" && cameras == name),
    };
    if wanted("cameras", "alicevision") {
        push("--alicevision", &tools.alicevision.value);
        push("--alicevision-library-path", &tools.alicevision_library_path.value);
    }
    if wanted("cameras", "colmap") {
        push("--colmap", &tools.colmap.value);
    }
    if wanted("masks", "external-sam") {
        push("--sam-python", &tools.sam_python.value);
        push("--sam-source", &tools.sam_source.value);
        push("--sam-checkpoint", &tools.sam_checkpoint.value);
        push("--sam-repository", repo);
    }
    words
}

/// Path fields of the form that belong to one provider: data of the run, not tools.
fn inputs_of(module: &str, provider: &str) -> Vec<Value> {
    match (module, provider) {
        ("masks", "import") => vec![json!({
            "key": "masks_import", "label": "Masks folder", "kind": "folder",
            "help": "One 8-bit PNG per photo, white object, named like the photo or capture_NNNN.png in capture order.",
        })],
        ("cameras", "import") => vec![json!({
            "key": "cameras_import", "label": "Camera solution", "kind": "file",
            "help": "An AliceVision .sfm file, or a COLMAP model folder (type its path).",
        })],
        ("cameras", "markers") => vec![json!({
            "key": "markers_mat", "label": "Marker mat description", "kind": "file",
            "help": "The file that `crispmesh-dense mat` wrote for the printed mat under the object.",
        })],
        _ => Vec::new(),
    }
}

fn tunable(settings: &mut Value) {
    if let Some(rows) = settings.as_array_mut() {
        rows.retain(|row| row["name"].as_str().is_some_and(|name| !reserved(name)));
    }
}

/// The crate's start points for the "New run" form, with the tools at their configured
/// places: every provider with `available`, `reason`, `version`, its tunable `settings`
/// and its path `inputs`. Options that name places are taken out, and a provider that
/// needs an external program is unavailable where the app cannot start one.
pub fn start_points(tools: &Tools, repo: &str, contained: bool) -> Value {
    let words = if contained { Vec::new() } else { location_words(None, tools, repo) };
    let mut points = crispmesh_dense::run::describe_with(&words)["start_points"].clone();
    for point in points.as_array_mut().into_iter().flatten() {
        for choice in point["providers"].as_array_mut().into_iter().flatten() {
            let module = choice["module"].as_str().unwrap_or_default().to_string();
            tunable(&mut choice["settings"]);
            for option in choice["options"].as_array_mut().into_iter().flatten() {
                let id = option["id"].as_str().unwrap_or_default().to_string();
                let external = option["external"].as_array().is_some_and(|programs| !programs.is_empty());
                if contained && external {
                    option["available"] = json!(false);
                    option["reason"] = json!(CONTAINED);
                    option["version"] = Value::Null;
                }
                tunable(&mut option["settings"]);
                option["inputs"] = json!(inputs_of(&module, &id));
            }
            let usable = |id: &str| choice["options"].as_array().into_iter().flatten().any(|option| option["id"] == id && option["available"] == true);
            let preferred = PREFERRED.iter().find(|(name, _)| *name == module).map(|(_, id)| *id).filter(|id| usable(id));
            let own = choice["default"].as_str().filter(|id| usable(id)).map(str::to_string);
            let first = choice["options"].as_array().into_iter().flatten().find(|option| option["available"] == true).and_then(|option| option["id"].as_str()).map(str::to_string);
            if let Some(id) = preferred.map(str::to_string).or(own).or(first) {
                choice["default"] = json!(id);
            }
        }
        if let Some(groups) = point["option_groups"].as_array_mut() {
            groups.retain(|group| group["id"] != "tools");
            for group in groups.iter_mut() {
                tunable(&mut group["settings"]);
            }
            groups.retain(|group| group["settings"].as_array().is_some_and(|rows| !rows.is_empty()));
        }
    }
    points
}

#[derive(Debug, Serialize)]
pub struct Checked {
    pub ok: bool,
    /// One line: the version, or what is wrong.
    pub summary: String,
    /// What was looked at.
    pub detail: String,
}

/// Asks the crate whether an external tool can run, with the settings' locations.
/// `tool` is `alicevision`, `colmap` or `sam`.
pub fn check(tool: &str, tools: &Tools, repo: &str) -> Checked {
    let (module, provider, name) = match tool {
        "alicevision" => ("cameras", "alicevision", "AliceVision"),
        "colmap" => ("cameras", "colmap", "COLMAP"),
        "sam" => ("masks", "external-sam", "SAM 2.1"),
        other => return Checked { ok: false, summary: format!("There is no tool named {other}."), detail: String::new() },
    };
    if contained() {
        return Checked { ok: false, summary: CONTAINED.into(), detail: String::new() };
    }
    let words = location_words(None, tools, repo);
    let locations = availability::ToolLocations::from_words(&words, &|variable| std::env::var(variable).ok());
    let answer = availability::check(module, provider, &locations);
    let detail = words.chunks(2).map(|pair| pair.join(" ")).collect::<Vec<_>>().join("\n");
    Checked {
        ok: answer.available,
        summary: match (answer.available, answer.version, answer.reason) {
            (true, Some(version), _) => format!("{name} {version} found."),
            (true, None, _) => format!("{name} found; it did not say which version it is."),
            (false, _, reason) => reason.unwrap_or_else(|| format!("{name} cannot be used here.")),
        },
        detail,
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Calibration {
    /// Shown in the list: the file name, and the camera or lens it names if it says so.
    pub label: String,
    pub path: String,
}

/// Lens calibration files (`crispmesh_lens_calibration_v1`) in the usual places.
pub fn calibrations(folders: &[PathBuf]) -> Vec<Calibration> {
    let mut found = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|e| e == "json")).collect();
        files.sort();
        for path in files {
            // Calibration files are small; anything large is something else.
            if std::fs::metadata(&path).map(|meta| meta.len() > 256 * 1024).unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
            if value["schema"] != "crispmesh_lens_calibration_v1" {
                continue;
            }
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let about = ["camera", "lens", "name", "description"].iter().find_map(|key| value[*key].as_str()).unwrap_or_default();
            let label = if about.is_empty() { name } else { format!("{name} ({about})") };
            let path = std::fs::canonicalize(&path).unwrap_or(path).to_string_lossy().into_owned();
            if !found.iter().any(|known: &Calibration| known.path == path) {
                found.push(Calibration { label, path });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Source, Value as Setting};

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("crispmesh-studio-tools-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::canonicalize(folder).unwrap()
    }

    fn tools() -> Tools {
        let empty = || Setting { value: String::new(), source: Source::Default };
        Tools {
            alicevision: Setting { value: "/definitely/not/alicevision".into(), source: Source::Setting },
            alicevision_library_path: empty(),
            colmap: Setting { value: "/definitely/not/colmap".into(), source: Source::Setting },
            sam_python: empty(),
            sam_source: empty(),
            sam_checkpoint: empty(),
        }
    }

    fn provider<'a>(points: &'a Value, module: &str, id: &str) -> &'a Value {
        let photos = points.as_array().unwrap().iter().find(|point| point["id"] == "photos").expect("a photos start");
        let choice = photos["providers"].as_array().unwrap().iter().find(|choice| choice["module"] == module).expect("the module");
        choice["options"].as_array().unwrap().iter().find(|option| option["id"] == id).unwrap_or_else(|| panic!("no {module} provider {id}"))
    }

    fn default_of(points: &Value, module: &str) -> String {
        let photos = points.as_array().unwrap().iter().find(|point| point["id"] == "photos").unwrap();
        photos["providers"].as_array().unwrap().iter().find(|choice| choice["module"] == module).unwrap()["default"].as_str().unwrap().to_string()
    }

    #[test]
    fn the_form_gets_providers_with_availability_and_only_tunable_options() {
        let points = start_points(&tools(), "", false);
        // Inside the crate: always there, and the app's defaults.
        for (module, id) in [("masks", "threshold"), ("masks", "import"), ("cameras", "turntable"), ("cameras", "markers"), ("cameras", "import")] {
            assert_eq!(provider(&points, module, id)["available"], true, "{module} {id}");
        }
        assert_eq!(default_of(&points, "masks"), "threshold");
        assert_eq!(default_of(&points, "cameras"), "turntable");
        // External, at places that do not exist: listed, unavailable, with the crate's reason.
        for (module, id) in [("cameras", "alicevision"), ("cameras", "colmap"), ("masks", "external-sam")] {
            let option = provider(&points, module, id);
            assert_eq!(option["available"], false, "{module} {id}");
            assert!(option["reason"].as_str().is_some_and(|reason| !reason.is_empty()), "{module} {id}");
        }
        // No option that names a place or a program reaches the form.
        let photos = points.as_array().unwrap().iter().find(|point| point["id"] == "photos").unwrap();
        let mut names = Vec::new();
        for choice in photos["providers"].as_array().unwrap() {
            names.extend(choice["settings"].as_array().unwrap().iter().map(|row| row["name"].as_str().unwrap().to_string()));
            for option in choice["options"].as_array().unwrap() {
                names.extend(option["settings"].as_array().unwrap().iter().map(|row| row["name"].as_str().unwrap().to_string()));
            }
        }
        for group in photos["option_groups"].as_array().unwrap() {
            assert_ne!(group["id"], "tools");
            names.extend(group["settings"].as_array().unwrap().iter().map(|row| row["name"].as_str().unwrap().to_string()));
        }
        assert!(names.iter().any(|name| name == "turntable-features") && names.iter().any(|name| name == "colmap-matching"), "{names:?}");
        for name in &names {
            let option = option_table::find(name).unwrap();
            assert!(!PLACE_KINDS.contains(&option.kind), "{name} names a place");
        }
        for place in ["alicevision", "colmap", "sam-python", "sam-checkpoint", "markers-mat", "turntable-matches", "python", "alicevision-env"] {
            assert!(!names.iter().any(|name| name == place), "{place} reached the form");
        }
        // Data of a run that is a path has a field of its own.
        assert_eq!(provider(&points, "cameras", "markers")["inputs"][0]["key"], "markers_mat");
        assert_eq!(provider(&points, "cameras", "import")["inputs"][0]["key"], "cameras_import");
        assert_eq!(provider(&points, "masks", "import")["inputs"][0]["key"], "masks_import");
        assert_eq!(provider(&points, "masks", "threshold")["inputs"], json!([]));
    }

    #[test]
    fn nothing_external_is_available_where_programs_cannot_be_started() {
        let points = start_points(&tools(), "", true);
        for (module, id) in [("cameras", "alicevision"), ("cameras", "colmap"), ("masks", "external-sam")] {
            let option = provider(&points, module, id);
            assert_eq!(option["available"], false);
            assert_eq!(option["reason"], CONTAINED);
        }
        for (module, id) in [("masks", "threshold"), ("cameras", "turntable"), ("cameras", "markers"), ("cameras", "import")] {
            assert_eq!(provider(&points, module, id)["available"], true);
        }
        assert_eq!((default_of(&points, "masks"), default_of(&points, "cameras")), ("threshold".to_string(), "turntable".to_string()));
    }

    #[test]
    fn locations_come_from_the_settings_for_the_chosen_providers_only() {
        let mut tools = tools();
        tools.alicevision.value = "/opt/av".into();
        tools.alicevision_library_path.value = "/opt/homebrew/lib".into();
        tools.sam_python.value = "/venv/bin/python".into();
        assert_eq!(location_words(Some(("threshold", "alicevision")), &tools, "/repo"), ["--alicevision", "/opt/av", "--alicevision-library-path", "/opt/homebrew/lib"]);
        assert_eq!(location_words(Some(("threshold", "turntable")), &tools, "/repo"), Vec::<String>::new());
        assert_eq!(location_words(Some(("external-sam", "colmap")), &tools, "/repo"), ["--colmap", "/definitely/not/colmap", "--sam-python", "/venv/bin/python", "--sam-repository", "/repo"]);
        assert_eq!(location_words(None, &tools, "").len(), 8);
    }

    #[test]
    fn a_run_request_cannot_name_programs_or_places() {
        for word in [
            "--alicevision", "--colmap=/bin/sh", "--sam-python", "--python", "--output", "--photos", "--events", "--calibration", "--masks", "--cameras",
            "--stop-after", "--sam-checkpoint", "--sam-pythonpath", "--sam-repository", "--alicevision-library-path", "--alicevision-env",
            "--alicevision-sensor-database", "--markers-mat", "--turntable-matches", "--list-providers",
        ] {
            assert!(check_tokens(&[word.to_string(), "x".to_string()]).is_err(), "{word} was accepted");
        }
        assert!(check_tokens(&["--colmap-matching".into(), "ring".into(), "--no-sam-multimask".into(), "--turntable-span".into(), "6".into()]).is_ok());
        // Values are not flags, whatever they look like after the first two characters.
        assert!(check_tokens(&["--alicevision-describer-preset".into(), "high".into()]).is_ok());
        assert!(check_tokens(&["a\0b".into()]).is_err());
    }

    #[test]
    fn checks_report_instead_of_failing() {
        if contained() {
            return;
        }
        let colmap = check("colmap", &tools(), "");
        assert!(!colmap.ok && !colmap.summary.is_empty());
        assert!(colmap.detail.contains("--colmap /definitely/not/colmap"));
        let alicevision = check("alicevision", &tools(), "");
        assert!(!alicevision.ok && !alicevision.summary.is_empty());
        assert!(!check("sam", &tools(), "").ok);
        assert!(!check("blender", &tools(), "").ok);
    }

    #[test]
    fn calibration_files_are_found_by_their_schema() {
        let root = scratch("calibrations");
        std::fs::write(root.join("3dlf-pro.json"), r#"{"schema": "crispmesh_lens_calibration_v1", "camera": "3DLF Pro"}"#).unwrap();
        std::fs::write(root.join("plain.json"), r#"{"schema": "crispmesh_lens_calibration_v1"}"#).unwrap();
        std::fs::write(root.join("other.json"), r#"{"schema": "something else"}"#).unwrap();
        std::fs::write(root.join("broken.json"), "{").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        let found = calibrations(&[root.clone(), root.join("missing"), root.clone()]);
        assert_eq!(found.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["3dlf-pro.json (3DLF Pro)", "plain.json"]);
        assert_eq!(found[0].path, root.join("3dlf-pro.json").to_string_lossy());
    }
}
