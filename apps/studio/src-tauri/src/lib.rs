//! CrispMesh as an app.
//!
//! Three ways to get a reconstruction, in the order the app prefers them:
//!
//! 1. **Built-in engine** (`native_engine`, module `native`): `crates/dense` runs inside
//!    this process on a worker thread. No Python, no child process. This is the default
//!    wherever the crate is linked, including the sandboxed Mac App Store variant.
//! 2. **External Python engine** (`local_engine`, module `engine`, desktop only): the shell
//!    starts `scripts.turntable_mesh.engine_server` with an interpreter the user names and
//!    hands its address and token to the web view. Kept for the photos start and for
//!    comparisons. Python is not bundled.
//! 3. An engine elsewhere over HTTP, recorded runs, the demo: plain web app, no shell code.
//!
//! The web view gets no Tauri permissions beyond calling the commands below. Every path it
//! can name is relative to the runs or the data folder and checked to stay inside.

mod config;
mod remote;
#[cfg(local_engine)]
mod engine;
#[cfg(native_engine)]
mod native;
#[cfg(all(native_engine, desktop))]
mod share;
#[cfg(native_engine)]
mod tools;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use config::{Config, Resolved, Surroundings};

#[allow(dead_code)]
const UNAVAILABLE: &str = "This build of the app does not include that.";

#[derive(Serialize)]
struct ShellInfo {
    /// The reconstruction is built into this app.
    native_engine: bool,
    /// This app can start the external Python engine (desktop builds outside the sandbox).
    can_run_engine: bool,
    /// Native folder and file pickers exist.
    can_pick_paths: bool,
    /// The app runs in the macOS App Sandbox: no other programs, only folders the user picked.
    sandboxed: bool,
    os: &'static str,
    version: &'static str,
    /// The device `auto` stands for when the Python engine is used on this computer.
    auto_device: &'static str,
}

#[tauri::command]
fn shell_info() -> ShellInfo {
    ShellInfo {
        native_engine: cfg!(native_engine),
        can_run_engine: cfg!(local_engine),
        can_pick_paths: cfg!(desktop),
        sandboxed: cfg!(any(target_os = "ios", target_os = "android")) || std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some(),
        os: std::env::consts::OS,
        version: env!("CARGO_PKG_VERSION"),
        auto_device: config::auto_device(),
    }
}

// ---------------------------------------------------------------------------------------------
// Settings: where the folders and (for the Python engine) the interpreters are

#[derive(Serialize)]
struct Settings {
    saved: Config,
    /// What will actually be used, and where each value comes from.
    resolved: Resolved,
    /// Where the external programs of the photos start are (AliceVision, COLMAP, SAM).
    tools: config::Tools,
    /// A sentence when the external Python engine cannot be started with these values.
    problem: Option<String>,
    file: String,
}

fn config_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_config_dir().map(|folder| folder.join("config.json")).map_err(|error| error.to_string())
}

fn settings(app: &AppHandle) -> Result<Settings, String> {
    let file = config_file(app)?;
    let saved = config::load(&file);
    // On an iPhone or iPad the folders are in the app's Documents, which the Files app shows
    // ("On My iPhone"): that is how photos get in and how models get out.
    let base = if cfg!(target_os = "ios") { app.path().document_dir() } else { app.path().app_data_dir() };
    let around = base.map(Surroundings::real).map_err(|error| error.to_string())?;
    let resolved = config::resolve(&saved, &around);
    let problem = config::problems(&resolved);
    let tools = config::resolve_tools(&saved, &around);
    Ok(Settings { saved, resolved, tools, problem, file: file.to_string_lossy().into_owned() })
}

