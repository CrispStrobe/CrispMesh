//! The built-in engine: `crates/dense` running inside this process.
//!
//! This is the "Local (desktop shell)" row of `docs/ENGINE-CONTRACT.md`. A run is a
//! directory under the runs folder; the pipeline runs on a worker thread through the
//! crate's library API with a cancel flag; `events.jsonl` on disk is the only source of
//! truth, and the web view reads it line by line exactly as it would from an HTTP engine.
//! Nothing here starts a process.
//!
//! Everything the web view can name is a path *relative* to the runs folder or the data
//! folder and is checked to stay inside it. The one exception is a path the user picked
//! in a native dialog during this session, which is remembered as granted.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crispmesh_dense::run::RunOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const EVENTS: &str = "events.jsonl";
/// Written into a run directory by this process, so that a later start can tell a run it
/// lost (app quit or crashed) from one that somebody else is still writing.
const MARKER: &str = "studio-run.json";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub status: String,
    pub started: Option<f64>,
    pub stage: Option<String>,
    pub stage_fraction: f64,
    pub events: usize,
}

#[derive(Debug, Serialize)]
pub struct DataEntry {
    pub name: String,
    pub directory: bool,
    pub inputs: bool,
    /// A folder with at least three images directly in it.
    pub photos: bool,
    /// A lens calibration file.
    pub calibration: bool,
}

#[cfg(unix)]
fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `name` is a valid C string and `stats` a properly sized, writable struct.
    if unsafe { libc::statvfs(name.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    Some(stats.f_bavail as u64 * stats.f_frsize as u64)
}

#[cfg(not(unix))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// Whether a folder has at least three images directly in it. Looks at a bounded number of entries.
fn holds_photos(folder: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else { return false };
    let image = |path: &Path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| ["jpg", "jpeg", "png", "tif", "tiff"].contains(&extension.to_ascii_lowercase().as_str()))
    };
    entries.flatten().take(2000).filter(|entry| image(&entry.path())).take(3).count() == 3
}

/// Whether a file is a lens calibration: a small JSON file that says so.
fn is_calibration(path: &Path) -> bool {
    if path.extension().is_none_or(|extension| extension != "json") || std::fs::metadata(path).map(|meta| meta.len() > 256 * 1024).unwrap_or(true) {
        return false;
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|value| value["schema"] == "crispmesh_lens_calibration_v1")
}

#[derive(Debug, Serialize)]
pub struct DataListing {
    pub path: String,
    pub entries: Vec<DataEntry>,
}

#[derive(Debug, Serialize)]
pub struct EventPage {
    pub events: Vec<Value>,
    pub next: usize,
}

/// Body of a start request: the same shape as `POST /api/runs` of the HTTP engine.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct StartBody {
    pub name: Option<String>,
    pub inputs: Option<String>,
    pub scene: Option<String>,
    pub prepared: Option<String>,
    pub raw_masks: Option<String>,
    /// Folder of turntable photos: masks and cameras are made first.
    pub photos: Option<String>,
    /// Lens calibration file for the photos start.
    pub calibration: Option<String>,
    /// Chosen provider per module of the photos start: `{"masks": "threshold", "cameras": "alicevision"}`.
    pub providers: Map<String, Value>,
    /// Folder of ready-made masks, for the `import` masks provider.
    pub masks_import: Option<String>,
    /// Existing camera solution (file or folder), for the `import` cameras provider.
    pub cameras_import: Option<String>,
    /// Description of the printed marker mat (a file), for the `markers` cameras provider.
    pub markers_mat: Option<String>,
    /// Where the photos come from and under which license, as text: written next to the run
    /// (ATTRIBUTION.txt) so it stays with the result.
    pub attribution: Option<String>,
    /// Tuning options of the photos stage, one word per element. Never a place or a program.
    pub photo_options: Vec<String>,
    pub settings: Map<String, Value>,
    pub texture: bool,
    /// The neural surface and the 2D splats after the mesh (`{"quality": "standard", "settings": ["crop=600"]}`), as
    /// `crispmesh-dense run --surface` / `--splats` with their `-set` options; absent: not run.
    pub surface: Option<crispmesh_dense::run::Method>,
    pub splats: Option<crispmesh_dense::run::Method>,
    pub reference: Option<String>,
    /// Accepted and ignored: the built-in engine uses the system's GPU through wgpu.
    pub device: Option<String>,
}

struct Active {
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

#[derive(Default)]
struct Folders {
    runs: PathBuf,
    data: PathBuf,
}

#[derive(Default)]
pub struct Native {
    folders: Mutex<Folders>,
    active: Mutex<HashMap<String, Active>>,
    /// Paths the user picked in a native dialog during this session.
    granted: Mutex<HashSet<PathBuf>>,
    /// Where the external programs of the photos start are, and the crispmesh checkout.
    tools: Mutex<Option<(crate::config::Tools, String)>>,
    /// The start points with the tools looked at, kept until the tools change: looking
    /// may start a program to ask for its version.
    described: Mutex<Option<Value>>,
}

// ---------------------------------------------------------------------------------------------
// Paths

/// `relative` joined to `root`, refused unless it stays inside: no absolute paths, no `..`,
/// no drive or root components, and (when it exists) no symbolic link leading out.
pub fn contained(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let outside = || "that path is outside the folder".to_string();
    if relative.contains('\\') || relative.contains('\0') {
        return Err(outside());
    }
    let candidate = Path::new(relative);
    let mut joined = root.to_path_buf();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => joined.push(part),
            Component::CurDir => {}
            _ => return Err(outside()),
        }
    }
    if joined.exists() {
        let real_root = std::fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
        let real = std::fs::canonicalize(&joined).map_err(|error| format!("{}: {error}", joined.display()))?;
        if !real.starts_with(&real_root) {
            return Err(outside());
        }
        return Ok(real);
    }
    Ok(joined)
}