/// Points the built-in engine at the configured folders.
fn configure_native(app: &AppHandle) -> Result<(), String> {
    #[cfg(native_engine)]
    {
        let settings = settings(app)?;
        let engine = app.state::<std::sync::Arc<native::Native>>();
        engine.configure_tools(settings.tools.clone(), settings.resolved.repo.value.clone());
        engine.configure(std::path::Path::new(&settings.resolved.runs_dir.value), std::path::Path::new(&settings.resolved.data_dir.value))?;
    }
    #[cfg(not(native_engine))]
    let _ = app;
    Ok(())
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<Settings, String> {
    settings(&app)
}

#[tauri::command]
fn save_settings(app: AppHandle, config: Config) -> Result<Settings, String> {
    config::save(&config_file(&app)?, &config)?;
    configure_native(&app)?;
    settings(&app)
}

/// Opens the native picker. `kind` is `folder` or `file`. Returns the chosen path or null.
/// What was picked may afterwards be used as the input of a run of the built-in engine.
#[tauri::command]
async fn pick_path(app: AppHandle, kind: String, title: String, start: Option<String>) -> Result<Option<String>, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        let mut dialog = app.dialog().file().set_title(title);
        if let Some(start) = start.map(std::path::PathBuf::from).filter(|path| path.is_dir()) {
            dialog = dialog.set_directory(start);
        }
        let chosen = if kind == "file" { dialog.blocking_pick_file() } else { dialog.blocking_pick_folder() };
        let path = chosen.and_then(|path| path.into_path().ok());
        #[cfg(native_engine)]
        if let Some(path) = &path {
            app.state::<std::sync::Arc<native::Native>>().grant(path);
        }
        Ok(path.map(|path| path.to_string_lossy().into_owned()))
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, kind, title, start);
        Err(UNAVAILABLE.into())
    }
}

// ---------------------------------------------------------------------------------------------
// External Python engine (optional)

#[tauri::command]
fn engine_status(app: AppHandle) -> Result<Value, String> {
    #[cfg(local_engine)]
    {
        serde_json::to_value(app.state::<engine::Engine>().status()).map_err(|error| error.to_string())
    }
    #[cfg(not(local_engine))]
    {
        let _ = app;
        Err(UNAVAILABLE.into())
    }
}

/// Starts (or restarts) the Python engine with the current settings. It is not started
/// until somebody asks: the built-in engine is the default.
#[tauri::command]
fn restart_engine(app: AppHandle) -> Result<(), String> {
    #[cfg(local_engine)]
    {
        let settings = settings(&app)?;
        app.state::<engine::Engine>().restart(settings.resolved);
        Ok(())
    }
    #[cfg(not(local_engine))]
    {
        let _ = app;
        Err(UNAVAILABLE.into())
    }
}

#[tauri::command]
fn stop_engine(app: AppHandle) -> Result<(), String> {
    #[cfg(local_engine)]
    {
        let engine = app.state::<engine::Engine>().inner().clone();
        std::thread::spawn(move || engine.stop());
    }
    let _ = app;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Built-in engine: the same operations as the HTTP engine of docs/ENGINE-CONTRACT.md

#[cfg(native_engine)]
fn built_in(app: &AppHandle) -> std::sync::Arc<native::Native> {
    app.state::<std::sync::Arc<native::Native>>().inner().clone()
}

/// `GET /api/health`, plus what the form needs to know about this engine.
#[tauri::command]
async fn native_health(app: AppHandle) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        let (runs, data) = built_in(&app).folders();
        Ok(serde_json::json!({
            "schema": crispmesh_dense::events::SCHEMA,
            "device": "wgpu",
            "can_start_runs": true,
            "runs_dir": runs.to_string_lossy(),
            "data_dir": data.to_string_lossy(),
            "start_points": built_in(&app).start_points(),
            "methods": crispmesh_dense::run::methods(),
            "sandboxed": tools::contained(),
            // Phones have no folder dialog of the app's own: files come in through the system's picker.
            "imports": cfg!(any(target_os = "ios", target_os = "android")),
            "note": if cfg!(target_os = "ios") {
                serde_json::json!("Photos get in and models get out through the Files app: under On My iPhone (or iPad), CrispMesh has a folder \"data\" for a folder of photos and the lens calibration, and a folder \"runs\" where each run leaves its STL.")
            } else {
                Value::Null
            },
        }))
    }
    #[cfg(not(native_engine))]
    {
        let _ = app;
        Err(UNAVAILABLE.into())
    }
}

/// `GET /api/settings`.
#[tauri::command]
fn native_settings() -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        Ok(native::settings_schema())
    }
    #[cfg(not(native_engine))]
    Err(UNAVAILABLE.into())
}

/// `GET /api/runs`.
#[tauri::command]
async fn native_runs(app: AppHandle) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        Ok(serde_json::json!({ "runs": built_in(&app).list_runs() }))
    }
    #[cfg(not(native_engine))]
    {
        let _ = app;
        Err(UNAVAILABLE.into())
    }
}

/// `POST /api/runs`. Returns `{"id"}`; an invalid request is an error with the engine's sentence.
#[tauri::command]
async fn native_start(app: AppHandle, body: Value) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        let body: native::StartBody = serde_json::from_value(body).map_err(|error| format!("JSON body required: {error}"))?;
        let id = built_in(&app).start(body)?;
        Ok(serde_json::json!({ "id": id }))
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, body);
        Err(UNAVAILABLE.into())
    }
}

/// `GET /api/runs/<id>/events?since=N`.
#[tauri::command]
async fn native_events(app: AppHandle, id: String, since: usize) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        serde_json::to_value(built_in(&app).events(&id, since)?).map_err(|error| error.to_string())
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, id, since);
        Err(UNAVAILABLE.into())
    }
}

/// `POST /api/runs/<id>/cancel`.
#[tauri::command]
async fn native_cancel(app: AppHandle, id: String) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        built_in(&app).cancel(&id)?;
        Ok(serde_json::json!({ "id": id, "cancel_requested": true }))
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, id);
        Err(UNAVAILABLE.into())
    }
}

/// `GET /api/runs/<id>/files/<path>`: the file's bytes, not as JSON.
#[tauri::command]
async fn native_file(app: AppHandle, id: String, path: String) -> Result<tauri::ipc::Response, String> {
    #[cfg(native_engine)]
    {
        let file = built_in(&app).file(&id, &path)?;
        std::fs::read(&file).map(tauri::ipc::Response::new).map_err(|_| "not found".to_string())
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, id, path);
        Err(UNAVAILABLE.into())
    }
}

/// `HEAD` of the same: the size in bytes.
#[tauri::command]
async fn native_file_size(app: AppHandle, id: String, path: String) -> Result<u64, String> {
    #[cfg(native_engine)]
    {
        let file = built_in(&app).file(&id, &path)?;
        std::fs::metadata(&file).map(|meta| meta.len()).map_err(|_| "not found".to_string())
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, id, path);
        Err(UNAVAILABLE.into())
    }
}

/// `GET /api/data?path=<relative>`.
#[tauri::command]
async fn native_data(app: AppHandle, path: String) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        serde_json::to_value(built_in(&app).list_data(&path)?).map_err(|error| error.to_string())
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, path);
        Err(UNAVAILABLE.into())
    }
}

/// Asks an external program of the photos start about itself: `alicevision`, `colmap` or `sam`.
/// Uses the saved settings; returns `{ok, summary, detail}`.
#[tauri::command]
async fn check_tool(app: AppHandle, tool: String) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        let settings = settings(&app)?;
        // Whatever the answer, the form's list of providers is made anew: the tool may have been installed since.
        built_in(&app).forget_tools();
        serde_json::to_value(tools::check(&tool, &settings.tools, &settings.resolved.repo.value)).map_err(|error| error.to_string())
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, tool);
        Err(UNAVAILABLE.into())
    }
}