/// Run ids are folder names made by this engine or the Python one: `[A-Za-z0-9][A-Za-z0-9._-]{0,80}`.
pub fn valid_run_id(id: &str) -> bool {
    let mut characters = id.chars();
    let first_ok = characters.next().is_some_and(|c| c.is_ascii_alphanumeric());
    first_ok && id.len() <= 81 && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `2026-10-05 16:13:37` UTC as `20261005-161337`, without a date library.
pub fn timestamp(unix_seconds: u64) -> String {
    let (days, rest) = (unix_seconds / 86_400, unix_seconds % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The run's name as part of a folder name: letters, digits, `.`, `_`, `-`, at most 40.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_matches(|c| c == '-' || c == '.').chars().take(40).collect();
    if trimmed.is_empty() {
        "run".into()
    } else {
        trimmed
    }
}

// ---------------------------------------------------------------------------------------------
// Event log

/// Events from line `since` on, each with its line number as `seq`. A last line without a
/// newline is still being written and is not consumed; a damaged line stops the reader.
pub fn read_events(path: &Path, since: usize) -> EventPage {
    let Ok(text) = std::fs::read_to_string(path) else {
        return EventPage { events: Vec::new(), next: since };
    };
    let mut events = Vec::new();
    let mut next = since;
    let mut number = 0;
    let mut rest = text.as_str();
    while let Some(end) = rest.find('\n') {
        let line = &rest[..end];
        rest = &rest[end + 1..];
        if number >= since {
            let Ok(Value::Object(mut event)) = serde_json::from_str::<Value>(line) else { break };
            event.insert("seq".into(), json!(number));
            events.push(Value::Object(event));
            next = number + 1;
        }
        number += 1;
    }
    if events.is_empty() {
        next = since.min(number);
    }
    EventPage { events, next }
}

fn append(path: &Path, event_type: &str, stage: Option<&str>, fields: Value) {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let mut record = json!({"type": event_type, "time": time, "stage": stage});
    if let (Value::Object(target), Value::Object(more)) = (&mut record, fields) {
        target.extend(more);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(format!("{record}\n").as_bytes());
        let _ = file.sync_all();
    }
}

fn finished(path: &Path) -> bool {
    read_events(path, 0).events.last().is_some_and(|event| event["type"] == "run_finished")
}

/// Closes a log that has `run_started` but no `run_finished`, saying why.
fn close_log(folder: &Path, reason: &str, status: &str) {
    let path = folder.join(EVENTS);
    let page = read_events(&path, 0);
    if page.events.is_empty() || page.events.last().is_some_and(|event| event["type"] == "run_finished") {
        return;
    }
    let started = page.events[0]["time"].as_f64().unwrap_or(0.0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(started);
    append(&path, "error", None, json!({"message": reason}));
    append(&path, "run_finished", None, json!({"status": status, "seconds": (now - started).max(0.0)}));
}

/// What the runs list shows, computed from the log like the HTTP engine does.
pub fn summarise(folder: &Path) -> RunSummary {
    let page = read_events(&folder.join(EVENTS), 0);
    let mut summary = RunSummary {
        id: folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        status: "unknown".into(),
        started: None,
        stage: None,
        stage_fraction: 0.0,
        events: page.events.len(),
    };
    for event in &page.events {
        match event["type"].as_str() {
            Some("run_started") => {
                summary.status = "running".into();
                summary.started = event["time"].as_f64();
            }
            Some("stage_started") => {
                summary.stage = event["stage"].as_str().map(str::to_string);
                summary.stage_fraction = 0.0;
            }
            Some("progress") => summary.stage_fraction = event["fraction"].as_f64().unwrap_or(summary.stage_fraction),
            Some("run_finished") => {
                summary.status = event["status"].as_str().unwrap_or("failed").to_string();
                if summary.status == "complete" {
                    summary.stage = None;
                }
            }
            _ => {}
        }
    }
    summary
}

// ---------------------------------------------------------------------------------------------
// The engine

impl Native {
    /// Sets the folders (at start and whenever the settings change) and closes the logs of
    /// runs that an earlier instance of the app left unfinished.
    pub fn configure(&self, runs: &Path, data: &Path) -> Result<(), String> {
        for folder in [runs, data] {
            std::fs::create_dir_all(folder).map_err(|error| format!("Could not create the folder {}: {error}", folder.display()))?;
        }
        *self.folders.lock().unwrap() = Folders { runs: runs.to_path_buf(), data: data.to_path_buf() };
        self.close_lost_runs();
        Ok(())
    }

    /// Sets where the external programs of the photos start are (from the settings).
    pub fn configure_tools(&self, tools: crate::config::Tools, repo: String) {
        *self.tools.lock().unwrap() = Some((tools, repo));
        self.forget_tools();
    }

    /// Makes the next look at the start points ask the tools again (after "Check").
    pub fn forget_tools(&self) {
        *self.described.lock().unwrap() = None;
    }

    /// The crate's start points for the form: every provider with whether it can run here
    /// and why not, its version, its options and its path fields.
    pub fn start_points(&self) -> Value {
        if let Some(points) = self.described.lock().unwrap().as_ref() {
            return points.clone();
        }
        let points = match self.tools.lock().unwrap().as_ref() {
            Some((tools, repo)) => crate::tools::start_points(tools, repo, crate::tools::contained()),
            None => start_points(),
        };
        *self.described.lock().unwrap() = Some(points.clone());
        points
    }

    /// The photos stage's options for a start request: calibration, providers, where their
    /// programs are (from the settings), then the request's tuning options.
    fn photo_options(&self, body: &StartBody) -> Result<Vec<String>, String> {
        let given = |text: &Option<String>| text.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
        let calibration = given(&body.calibration).ok_or("calibration: a lens calibration file, or auto, is needed to start from photos")?;
        // `auto`: no lens file; the turntable solver finds the focal length from the closed turn.
        let calibration = if calibration == "auto" {
            calibration
        } else {
            let path = self.resolve_source("calibration", &calibration)?;
            if !path.is_file() {
                return Err("calibration: that is not a file".into());
            }
            path.to_string_lossy().into_owned()
        };
        let points = self.start_points();
        let guard = self.tools.lock().unwrap();
        let (tools, repo) = guard.as_ref().ok_or("the tools of the photos start are not configured")?;
        let mut chosen = Vec::new();
        for (module, import) in [("masks", &body.masks_import), ("cameras", &body.cameras_import)] {
            let choice = points
                .as_array()
                .into_iter()
                .flatten()
                .filter(|point| point["id"] == "photos")
                .flat_map(|point| point["providers"].as_array().cloned().unwrap_or_default())
                .find(|choice| choice["module"] == module)
                .ok_or_else(|| format!("{module}: this build has no providers for it"))?;
            let name = body.providers.get(module).and_then(Value::as_str).or(choice["default"].as_str()).unwrap_or_default().to_string();
            let Some(option) = choice["options"].as_array().into_iter().flatten().find(|option| option["id"] == name.as_str()) else {
                return Err(format!("{module}: there is no provider named {name:?}"));
            };
            if option["available"] == false {
                return Err(format!("{module}: {}", option["reason"].as_str().unwrap_or("this provider cannot run here")));
            }
            let selector = if name == "import" {
                let key = format!("{module}_import");
                let path = given(import).ok_or_else(|| format!("{key}: say where the existing {module} are"))?;
                format!("import:{}", self.resolve_source(&key, &path)?.display())
            } else {
                name.clone()
            };
            chosen.push((name, selector));
        }
        crate::tools::check_tokens(&body.photo_options)?;
        let mut options = vec!["--calibration".to_string(), calibration];
        options.extend(["--masks".to_string(), chosen[0].1.clone(), "--cameras".to_string(), chosen[1].1.clone()]);
        if chosen[1].0 == "markers" {
            // The mat is a place: it comes from its own field, checked like every other path.
            let mat = given(&body.markers_mat).ok_or("markers_mat: the description of the printed marker mat is needed")?;
            let mat = self.resolve_source("markers_mat", &mat)?;
            if !mat.is_file() {
                return Err("markers_mat: that is not a file".into());
            }
            options.extend(["--markers-mat".to_string(), mat.to_string_lossy().into_owned()]);
        }
        if !crate::tools::contained() {
            options.extend(crate::tools::location_words(Some((&chosen[0].0, &chosen[1].0)), tools, repo));
        }
        options.extend(body.photo_options.iter().cloned());
        Ok(options)
    }

    fn runs(&self) -> PathBuf {
        self.folders.lock().unwrap().runs.clone()
    }

    fn data(&self) -> PathBuf {
        self.folders.lock().unwrap().data.clone()
    }

    pub fn folders(&self) -> (PathBuf, PathBuf) {
        (self.runs(), self.data())
    }

    /// Remembers a path picked in a native dialog; it may then be used as a run's input.
/// Writes one file that arrived from the system's file picker (phones) into the data folder,
    /// as `<folder>/<name>`, and returns that relative path. Both are single, plain names.
    pub fn import(&self, folder: &str, name: &str, bytes: &[u8]) -> Result<String, String> {
        let plain = |text: &str, what: &str| -> Result<String, String> {
            let text = text.trim();
            let bad = text.is_empty() || text.starts_with('.') || text.len() > 200 || text.chars().any(|c| c == '/' || c == '\\' || c == '\0' || c.is_control());
            if bad {
                return Err(format!("{what}: {text:?} is not a plain file or folder name"));
            }
            Ok(text.to_string())
        };
        // The folder may be nested (examples/dragon/photos); every part is a plain name.
        let parts: Vec<String> = folder.split('/').map(|part| plain(part, "folder")).collect::<Result<_, _>>()?;
        if parts.len() > 4 {
            return Err("folder: too deep".into());
        }
        let folder = parts.join("/");
        let name = plain(name, "name")?;
        let target = parts.iter().fold(self.data(), |path, part| path.join(part));
        std::fs::create_dir_all(&target).map_err(|error| format!("Could not create {folder}: {error}"))?;
        std::fs::write(target.join(&name), bytes).map_err(|error| format!("Could not write {name}: {error}"))?;
        Ok(format!("{folder}/{name}"))
    }

/// Removes a downloaded example object: `<data>/examples/<id>` and nothing else.
    pub fn delete_example(&self, id: &str) -> Result<(), String> {
        let plain = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plain {
            return Err(format!("{id:?} is not the name of an example object"));
        }
        let folder = self.data().join("examples").join(id);
        if folder.exists() {
            std::fs::remove_dir_all(&folder).map_err(|error| format!("Could not remove {id}: {error}"))?;
        }
        Ok(())
    }

    /// Free bytes on the disk of the data folder, where the platform says.
    pub fn free_bytes(&self) -> Option<u64> {
        free_space(&self.data())
    }

    pub fn grant(&self, path: &Path) {
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.granted.lock().unwrap().insert(real);
    }

    fn close_lost_runs(&self) {
        let Ok(entries) = std::fs::read_dir(self.runs()) else { return };
        let active = self.active.lock().unwrap();
        for entry in entries.flatten() {
            let folder = entry.path();
            let id = entry.file_name().to_string_lossy().into_owned();
            // Only runs this app started (they carry the marker), and not ones running right now.
            if active.contains_key(&id) || !folder.join(MARKER).is_file() {
                continue;
            }
            close_log(&folder, "The application stopped while this run was in progress.", "failed");
        }
    }

    pub fn run_folder(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_run_id(id) {
            return Err("unknown run".into());
        }
        let folder = self.runs().join(id);
        if folder.join(EVENTS).is_file() {
            Ok(folder)
        } else {
            Err("unknown run".into())
        }
    }

    pub fn list_runs(&self) -> Vec<RunSummary> {
        let mut runs: Vec<RunSummary> = std::fs::read_dir(self.runs())
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| valid_run_id(&entry.file_name().to_string_lossy()) && entry.path().join(EVENTS).is_file())
                    .map(|entry| summarise(&entry.path()))
                    .collect()
            })
            .unwrap_or_default();
        runs.sort_by(|a, b| b.started.unwrap_or(0.0).total_cmp(&a.started.unwrap_or(0.0)).then_with(|| b.id.cmp(&a.id)));
        runs
    }

    pub fn events(&self, id: &str, since: usize) -> Result<EventPage, String> {
        Ok(read_events(&self.run_folder(id)?.join(EVENTS), since))
    }

    /// A file of a run, by its path relative to the run directory.
    pub fn file(&self, id: &str, relative: &str) -> Result<PathBuf, String> {
        let path = contained(&self.run_folder(id)?, relative).map_err(|_| "not found".to_string())?;
        if path.is_file() {
            Ok(path)
        } else {
            Err("not found".into())
        }
    }

    /// Folders and files under the data folder; `inputs` marks folders a run can start from.
    pub fn list_data(&self, relative: &str) -> Result<DataListing, String> {
        let root = self.data();
        let folder = contained(&root, relative).ok().filter(|path| path.is_dir()).ok_or("no such folder under the data folder")?;
        let mut entries: Vec<DataEntry> = std::fs::read_dir(&folder)
            .map_err(|error| error.to_string())?
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                let path = entry.path();
                let directory = path.is_dir();
                Some(DataEntry {
                    inputs: directory && path.join("cameras.json").is_file(),
                    photos: directory && holds_photos(&path),
                    calibration: !directory && is_calibration(&path),
                    name,
                    directory,
                })
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries.truncate(500);
        let clean: Vec<&str> = relative.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
        Ok(DataListing { path: clean.join("/"), entries })
    }

    /// Where a path named in a start request really is: relative paths under the data
    /// folder, absolute ones only inside it or inside something picked in a dialog.
    fn resolve_source(&self, key: &str, text: &str) -> Result<PathBuf, String> {
        let missing = || format!("{key} must be an existing path under the data folder");
        let candidate = Path::new(text.trim());
        if candidate.as_os_str().is_empty() {
            return Err(missing());
        }
        if !candidate.is_absolute() {
            let path = contained(&self.data(), text.trim()).map_err(|_| missing())?;
            return if path.exists() { Ok(path) } else { Err(missing()) };
        }
        let real = std::fs::canonicalize(candidate).map_err(|_| format!("{key}: {} does not exist", candidate.display()))?;
        let data = std::fs::canonicalize(self.data()).unwrap_or_else(|_| self.data());
        let granted = self.granted.lock().unwrap();
        if real.starts_with(&data) || granted.iter().any(|allowed| real.starts_with(allowed)) {
            Ok(real)
        } else {
            Err(format!("{key}: choose this folder with the Choose button first, or put it under the data folder"))
        }
    }

    /// Checks a start request and turns it into the crate's options. Nothing is started.
    pub fn options(&self, body: &StartBody, id: &str) -> Result<RunOptions, String> {
        if body.reference.as_deref().is_some_and(|text| !text.trim().is_empty()) {
            return Err("reference: the built-in engine cannot score against a reference scan; leave it empty".into());
        }
        let mut options = RunOptions {
            output: self.runs().join(id), settings: body.settings.clone(), texture: body.texture,
            attribution: body.attribution.clone(), surface: body.surface.clone(), splats: body.splats.clone(), ..RunOptions::default()
        };
        for (name, method) in [("surface", &options.surface), ("splats", &options.splats)] {
            if let Some(method) = method {
                crispmesh_dense::run::Method::parse(&method.quality).map_err(|e| format!("{name}: {e}"))?;
                // Numbers only: the methods' path settings (hull mesh, photos, masks, start splats) would reach past
                // the folders chosen in the app.
                let numeric = |item: &String| item.split_once('=').is_some_and(|(_, value)| value.trim().parse::<f64>().is_ok());
                if let Some(bad) = method.settings.iter().find(|item| !numeric(item)) {
                    return Err(format!("{name}: {bad} is not a NAME=NUMBER setting"));
                }
            }
        }
        let given = |text: &Option<String>| text.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
        if let Some(photos) = given(&body.photos) {
            let folder = self.resolve_source("photos", &photos)?;
            if !folder.is_dir() {
                return Err("photos: that is not a folder".into());
            }
            options.photo_options = self.photo_options(body)?;
            // The crate's own reading of the options, before anything starts: a wrong value is
            // refused here with its sentence instead of failing the run.
            let mut arguments = vec!["--photos".to_string(), folder.to_string_lossy().into_owned()];
            arguments.extend(["--output".to_string(), options.output.join("frontend").to_string_lossy().into_owned()]);
            arguments.extend(options.photo_options.iter().cloned());
            crispmesh_dense::photos::options::resolve(&arguments, &|name| std::env::var(name).ok()).map_err(|error| format!("photos: {error:#}"))?;
            options.photos = Some(folder);
        } else if let Some(inputs) = given(&body.inputs) {
            let folder = self.resolve_source("inputs", &inputs)?;
            if !folder.join("cameras.json").is_file() {
                return Err("inputs: this folder has no cameras.json; it is not an inputs folder".into());
            }
            options.inputs = Some(folder);
        } else {
            let (Some(scene), Some(prepared), Some(raw_masks)) = (given(&body.scene), given(&body.prepared), given(&body.raw_masks)) else {
                return Err("inputs: give a photos folder with a calibration, an inputs folder, or all of scene, prepared and raw_masks".into());
            };
            options.scene = Some(self.resolve_source("scene", &scene)?);
            options.prepared = Some(self.resolve_source("prepared", &prepared)?);
            options.raw_masks = Some(self.resolve_source("raw_masks", &raw_masks)?);
        }
        // The crate words these like the Python reference does ("dense configuration: ...").
        options.configuration().map_err(|error| format!("{error:#}"))?;
        Ok(options)
    }

    fn new_id(&self, name: Option<&str>) -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let base = format!("{}-{}", timestamp(now), slug(name.unwrap_or("run")));
        let runs = self.runs();
        let mut id = base.clone();
        let mut counter = 2;
        while runs.join(&id).exists() {
            id = format!("{base}-{counter}");
            counter += 1;
        }
        id
    }

    /// Starts a run on a worker thread and returns its id once its event log exists.
    pub fn start(self: &Arc<Self>, body: StartBody) -> Result<String, String> {
        let id = self.new_id(body.name.as_deref());
        let options = self.options(&body, &id)?;
        let attribution = body.attribution.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
        if attribution.as_ref().is_some_and(|text| text.len() > 16 * 1024) {
            return Err("attribution: too long".into());
        }
        // The pipeline writes source credit before emitting its first event.
        self.start_with(id, options, crispmesh_dense::run::run)
    }

    /// The same with the pipeline passed in, so tests can run without a GPU.
    pub fn start_with<F>(self: &Arc<Self>, id: String, options: RunOptions, pipeline: F) -> Result<String, String>
    where
        F: FnOnce(&RunOptions, Option<crispmesh_dense::events::Observer>, Option<Arc<AtomicBool>>) -> anyhow::Result<Value> + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        self.active.lock().unwrap().insert(id.clone(), Active { cancel: Arc::clone(&cancel), done: Arc::clone(&done) });
        let (first, started) = mpsc::channel::<Result<(), String>>();
        let folder = options.output.clone();
        let engine = Arc::clone(self);
        let thread_id = id.clone();
        let spawned = std::thread::Builder::new().name(format!("run-{id}")).spawn(move || {
            let marker = folder.join(MARKER);
            let announced = AtomicBool::new(false);
            let first_event = Mutex::new(first.clone());
            let observer: crispmesh_dense::events::Observer = Arc::new(move |_event: &Value| {
                if !announced.swap(true, Ordering::SeqCst) {
                    let _ = std::fs::write(&marker, format!("{{\"pid\": {}}}\n", std::process::id()));
                    let _ = first_event.lock().unwrap().send(Ok(()));
                }
            });
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pipeline(&options, Some(observer), Some(cancel))));
            let log = folder.join(EVENTS);
            match outcome {
                // Refused before anything was written (bad output folder, full disk).
                Ok(Err(error)) if !log.is_file() => {
                    let _ = first.send(Err(format!("{error:#}")));
                }
                Ok(_) => {}
                Err(_) => close_log(&folder, "The engine stopped unexpectedly (internal error).", "failed"),
            }
            // Whatever happened, a log that was started is closed.
            if log.is_file() && !finished(&log) {
                close_log(&folder, "The engine stopped without finishing the run.", "failed");
            }
            done.store(true, Ordering::SeqCst);
            engine.active.lock().unwrap().remove(&thread_id);
        });
        if let Err(error) = spawned {
            self.active.lock().unwrap().remove(&id);
            return Err(format!("could not start a thread for the run: {error}"));
        }
        match started.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(id),
            Ok(Err(message)) => Err(format!("run did not start: {message}")),
            Err(_) => Err("run did not start: the engine did not write its first event".into()),
        }
    }

    /// Asks a run to stop: the cancel flag if it runs in this process, and the `cancel`
    /// file of the contract either way.
    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let folder = self.run_folder(id)?;
        let _ = std::fs::write(folder.join("cancel"), "");
        let active = self.active.lock().unwrap();
        Ok(active.get(id).map(|run| run.cancel.store(true, Ordering::SeqCst)).is_some())
    }

    pub fn running(&self) -> usize {
        self.active.lock().unwrap().values().filter(|run| !run.done.load(Ordering::SeqCst)).count()
    }

    /// Called when the app quits: asks every run to stop and waits for them, up to `grace`.
    /// A run that is still busy then gets its log closed here, because the process ends next.
    pub fn shutdown(&self, grace: Duration) {
        let ids: Vec<String> = {
            let active = self.active.lock().unwrap();
            for run in active.values() {
                run.cancel.store(true, Ordering::SeqCst);
            }
            active.keys().cloned().collect()
        };
        let deadline = Instant::now() + grace;
        while self.running() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let runs = self.runs();
        for id in ids {
            close_log(&runs.join(&id), "The application quit while this run was in progress.", "cancelled");
        }
    }
}