/// One file chosen with the system's file picker on a phone, sent as raw bytes with the
/// headers `x-folder` and `x-name` (percent-encoded); written to `<data>/<folder>/<name>`.
/// Returns the path relative to the data folder.
#[tauri::command]
async fn native_import(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        let header = |name: &str| -> Result<String, String> {
            let raw = request.headers().get(name).and_then(|value| value.to_str().ok()).ok_or(format!("{name} is missing"))?;
            percent_decode(raw).ok_or(format!("{name} is not valid"))
        };
        let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
            return Err("the file must be sent as raw bytes".into());
        };
        let path = built_in(&app).import(&header("x-folder")?, &header("x-name")?, bytes)?;
        Ok(serde_json::json!({ "path": path }))
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, request);
        Err(UNAVAILABLE.into())
    }
}

/// Removes a downloaded example object (`<data>/examples/<id>`).
#[tauri::command]
async fn native_delete_example(app: AppHandle, id: String) -> Result<(), String> {
    #[cfg(native_engine)]
    {
        built_in(&app).delete_example(&id)
    }
    #[cfg(not(native_engine))]
    {
        let _ = (app, id);
        Err(UNAVAILABLE.into())
    }
}

/// Free bytes on the disk of the data folder, or null where the platform does not say.
#[tauri::command]
async fn native_free_space(app: AppHandle) -> Result<Option<u64>, String> {
    #[cfg(native_engine)]
    {
        Ok(built_in(&app).free_bytes())
    }
    #[cfg(not(native_engine))]
    {
        let _ = app;
        Err(UNAVAILABLE.into())
    }
}

/// `%E2%82%AC` -> `€`: what `encodeURIComponent` wrote.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Lens calibration files found next to the app: in the crispmesh checkout and in the data
/// folder. What is listed may be used as a run's calibration.
#[tauri::command]
async fn native_calibrations(app: AppHandle) -> Result<Value, String> {
    #[cfg(native_engine)]
    {
        let settings = settings(&app)?;
        let data = std::path::PathBuf::from(&settings.resolved.data_dir.value);
        let mut folders = vec![data.clone(), data.join("calibrations")];
        if !settings.resolved.repo.value.is_empty() {
            folders.push(std::path::Path::new(&settings.resolved.repo.value).join("scripts/turntable_mesh/calibrations"));
        }
        let found = tools::calibrations(&folders);
        let engine = built_in(&app);
        for calibration in &found {
            engine.grant(std::path::Path::new(&calibration.path));
        }
        serde_json::to_value(found).map_err(|error| error.to_string())
    }
    #[cfg(not(native_engine))]
    {
        let _ = app;
        Err(UNAVAILABLE.into())
    }
}

// ---------------------------------------------------------------------------------------------

/// A request to an engine at a plain `http://` address, made by the app instead of the web view (see remote.rs).
/// The request: headers `x-url`, `x-method`, `x-headers` (percent-encoded JSON object), the body raw. The answer:
/// `remote::frame` of status, headers and body.
#[tauri::command]
async fn remote_fetch(request: tauri::ipc::Request<'_>) -> Result<tauri::ipc::Response, String> {
    let header = |name: &str| -> Result<String, String> {
        let raw = request.headers().get(name).and_then(|value| value.to_str().ok()).ok_or(format!("{name} is missing"))?;
        percent_decode(raw).ok_or(format!("{name} is not valid"))
    };
    let (url, method) = (header("x-url")?, header("x-method")?);
    let headers: Vec<(String, String)> = serde_json::from_str::<serde_json::Map<String, Value>>(&header("x-headers")?)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|(name, value)| value.as_str().map(|value| (name, value.to_string())))
        .collect();
    let body = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.clone(),
        _ => Vec::new(),
    };
    let answer = tauri::async_runtime::spawn_blocking(move || remote::fetch(&method, &url, &headers, &body))
        .await
        .map_err(|error| error.to_string())??;
    Ok(tauri::ipc::Response::new(remote::frame(answer.0, &answer.1, &answer.2)))
}

/// "Let other devices use this computer" (desktop): `on` true starts serving the built-in engine on the local
/// network with a new token, false stops; absent, only says. Answers `{running, port, token, addresses}`.
#[tauri::command]
fn share_engine(app: AppHandle, on: Option<bool>, port: Option<u16>) -> Result<Value, String> {
    #[cfg(all(native_engine, desktop))]
    {
        let state = app.state::<share::Shared>();
        let mut current = state.0.lock().map_err(|_| "the sharing state is not available".to_string())?;
        match on {
            Some(true) => {
                if let Some(running) = current.take() {
                    running.stop();
                }
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes).map_err(|error| format!("No random numbers from the system: {error}"))?;
                let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                *current = Some(share::Sharing::start(built_in(&app), port.unwrap_or(8766), token)?);
            }
            Some(false) => {
                if let Some(running) = current.take() {
                    running.stop();
                }
            }
            None => {}
        }
        Ok(current.as_ref().map(share::Sharing::describe).unwrap_or_else(|| serde_json::json!({ "running": false })))
    }
    #[cfg(not(all(native_engine, desktop)))]
    {
        let _ = (app, on, port);
        Err("Only the desktop app can share its engine.".into())
    }
}

/// Debug builds only: lets a test script that the shell injected report back on stderr.
#[cfg(debug_assertions)]
#[tauri::command]
fn autopilot_log(app: AppHandle, line: String) {
    eprintln!("[autopilot] {line}");
    // Also into a file the host can read: on an iOS simulator or in the App Sandbox the
    // app's standard error does not reach whoever started it.
    if let Ok(folder) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&folder);
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(folder.join("autopilot.log")) {
            use std::io::Write as _;
            let _ = writeln!(file, "[autopilot] {line}");
        }
    }
}