/// Name, group, meaning, kind and default of every setting, from the crate.
pub fn settings_schema() -> Value {
    crispmesh_dense::config::settings_schema()
}

/// How a run may start in this build, from the crate: the "New run" form is built from this
/// list, including the photos start with its mask and camera providers.
pub fn start_points() -> Value {
    crispmesh_dense::run::describe()["start_points"].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("crispmesh-studio-native-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::canonicalize(folder).unwrap()
    }

    fn engine(name: &str) -> (Arc<Native>, PathBuf) {
        let root = scratch(name);
        let engine = Arc::new(Native::default());
        engine.configure(&root.join("runs"), &root.join("data")).unwrap();
        (engine, root)
    }

    fn log(folder: &Path, lines: &[&str]) {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join(EVENTS), lines.concat()).unwrap();
    }

    const STARTED: &str = "{\"type\":\"run_started\",\"time\":100.0,\"stage\":null}\n";
    const STAGE: &str = "{\"type\":\"stage_started\",\"time\":101.0,\"stage\":\"stereo\"}\n";
    const PROGRESS: &str = "{\"type\":\"progress\",\"time\":102.0,\"stage\":\"stereo\",\"fraction\":0.43,\"message\":\"m\"}\n";
    const DONE: &str = "{\"type\":\"run_finished\",\"time\":110.0,\"stage\":null,\"status\":\"complete\",\"seconds\":10.0}\n";

    #[test]
    fn paths_stay_inside_their_folder() {
        let root = scratch("contained");
        std::fs::create_dir_all(root.join("run/mesh")).unwrap();
        std::fs::write(root.join("run/mesh/mesh.stl"), "x").unwrap();
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        assert_eq!(contained(&root.join("run"), "mesh/mesh.stl").unwrap(), root.join("run/mesh/mesh.stl"));
        assert_eq!(contained(&root.join("run"), "./mesh/./mesh.stl").unwrap(), root.join("run/mesh/mesh.stl"));
        assert_eq!(contained(&root.join("run"), "not/yet/there.png").unwrap(), root.join("run/not/yet/there.png"));
        for bad in ["../secret.txt", "mesh/../../secret.txt", "/etc/passwd", "mesh\\..\\..\\secret.txt", "a/\0/b"] {
            assert!(contained(&root.join("run"), bad).is_err(), "{bad} was accepted");
        }
        #[cfg(windows)]
        assert!(contained(&root.join("run"), "C:/Windows/win.ini").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_does_not_lead_out() {
        let root = scratch("symlink");
        std::fs::create_dir_all(root.join("run")).unwrap();
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("secret.txt"), root.join("run/link.txt")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("run/up")).unwrap();
        assert!(contained(&root.join("run"), "link.txt").is_err());
        assert!(contained(&root.join("run"), "up/secret.txt").is_err());
    }

    #[test]
    fn run_ids_names_and_timestamps() {
        assert!(valid_run_id("20261005-161337-studio-check"));
        assert!(valid_run_id("a"));
        for bad in ["", ".hidden", "-x", "a/b", "..", "a b", &"x".repeat(82)] {
            assert!(!valid_run_id(bad), "{bad:?} was accepted");
        }
        assert_eq!(timestamp(0), "19700101-000000");
        assert_eq!(timestamp(1_791_209_617), "20261005-141337");
        assert_eq!(timestamp(951_782_400), "20000229-000000");
        assert_eq!(slug("my dragon / take 2!"), "my-dragon-take-2");
        assert_eq!(slug("../.."), "run");
        assert_eq!(slug(&"x".repeat(60)).len(), 40);
    }

    #[test]
    fn events_are_paged_by_line_and_a_half_written_line_waits() {
        let root = scratch("events");
        log(&root, &[STARTED, STAGE, PROGRESS, "{\"type\":\"progress\",\"time\":103"]);
        let path = root.join(EVENTS);
        let all = read_events(&path, 0);
        assert_eq!(all.events.len(), 3);
        assert_eq!(all.next, 3);
        assert_eq!(all.events[2]["seq"], 2);
        assert_eq!(all.events[2]["fraction"], 0.43);
        let tail = read_events(&path, 2);
        assert_eq!((tail.events.len(), tail.next), (1, 3));
        assert_eq!(tail.events[0]["seq"], 2);
        let nothing = read_events(&path, 3);
        assert_eq!((nothing.events.len(), nothing.next), (0, 3));
        // The cursor never runs ahead of the file, whatever the caller asks.
        assert_eq!(read_events(&path, 99).next, 3);
        // Once the line is complete it is delivered.
        log(&root, &[STARTED, STAGE, PROGRESS, "{\"type\":\"progress\",\"time\":103}\n"]);
        assert_eq!(read_events(&path, 3).events.len(), 1);
        // A damaged line stops the reader there.
        log(&root, &[STARTED, "not json\n", DONE]);
        assert_eq!(read_events(&path, 0).next, 1);
        assert_eq!(read_events(&root.join("missing.jsonl"), 4).next, 4);
    }

    #[test]
    fn runs_are_listed_newest_first_with_their_state() {
        let (engine, root) = engine("list");
        log(&root.join("runs/20260101-000000-old"), &[STARTED, STAGE, PROGRESS, DONE]);
        log(&root.join("runs/20260102-000000-busy"), &[&STARTED.replace("100.0", "200.0"), STAGE, PROGRESS]);
        log(&root.join("runs/bad name"), &[STARTED]);
        std::fs::create_dir_all(root.join("runs/20260103-000000-empty")).unwrap();
        let runs = engine.list_runs();
        assert_eq!(runs.iter().map(|run| run.id.as_str()).collect::<Vec<_>>(), ["20260102-000000-busy", "20260101-000000-old"]);
        assert_eq!(runs[0].status, "running");
        assert_eq!(runs[0].stage.as_deref(), Some("stereo"));
        assert_eq!(runs[0].stage_fraction, 0.43);
        assert_eq!(runs[0].events, 3);
        assert_eq!((runs[1].status.as_str(), runs[1].stage.clone(), runs[1].started), ("complete", None, Some(100.0)));
        assert!(engine.events("20260101-000000-old", 0).is_ok());
        assert!(engine.events("../runs/20260101-000000-old", 0).is_err());
        assert!(engine.events("20260103-000000-empty", 0).is_err());
    }

    #[test]
    fn run_files_are_served_only_from_inside_the_run() {
        let (engine, root) = engine("files");
        let run = root.join("runs/r1");
        log(&run, &[STARTED, DONE]);
        std::fs::create_dir_all(run.join("mesh")).unwrap();
        std::fs::write(run.join("mesh/mesh.stl"), "stl").unwrap();
        log(&root.join("runs/r2"), &[STARTED]);
        std::fs::write(root.join("runs/r2/private.txt"), "x").unwrap();
        assert_eq!(engine.file("r1", "mesh/mesh.stl").unwrap(), run.join("mesh/mesh.stl"));
        for bad in ["../r2/private.txt", "mesh", "mesh/none.stl", "/etc/hosts", ""] {
            assert!(engine.file("r1", bad).is_err(), "{bad:?} was served");
        }
        assert!(engine.file("nope", "mesh/mesh.stl").is_err());
    }

    #[test]
    fn files_from_the_picker_land_in_the_data_folder_and_nowhere_else() {
        let (engine, root) = engine("import");
        assert_eq!(engine.import("photos-1", "IMG_0001.HEIC", b"x").unwrap(), "photos-1/IMG_0001.HEIC");
        assert_eq!(std::fs::read(root.join("data/photos-1/IMG_0001.HEIC")).unwrap(), b"x");
        assert_eq!(engine.import("examples/dragon/photos", "d_0.png", b"y").unwrap(), "examples/dragon/photos/d_0.png");
        assert!(root.join("data/examples/dragon/photos/d_0.png").is_file());
        for (folder, name) in [("..", "a"), ("a/../b", "c"), ("a//b", "c"), ("/abs", "c"), ("a/b/c/d/e", "f"), ("ok", "../x"), ("ok", ".hidden"), ("", "a"), ("ok", "a\\b")] {
            assert!(engine.import(folder, name, b"x").is_err(), "{folder:?} {name:?} was accepted");
        }
        assert!(!root.join("x").exists() && !root.join("data/x").exists());
    }

    #[test]
    fn examples_are_deleted_by_name_only_and_the_attribution_goes_with_the_run() {
        let (engine, root) = engine("examples");
        std::fs::create_dir_all(root.join("data/examples/dragon/rgb")).unwrap();
        std::fs::write(root.join("data/examples/dragon/rgb/a.png"), "x").unwrap();
        for bad in ["", "..", "../runs", "a/b", "dragon.png"] {
            assert!(engine.delete_example(bad).is_err(), "{bad:?} was accepted");
        }
        engine.delete_example("dragon").unwrap();
        assert!(!root.join("data/examples/dragon").exists() && root.join("data").exists());
        engine.delete_example("never-downloaded").unwrap();
        assert!(engine.free_bytes().is_none_or(|bytes| bytes > 0));
    }

    #[test]
    fn the_data_folder_is_listed_with_inputs_marked() {
        let (engine, root) = engine("data");
        std::fs::create_dir_all(root.join("data/objects/sphere")).unwrap();
        std::fs::write(root.join("data/objects/sphere/cameras.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("data/objects/masks")).unwrap();
        std::fs::write(root.join("data/objects/scan.ply"), "").unwrap();
        std::fs::write(root.join("data/objects/.hidden"), "").unwrap();
        let listing = engine.list_data("objects/").unwrap();
        assert_eq!(listing.path, "objects");
        let rows: Vec<(&str, bool, bool)> = listing.entries.iter().map(|e| (e.name.as_str(), e.directory, e.inputs)).collect();
        assert_eq!(rows, [("masks", true, false), ("scan.ply", false, false), ("sphere", true, true)]);
        assert!(listing.entries.iter().all(|e| !e.photos && !e.calibration));
        // Photos folders (three images or more) and lens calibrations are marked too.
        std::fs::create_dir_all(root.join("data/shoot/rgb")).unwrap();
        for name in ["a.JPG", "b.png", "c.tiff", "notes.txt"] {
            std::fs::write(root.join("data/shoot/rgb").join(name), "").unwrap();
        }
        std::fs::create_dir_all(root.join("data/shoot/two")).unwrap();
        std::fs::write(root.join("data/shoot/two/a.jpg"), "").unwrap();
        std::fs::write(root.join("data/shoot/two/b.jpg"), "").unwrap();
        std::fs::write(root.join("data/shoot/lens.json"), r#"{"schema":"crispmesh_lens_calibration_v1"}"#).unwrap();
        std::fs::write(root.join("data/shoot/other.json"), r#"{"schema":"something_else"}"#).unwrap();
        let shoot = engine.list_data("shoot").unwrap();
        let marks: Vec<(&str, bool, bool)> = shoot.entries.iter().map(|e| (e.name.as_str(), e.photos, e.calibration)).collect();
        assert_eq!(marks, [("lens.json", false, true), ("other.json", false, false), ("rgb", true, false), ("two", false, false)]);
        assert_eq!(engine.list_data("").unwrap().path, "");
        assert!(engine.list_data("../runs").is_err());
        assert!(engine.list_data("objects/scan.ply").is_err());
        assert!(engine.list_data("nowhere").is_err());
    }

    #[test]
    fn start_requests_are_checked_before_anything_runs() {
        let (engine, root) = engine("options");
        std::fs::create_dir_all(root.join("data/sphere")).unwrap();
        std::fs::create_dir_all(root.join("data/empty")).unwrap();
        std::fs::write(root.join("data/sphere/cameras.json"), "{}").unwrap();
        let body = |inputs: &str| StartBody { inputs: Some(inputs.into()), ..StartBody::default() };
        let options = engine.options(&body("sphere"), "id").unwrap();
        assert_eq!(options.inputs.as_deref(), Some(root.join("data/sphere").as_path()));
        assert_eq!(options.output, root.join("runs/id"));
        assert!(engine.options(&body("empty"), "id").unwrap_err().contains("no cameras.json"));
        assert!(engine.options(&body("../runs"), "id").unwrap_err().starts_with("inputs must be an existing path"));
        assert!(engine.options(&body("nowhere"), "id").unwrap_err().starts_with("inputs must be an existing path"));
        assert!(engine.options(&StartBody::default(), "id").unwrap_err().starts_with("inputs:"));
        // Settings are validated by the crate, with the reference's wording.
        let mut wrong = body("sphere");
        wrong.settings.insert("sizes".into(), json!([128, 64]));
        let message = engine.options(&wrong, "id").unwrap_err();
        assert!(message.contains("sizes must increase"), "{message}");
        let mut unknown = body("sphere");
        unknown.settings.insert("bogus".into(), json!(1));
        assert!(engine.options(&unknown, "id").unwrap_err().contains("unknown setting"));
        let mut scored = body("sphere");
        scored.reference = Some("scan.ply".into());
        assert!(engine.options(&scored, "id").unwrap_err().starts_with("reference:"));
        // An absolute path is refused until the user picked it (or something above it) in a dialog.
        let elsewhere = scratch("options-elsewhere");
        std::fs::write(elsewhere.join("cameras.json"), "{}").unwrap();
        let absolute = elsewhere.to_string_lossy().into_owned();
        assert!(engine.options(&body(&absolute), "id").unwrap_err().contains("Choose"));
        engine.grant(&elsewhere);
        assert_eq!(engine.options(&body(&absolute), "id").unwrap().inputs, Some(elsewhere));
        assert!(!root.join("runs/id").exists(), "checking must not create the run");
    }

    /// A stand-in pipeline: writes a log like the crate does, then waits to be cancelled.
    fn waiting_pipeline(
        options: &RunOptions,
        observer: Option<crispmesh_dense::events::Observer>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> anyhow::Result<Value> {
        std::fs::create_dir_all(&options.output)?;
        let events = crispmesh_dense::events::EventLog::for_run(&options.output, observer);
        events.emit("run_started", json!({"schema": "crispmesh_dense_events_v1"}))?;
        events.stage("stereo").emit("stage_started", json!({}))?;
        let cancel = cancel.expect("a cancel flag is always passed");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !cancel.load(Ordering::SeqCst) && !options.output.join("cancel").exists() {
            anyhow::ensure!(Instant::now() < deadline, "nobody cancelled");
            std::thread::sleep(Duration::from_millis(10));
        }
        events.stage("stereo").emit("error", json!({"message": "cancelled on request"}))?;
        events.emit("run_finished", json!({"status": "cancelled", "seconds": 0.1}))?;
        Ok(json!({}))
    }

    fn wait_until(what: &str, test: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !test() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_run_starts_on_a_thread_and_cancelling_ends_it() {
        let (engine, root) = engine("cancel");
        let options = RunOptions { output: root.join("runs/r1"), ..RunOptions::default() };
        let id = engine.start_with("r1".into(), options, waiting_pipeline).unwrap();
        assert_eq!(id, "r1");
        assert_eq!(engine.running(), 1);
        assert!(root.join("runs/r1").join(MARKER).is_file());
        assert_eq!(engine.list_runs()[0].status, "running");
        assert_eq!(engine.cancel("r1"), Ok(true));
        wait_until("the run to end", || engine.running() == 0);
        let events = engine.events("r1", 0).unwrap().events;
        assert_eq!(events.last().unwrap()["status"], "cancelled");
        assert_eq!(engine.list_runs()[0].status, "cancelled");
        // Cancelling a finished run only leaves the file; an unknown run is an error.
        assert_eq!(engine.cancel("r1"), Ok(false));
        assert!(engine.cancel("nope").is_err());
    }

    #[test]
    fn a_pipeline_that_refuses_fails_the_start_with_its_message() {
        let (engine, root) = engine("refuse");
        let options = RunOptions { output: root.join("runs/r1"), ..RunOptions::default() };
        let error = engine.start_with("r1".into(), options, |_, _, _| anyhow::bail!("only 0.3 GiB free; need 2")).unwrap_err();
        assert!(error.contains("only 0.3 GiB free"), "{error}");
        wait_until("the thread to end", || engine.running() == 0);
        assert!(engine.list_runs().is_empty());
    }

    #[test]
    fn a_log_is_always_closed_even_when_the_pipeline_dies_or_gives_up() {
        let (engine, root) = engine("closed");
        let dying = |options: &RunOptions, observer: Option<crispmesh_dense::events::Observer>, _: Option<Arc<AtomicBool>>| {
            std::fs::create_dir_all(&options.output).unwrap();
            crispmesh_dense::events::EventLog::for_run(&options.output, observer).emit("run_started", json!({})).unwrap();
            if options.output.ends_with("panics") {
                panic!("boom");
            }
            anyhow::bail!("gave up without closing the log")
        };
        for name in ["panics", "gives-up"] {
            let options = RunOptions { output: root.join("runs").join(name), ..RunOptions::default() };
            engine.start_with(name.into(), options, dying).unwrap();
        }
        wait_until("both threads to end", || engine.running() == 0);
        for name in ["panics", "gives-up"] {
            let events = engine.events(name, 0).unwrap().events;
            assert_eq!(events[events.len() - 2]["type"], "error", "{name}");
            assert_eq!(events.last().unwrap()["status"], "failed", "{name}");
        }
    }

    #[test]
    fn quitting_stops_runs_and_a_later_start_closes_what_was_lost() {
        let (engine, root) = engine("shutdown");
        let options = RunOptions { output: root.join("runs/r1"), ..RunOptions::default() };
        engine.start_with("r1".into(), options, waiting_pipeline).unwrap();
        engine.shutdown(Duration::from_secs(5));
        assert_eq!(engine.running(), 0);
        assert_eq!(engine.events("r1", 0).unwrap().events.last().unwrap()["status"], "cancelled");

        // A run of an earlier app instance without an end, and one by another engine (no marker).
        log(&root.join("runs/lost"), &[STARTED, STAGE]);
        std::fs::write(root.join("runs/lost").join(MARKER), "{\"pid\": 1}").unwrap();
        log(&root.join("runs/foreign"), &[STARTED, STAGE]);
        let again = Native::default();
        again.configure(&root.join("runs"), &root.join("data")).unwrap();
        let lost = again.events("lost", 0).unwrap().events;
        assert_eq!(lost.last().unwrap()["status"], "failed");
        assert!(lost[lost.len() - 2]["message"].as_str().unwrap().contains("application stopped"));
        assert_eq!(again.events("foreign", 0).unwrap().events.len(), 2);
    }

    #[test]
    fn the_settings_schema_comes_from_the_crate_and_matches_its_defaults() {
        let schema = settings_schema();
        let rows = schema["settings"].as_array().unwrap();
        let mut native = serde_json::to_value(crispmesh_dense::config::DenseConfig::default()).unwrap();
        // The normal engine build excludes the research-only PatchMatch controls.
        for key in ["patchmatch", "patchmatch_iterations", "patchmatch_normal_agreement"] {
            assert!(native.as_object_mut().unwrap().remove(key).is_some());
        }
        let from_schema: Map<String, Value> = rows.iter().map(|row| (row["name"].as_str().unwrap().to_string(), row["default"].clone())).collect();
        assert_eq!(Value::Object(from_schema), native);
        for row in rows {
            assert!(row["group"].as_str().is_some_and(|text| !text.is_empty()), "{row}");
            assert!(row["meaning"].as_str().is_some_and(|text| !text.is_empty()), "{row}");
            assert!(matches!(row["kind"].as_str(), Some("boolean" | "integer" | "number" | "integer_list" | "number_list")), "{row}");
        }
    }

    /// The real pipeline on the crate's synthetic sphere, started the way the app starts it.
    /// With a GPU it completes; without one (CI runners) it must fail cleanly: an `error`
    /// event with the reason, then `run_finished`, and nothing left running.
    #[test]
    fn a_real_run_completes_or_fails_cleanly_without_a_gpu() {
        let (engine, root) = engine("real");
        crispmesh_dense::stereo::synthetic::write(&root.join("data/sphere"), 24, 128).unwrap();
        let mut body = StartBody { inputs: Some("sphere".into()), name: Some("ci sphere".into()), ..StartBody::default() };
        for item in crispmesh_dense::stereo::synthetic::SMALL {
            let (key, value) = item.split_once('=').unwrap();
            let numbers: Vec<Value> = value.split(',').map(|text| serde_json::from_str(text).unwrap()).collect();
            body.settings.insert(key.into(), if numbers.len() == 1 { numbers[0].clone() } else { Value::Array(numbers) });
        }
        let id = engine.start(body).unwrap();
        assert!(id.ends_with("-ci-sphere"), "{id}");
        let deadline = Instant::now() + Duration::from_secs(600);
        while engine.running() > 0 {
            assert!(Instant::now() < deadline, "the run did not end");
            std::thread::sleep(Duration::from_millis(100));
        }
        let events = engine.events(&id, 0).unwrap().events;
        assert_eq!(events[0]["type"], "run_started");
        let last = events.last().unwrap();
        assert_eq!(last["type"], "run_finished");
        let summary = &engine.list_runs()[0];
        match last["status"].as_str() {
            Some("complete") => {
                assert!(engine.file(&id, "mesh/mesh.stl").is_ok());
                assert!(events.iter().any(|event| event["kind"] == "preview_mesh"));
                eprintln!("real run: complete, {} events, device {}", events.len(), events[0]["device"]);
            }
            Some("failed") => {
                let error = events.iter().rev().find(|event| event["type"] == "error").expect("a failed run says why");
                let message = error["message"].as_str().unwrap_or_default();
                assert!(!message.is_empty());
                eprintln!("real run: failed cleanly: {message}");
            }
            other => panic!("unexpected end: {other:?}"),
        }
        assert_eq!(summary.status, last["status"].as_str().unwrap());
    }

    #[test]
    fn start_points_come_from_the_crate_with_the_photos_start_and_its_providers() {
        let points = start_points();
        let ids: Vec<&str> = points.as_array().unwrap().iter().map(|point| point["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["inputs", "scene", "photos"]);
        for point in points.as_array().unwrap() {
            assert!(point["providers"].is_array());
            assert!(!point["fields"].as_array().unwrap().is_empty());
        }
        let modules: Vec<&str> = points[2]["providers"].as_array().unwrap().iter().map(|choice| choice["module"].as_str().unwrap()).collect();
        assert_eq!(modules, ["masks", "cameras"]);
    }

    fn no_tools() -> crate::config::Tools {
        let empty = || crate::config::Value { value: String::new(), source: crate::config::Source::Default };
        crate::config::Tools {
            alicevision: empty(),
            alicevision_library_path: empty(),
            colmap: crate::config::Value { value: "/definitely/not/colmap".into(), source: crate::config::Source::Setting },
            sam_python: empty(),
            sam_source: empty(),
            sam_checkpoint: empty(),
        }
    }

    #[test]
    fn a_photos_start_is_checked_and_turned_into_the_photos_stage_options() {
        let (engine, root) = engine("photos");
        std::fs::create_dir_all(root.join("data/bunny/rgb")).unwrap();
        for n in 0..4 {
            std::fs::write(root.join(format!("data/bunny/rgb/photo_{n:03}.png")), "x").unwrap();
        }
        std::fs::create_dir_all(root.join("data/bunny/masks")).unwrap();
        std::fs::write(root.join("data/bunny/final.sfm"), "{}").unwrap();
        let calibration = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../scripts/turntable_mesh/calibrations/3dlf-pro.json");
        std::fs::copy(calibration, root.join("data/lens.json")).unwrap();
        let mut tools = no_tools();
        tools.alicevision.value = root.join("no-alicevision-here").to_string_lossy().into_owned();
        engine.configure_tools(tools, String::new());

        let body = |providers: Value| StartBody {
            photos: Some("bunny/rgb".into()),
            calibration: Some("lens.json".into()),
            providers: providers.as_object().cloned().unwrap(),
            ..StartBody::default()
        };
        // Providers inside the crate need no tool, and none is named.
        let mut request = body(json!({"masks": "threshold", "cameras": "turntable"}));
        request.texture = true;
        request.attribution = Some("Photo source credit".into());
        request.photo_options = ["--threshold-level", "otsu", "--turntable-span", "6"].map(String::from).to_vec();
        request.surface = Some(crispmesh_dense::run::Method { quality: "fast".into(), settings: vec!["crop=600".into()] });
        let options = engine.options(&request, "id").unwrap();
        assert_eq!(options.surface.as_ref().map(|m| m.quality.as_str()), Some("fast"));
        assert_eq!(options.splats, None);
        // Only numeric single settings: the methods' path settings would reach past the chosen folders.
        let mut bad = request.clone();
        bad.splats = Some(crispmesh_dense::run::Method { quality: "standard".into(), settings: vec!["init=/etc/passwd".into()] });
        assert!(engine.options(&bad, "id").unwrap_err().contains("NAME=NUMBER"));
        bad.splats = Some(crispmesh_dense::run::Method { quality: "quick".into(), settings: Vec::new() });
        assert!(engine.options(&bad, "id").is_err());
        assert!(options.texture);
        assert_eq!(options.attribution.as_deref(), Some("Photo source credit"));
        assert_eq!(options.photos.as_deref(), Some(root.join("data/bunny/rgb").as_path()));
        assert_eq!(options.inputs, None);
        let lens = root.join("data/lens.json").to_string_lossy().into_owned();
        assert_eq!(options.photo_options, ["--calibration", lens.as_str(), "--masks", "threshold", "--cameras", "turntable", "--threshold-level", "otsu", "--turntable-span", "6"]);
        // No lens file: `auto` goes through as it is (the solver finds the focal length); a missing file does not.
        let mut automatic = body(json!({}));
        automatic.calibration = Some("auto".into());
        assert_eq!(engine.options(&automatic, "id").unwrap().photo_options[..2], ["--calibration", "auto"]);
        automatic.calibration = Some("auto.json".into());
        assert!(engine.options(&automatic, "id").is_err());
        // Nothing chosen: the app's defaults, which need nothing external.
        let defaults = engine.options(&body(json!({})), "id").unwrap().photo_options;
        assert_eq!(defaults[2..6], ["--masks", "threshold", "--cameras", "turntable"]);

        // Imports name their place through the request's own fields, inside the data folder.
        let mut imported = body(json!({"masks": "import", "cameras": "import"}));
        imported.masks_import = Some("bunny/masks".into());
        imported.cameras_import = Some("bunny/final.sfm".into());
        let options = engine.options(&imported, "id").unwrap();
        assert!(options.photo_options.contains(&format!("import:{}", root.join("data/bunny/masks").display())));
        assert!(options.photo_options.contains(&format!("import:{}", root.join("data/bunny/final.sfm").display())));
        imported.cameras_import = Some("../../etc".into());
        assert!(engine.options(&imported, "id").unwrap_err().starts_with("cameras_import"));
        imported.cameras_import = None;
        assert!(engine.options(&imported, "id").unwrap_err().starts_with("cameras_import:"));

        // The marker mat is a file named by its own field, inside the data folder like every path.
        let mut mat = imported.clone();
        mat.providers.insert("cameras".into(), "markers".into());
        assert!(engine.options(&mat, "id").unwrap_err().starts_with("markers_mat:"));
        mat.markers_mat = Some("../outside.json".into());
        assert!(engine.options(&mat, "id").unwrap_err().starts_with("markers_mat"));
        mat.markers_mat = Some("bunny/final.sfm".into());
        let words = engine.options(&mat, "id").unwrap().photo_options;
        let at = words.iter().position(|word| word == "--markers-mat").expect("the mat is passed on");
        assert!(words[at + 1].ends_with("final.sfm"));
        assert_eq!(words[words.iter().position(|word| word == "--cameras").unwrap() + 1], "markers");
        mat.photo_options = vec!["--markers-mat".into(), "/etc/passwd".into()];
        assert!(engine.options(&mat, "id").unwrap_err().contains("--markers-mat cannot be set by a run request"));

        // What cannot run, does not exist, or tries to name a program is refused with a sentence.
        assert!(engine.options(&body(json!({"masks": "threshold", "cameras": "colmap"})), "id").unwrap_err().starts_with("cameras: "));
        assert!(engine.options(&body(json!({"masks": "threshold", "cameras": "alicevision"})), "id").unwrap_err().starts_with("cameras: "));
        assert!(engine.options(&body(json!({"masks": "external-sam", "cameras": "turntable"})), "id").unwrap_err().starts_with("masks: "));
        assert!(engine.options(&body(json!({"masks": "magic", "cameras": "turntable"})), "id").unwrap_err().contains("no provider named"));
        let mut sneaky = body(json!({"masks": "threshold", "cameras": "turntable"}));
        sneaky.photo_options = vec!["--alicevision".into(), "/bin/sh".into()];
        assert!(engine.options(&sneaky, "id").unwrap_err().contains("cannot be set by a run request"));
        let mut wrong = body(json!({"masks": "threshold", "cameras": "turntable"}));
        wrong.photo_options = vec!["--no-such-option".into()];
        assert!(engine.options(&wrong, "id").unwrap_err().starts_with("photos: "));
        let mut uncalibrated = body(json!({"masks": "threshold", "cameras": "turntable"}));
        uncalibrated.calibration = None;
        assert!(engine.options(&uncalibrated, "id").unwrap_err().starts_with("calibration:"));
        assert!(!root.join("runs/id").exists(), "checking must not create the run");

        // The start points for the form say the same.
        let points = engine.start_points();
        let photos = points.as_array().unwrap().iter().find(|point| point["id"] == "photos").unwrap();
        let cameras = photos["providers"].as_array().unwrap().iter().find(|choice| choice["module"] == "cameras").unwrap();
        let available = |id: &str| {
            let options = cameras["options"].as_array().expect("camera providers");
            options.iter().find(|option| option["id"] == id).unwrap_or_else(|| panic!("no provider {id}"))["available"].clone()
        };
        assert_eq!(available("turntable"), true);
        assert_eq!(available("alicevision"), false);
        assert_eq!(available("colmap"), false);
        assert_eq!(cameras["default"], "turntable");
    }
}