/// Debug builds only: waits until whoever drives the app has taken the screenshot named
/// `name`, which it says by creating `<app data>/ack-<name>`; at most `limit_ms`. Returns
/// whether it was acknowledged. Lets a slow machine photograph every screen.
#[cfg(debug_assertions)]
#[tauri::command]
async fn autopilot_await(app: AppHandle, name: String, limit_ms: u64) -> bool {
    let Ok(folder) = app.path().app_data_dir() else { return false };
    let flag = folder.join(format!("ack-{}", name.replace(['/', '\\', '.'], "-")));
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(limit_ms);
    while std::time::Instant::now() < deadline {
        if flag.exists() {
            let _ = std::fs::remove_file(&flag);
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Debug builds only: quits the app the way the menu does, so the exit path can be tested.
#[cfg(debug_assertions)]
#[tauri::command]
fn autopilot_quit(app: AppHandle, after_ms: Option<u64>) {
    // Timed here, not in the page: a web view that is not visible may not run its timers.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(after_ms.unwrap_or(0)));
        app.exit(0);
    });
}

/// Debug builds only: a pause timed here, because a web view that is not visible (another
/// window in front, a locked screen) slows or stops its own timers.
#[cfg(debug_assertions)]
#[tauri::command]
async fn autopilot_sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// Debug builds only: memory of this process in bytes (resident now, and its peak).
#[cfg(debug_assertions)]
#[tauri::command]
fn autopilot_memory() -> (u64, u64) {
    #[cfg(unix)]
    {
        // SAFETY: getrusage fills the struct it is given.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        // macOS reports bytes, Linux kilobytes.
        let peak = usage.ru_maxrss as u64 * if cfg!(target_os = "macos") || cfg!(target_os = "ios") { 1 } else { 1024 };
        (0, peak)
    }
    #[cfg(not(unix))]
    (0, 0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_dialog::init());
    #[cfg(local_engine)]
    let builder = builder.manage(engine::Engine::default());
    #[cfg(native_engine)]
    let builder = builder.manage(std::sync::Arc::new(native::Native::default()));
    #[cfg(all(native_engine, desktop))]
    let builder = builder.manage(share::Shared::default());

    let builder = builder.setup(|app| {
        #[cfg(local_engine)]
        engine::install_signal_handlers();
        if let Err(error) = configure_native(app.handle()) {
            eprintln!("crispmesh studio: {error}");
        }
        // Debug builds only: CRISPMESH_STUDIO_AUTORUN=<start request as JSON> starts that run on
        // the built-in engine without the window taking part, reports on stderr and quits.
        // For measuring the engine inside the app when no screen is available to drive the UI.
        #[cfg(all(debug_assertions, native_engine))]
        if let Some(body) = std::env::var("CRISPMESH_STUDIO_AUTORUN").ok().and_then(|text| serde_json::from_str::<native::StartBody>(&text).ok()) {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let engine = built_in(&handle);
                let began = std::time::Instant::now();
                match engine.start(body) {
                    Ok(id) => {
                        eprintln!("[autorun] started {id}");
                        let mut seen = 0;
                        loop {
                            let page = engine.events(&id, seen).unwrap_or(native::EventPage { events: Vec::new(), next: seen });
                            for event in &page.events {
                                if event["type"] == "artifact" || event["type"] == "stage_finished" || event["type"] == "error" || event["type"] == "run_finished" {
                                    let what = event["label"].as_str().or(event["message"].as_str()).or(event["status"].as_str()).unwrap_or("");
                                    eprintln!("[autorun] {:6.1} s  {} {} {what}", began.elapsed().as_secs_f64(), event["type"].as_str().unwrap_or(""), event["stage"].as_str().unwrap_or(""));
                                }
                            }
                            seen = page.next;
                            if engine.running() == 0 {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(200));
                        }
                        eprintln!("[autorun] ended after {:.1} s; peak resident memory {} MB", began.elapsed().as_secs_f64(), autopilot_memory().1 / 1_000_000);
                    }
                    Err(error) => eprintln!("[autorun] refused: {error}"),
                }
                handle.exit(0);
            });
        }
        Ok(())
    });

    // Debug builds only: CRISPMESH_STUDIO_AUTOPILOT=<file.js> runs that script in the window
    // once the page has loaded, so the real app can be driven end to end without a human.
    #[cfg(debug_assertions)]
    let builder = builder.on_page_load(|webview, payload| {
        if payload.event() != tauri::webview::PageLoadEvent::Finished {
            return;
        }
        if let Some(script) = std::env::var_os("CRISPMESH_STUDIO_AUTOPILOT").and_then(|file| std::fs::read_to_string(file).ok()) {
            let _ = webview.eval(script);
        }
    });

    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        shell_info,
        get_settings,
        save_settings,
        pick_path,
        engine_status,
        restart_engine,
        stop_engine,
        native_health,
        native_settings,
        native_runs,
        native_start,
        native_events,
        native_cancel,
        native_file,
        native_file_size,
        native_data,
        native_calibrations,
        native_import,
        share_engine,
        remote_fetch,
        native_delete_example,
        native_free_space,
        check_tool,
        autopilot_log,
        autopilot_await,
        autopilot_quit,
        autopilot_sleep,
        autopilot_memory
    ]);
    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        shell_info,
        get_settings,
        save_settings,
        pick_path,
        engine_status,
        restart_engine,
        stop_engine,
        native_health,
        native_settings,
        native_runs,
        native_start,
        native_events,
        native_cancel,
        native_file,
        native_file_size,
        native_data,
        native_calibrations,
        native_import,
        share_engine,
        remote_fetch,
        native_delete_example,
        native_free_space,
        check_tool
    ]);

    let app = builder.build(tauri::generate_context!()).expect("error while building CrispMesh");
    app.run(|_handle, _event| {
        if let tauri::RunEvent::Exit = _event {
            // Runs of the built-in engine are asked to stop and given a moment; whatever is
            // still busy then has its log closed, because the process ends next.
            #[cfg(native_engine)]
            built_in(_handle).shutdown(std::time::Duration::from_secs(8));
            // Blocks until the Python engine and its process group are gone.
            #[cfg(local_engine)]
            _handle.state::<engine::Engine>().stop();
        }
    });
}
