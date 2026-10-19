//! A whole run in one process: port of `scripts/turntable_mesh/dense_pipeline.py`
//! for the `--inputs` and `--scene`/`--prepared`/`--raw-masks` starting points.
//!
//! Stages run in order on the calling thread: `inputs` (only from a scene),
//! `hull` (or optional `stereo`), `mesh`, optional photo refinement, `check`. Preview volumes written while matching runs are
//! meshed coarsely on a worker thread. The run directory follows
//! `docs/ENGINE-CONTRACT.md`: `config.json`, `events.jsonl` (always closed by
//! `run_finished` once `run_started` was written), `pipeline.json`, a `cancel`
//! file that stops the run. An embedding application calls [`run`] on a worker
//! thread with an event observer and a cancel flag.
//!
//! Not here: masks and cameras from plain photos (SAM, AliceVision) and scoring
//! against a reference scan; those stay in the Python driver.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use web_time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::check;
use crate::config::DenseConfig;
use crate::control::{Control, Stopped};
use crate::events::{EventLog, Observer, SCHEMA};
use crate::gpu::Gpu;
use crate::inputs::load_views;
use crate::render::Rgb;
use crate::scene::{self, read_gray};
use crate::stereo;

/// Interchangeable base reconstruction paths. Gaussian/neural refinement is optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSurface {
    #[default]
    Silhouette,
    Stereo,
}

/// What to run. Either `inputs`, or all of `scene`, `prepared` and `raw_masks`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunOptions {
    /// Fresh run directory.
    pub output: PathBuf,
    /// Existing inputs directory (`cameras.json`, `sparse_points.npy`).
    pub inputs: Option<PathBuf>,
    /// AliceVision `.sfm` with poses and one `radialk3` intrinsic.
    pub scene: Option<PathBuf>,
    /// Undistorted images named `<viewId>.png`.
    pub prepared: Option<PathBuf>,
    /// 0/255 masks named like the source photos.
    pub raw_masks: Option<PathBuf>,
    /// Folder of turntable photos: the photos stage (masks, cameras) runs first
    /// and its scene is the inputs directory. In a browser, or with an output in
    /// the in-memory tree, only providers without external programs can run.
    pub photos: Option<PathBuf>,
    /// Command-line options of the photos stage (`crispmesh-dense photos --help`):
    /// `--calibration FILE`, `--masks PROVIDER`, `--cameras PROVIDER` and the
    /// providers' own options, one word per element.
    pub photo_options: Vec<String>,
    pub base_surface: BaseSurface,
    /// JSON with settings (a `config.json` or a previous `result.json`).
    pub config: Option<PathBuf>,
    /// A named set of settings applied before `overrides` (`detail`; see [`PRESETS`]).
    pub preset: Option<String>,
    /// `key=value` overrides, applied after `config` and the preset.
    pub overrides: Vec<String>,
    /// Settings by name, as `GET /api/settings` lists them; applied after `overrides`.
    pub settings: serde_json::Map<String, Value>,
    /// Worker threads of the surface stage.
    pub threads: usize,
    /// Seconds the stereo stage may take.
    pub stereo_timeout: f64,
    pub minimum_free_gib: f64,
    /// `depths.npz` of an earlier run on the same inputs: skip matching, fuse only.
    pub reuse_depths: Option<PathBuf>,
    /// Mesh intermediate surfaces while matching runs.
    pub live_previews: bool,
    /// Marching-cubes step of live preview meshes.
    pub preview_step: usize,
    /// Run the photo check.
    pub check: bool,
    /// Write the check's overlay and preview sheets.
    pub preview: bool,
    /// Keep `stereo/volume.npz` for re-meshing.
    pub keep_volume: bool,
    /// Export a photo-textured GLB after meshing (additional memory and time).
    pub texture: bool,
    /// Source credit retained with the run and embedded in textured export.
    pub attribution: Option<String>,
    /// The neural surface after the mesh stage (experimental): `surface/mesh.stl`.
    pub surface: Option<Method>,
    /// 2D Gaussian splats after the mesh stage (experimental): `splats/mesh.stl` and `splats.npz`.
    pub splats: Option<Method>,
}

/// An optional photo-optimised surface of a run: its quality level (`capture`, `fast`, `standard`, `best`) and single settings
/// (`NAME=VALUE`, as for `crispmesh-dense surface` / `splats`) applied after the level's.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Method {
    pub quality: String,
    pub settings: Vec<String>,
}

impl Default for Method {
    fn default() -> Self {
        Self { quality: "capture".into(), settings: Vec::new() }
    }
}

impl Method {
    /// From a command-line value: a quality level, or a number of steps (with the standard level).
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        if value.parse::<u32>().is_ok() {
            return Ok(Method { quality: "standard".into(), settings: vec![format!("steps={value}")] });
        }
        value.parse::<crate::surface::options::Quality>()?;
        Ok(Method { quality: value.to_string(), settings: Vec::new() })
    }

    fn arguments(&self) -> Vec<String> {
        std::iter::once(format!("quality={}", self.quality)).chain(self.settings.iter().cloned()).collect()
    }
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            output: PathBuf::new(),
            inputs: None,
            scene: None,
            prepared: None,
            raw_masks: None,
            photos: None,
            photo_options: Vec::new(),
            base_surface: BaseSurface::default(),
            config: None,
            preset: None,
            overrides: Vec::new(),
            settings: serde_json::Map::new(),
            threads: 2,
            stereo_timeout: 3600.0,
            minimum_free_gib: 2.0,
            reuse_depths: None,
            live_previews: true,
            preview_step: 2,
            check: true,
            preview: true,
            keep_volume: false,
            texture: false,
            attribution: None,
            surface: None,
            splats: None,
        }
    }
}

impl RunOptions {
    fn silhouette_only(&self) -> bool {
        self.base_surface == BaseSurface::Silhouette
    }

    /// The resolved settings: defaults, `config`, `overrides`, `settings`.
    pub fn configuration(&self) -> anyhow::Result<DenseConfig> {
        let mut overrides: Vec<String> = if self.silhouette_only() {
            ["grid=192", "repair_masks=false", "mask_grow=false", "silhouette_first=0", "photo_refine=0", "mesh_open_unseen=false"]
                .map(String::from)
                .to_vec()
        } else {
            Vec::new()
        };
        overrides.extend(match self.preset.as_deref() {
            None => Vec::new(),
            Some(name) => match PRESETS.iter().find(|(n, _, _)| *n == name) {
                Some((_, settings, _)) => settings.iter().map(|s| s.to_string()).collect(),
                None => anyhow::bail!("unknown preset {name:?}; known: {}", PRESETS.iter().map(|p| p.0).collect::<Vec<_>>().join(", ")),
            },
        });
        overrides.extend(self.overrides.iter().cloned());
        for (key, value) in &self.settings {
            let text = match value {
                Value::Array(items) => items.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            overrides.push(format!("{key}={text}"));
        }
        stereo::options::build(self.config.as_deref(), &overrides)
    }
}

/// Named sets of settings: name, settings, what they trade.
pub const PRESETS: &[(&str, &[&str], &str)] = &[(
    "detail",
    &["band_first=8,8,12"],
    "Searches 12 inverse-depth steps either side of the coarser surface at the finest levels instead of 5: more consistent depth \
     on fine relief (Happy Buddha coverage at native pixels 0.53 to 0.66, F1 at 0.5 % +0.011 whole, +0.017 above the support) \
     at the cost of more stray depth (F1 at 2 % -0.006)",
)];

/// Contact sheet of evenly spaced photos with their mask outlines (`input_sheet`).
pub fn input_sheet(inputs: &Path, path: &Path) -> anyhow::Result<()> {
    const COUNT: usize = 8;
    const COLUMNS: usize = 4;
    let rows = load_views(inputs)?;
    let shown = COUNT.min(rows.len());
    let picks: Vec<usize> = (0..shown).map(|n| (n as f64 * (rows.len() as f64 / shown as f64)) as usize).collect();
    let files: Vec<PathBuf> = picks.iter().flat_map(|&n| [PathBuf::from(&rows[n].image), PathBuf::from(&rows[n].mask)]).collect();
    let held = crate::inputs::hold_for_pool(&files)?;
    let tiles: Vec<anyhow::Result<Rgb>> = crate::inputs::parallel_map(picks.len(), |n| {
        let row = &rows[picks[n]];
        let mut photo = Rgb::open(Path::new(&row.image))?;
        let mask = read_gray(Path::new(&row.mask))?;
        if (mask.width, mask.height) != (photo.width, photo.height) {
            bail!("photo and mask sizes differ for {}", row.name);
        }
        // Mask pixels with a background neighbour, drawn wide enough to survive the reduction to the tile.
        let (w, h) = (mask.width, mask.height);
        let inside = |x: i64, y: i64| x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && mask.data[y as usize * w + x as usize] > 127;
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                if inside(x, y) && !(inside(x - 1, y) && inside(x + 1, y) && inside(x, y - 1) && inside(x, y + 1)) {
                    for (dx, dy) in (-1..=2).flat_map(|dy| (-1..=2).map(move |dx| (dx, dy))) {
                        if x + dx >= 0 && y + dy >= 0 && x + dx < w as i64 && y + dy < h as i64 {
                            photo.set((x + dx) as usize, (y + dy) as usize, [60, 220, 60]);
                        }
                    }
                }
            }
        }
        let height = (crate::inputs::round_half_even(420.0 * photo.height as f64 / photo.width as f64) as usize).max(1);
        Ok(photo.resize(420, height))
    });
    crate::inputs::release(held);
    let tiles: Vec<Rgb> = tiles.into_iter().collect::<anyhow::Result<_>>()?;
    let height = tiles.first().map(|t| t.height).ok_or_else(|| anyhow!("no views"))?;
    let mut sheet = Rgb::filled(420 * COLUMNS, height * tiles.len().div_ceil(COLUMNS), [30; 3]);
    for (n, tile) in tiles.iter().enumerate() {
        let clipped = tile.crop(0, 0, tile.width, tile.height.min(height));
        sheet.paste(&clipped, (n % COLUMNS) * 420, (n / COLUMNS) * height);
    }
    sheet.save(path)
}

/// Preview volumes waiting to be meshed coarsely. With threads a worker takes
/// them as they are announced; without (wasm32) the driver meshes them itself
/// at the next stage boundary.
#[derive(Default)]
struct PreviewQueue {
    state: Mutex<PreviewState>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct PreviewState {
    waiting: std::collections::VecDeque<(PathBuf, String)>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    meshing: bool,
    closed: bool,
    /// Set when the run failed or was cancelled: waiting volumes are dropped.
    abandoned: bool,
}

struct PreviewMesher {
    queue: Arc<PreviewQueue>,
    events: EventLog,
    config: DenseConfig,
    step: usize,
    threads: usize,
    /// Stops a preview mesh in progress: the run's cancel file, and `stop`, which `settle` sets when the
    /// run is cancelled or has failed, so that the run ends without waiting for a mesh nobody will see.
    control: Control,
    stop: Arc<AtomicBool>,
    /// Draws an inspection sheet of every preview surface, once the run's views are known.
    inspector: std::sync::OnceLock<Arc<crate::inspect::Inspector>>,
}

impl PreviewMesher {
    fn lock(&self) -> std::sync::MutexGuard<'_, PreviewState> {
        self.queue.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn mesh(&self, volume: &Path, label: &str) {
        let output = volume.with_extension("");
        let (config, events, control) = (&self.config, &self.events, &self.control);
        let result = match crate::mesh::field::Volume::take_handed_over(volume) {
            // A coarse preview is meshed on a coarser grid, not as every step-th cell of the full one:
            // the same coarseness for a fraction of the memory, which a browser run needs.
            Some(held) => crate::mesh::run_preview(held, &output, config, self.step, events, Some(label), self.threads, control),
            None => crate::mesh::run_with(volume, &output, config, self.step, events, Some(label), self.threads, control),
        };
        if let (Ok(_), Some(inspector)) = (&result, self.inspector.get()) {
            if crate::inspect::memory_allows() {
                let step = volume.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let _ = inspector.add(&step, label, &output.join("mesh.stl"), &self.events);
            }
        }
        let log = match &result {
            Ok(report) => serde_json::to_string_pretty(report).unwrap_or_default(),
            Err(error) => format!("{error:#}"),
        };
        let _ = crate::storage::write(volume.with_extension("log"), log + "\n");
        // The volume has served its purpose; it is large, and without a disk it occupies memory.
        let _ = crate::storage::remove_file(volume);
    }

    /// The worker: meshes volumes until the queue is closed and empty.
    #[cfg(not(target_arch = "wasm32"))]
    fn work(&self) {
        loop {
            let next = {
                let mut state = self.lock();
                loop {
                    if state.abandoned {
                        state.waiting.clear();
                    }
                    if let Some(next) = state.waiting.pop_front() {
                        state.meshing = true;
                        break Some(next);
                    }
                    if state.closed {
                        break None;
                    }
                    state = self.queue.changed.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            let Some((volume, label)) = next else { return };
            self.mesh(&volume, &label);
            self.lock().meshing = false;
            self.queue.changed.notify_all();
        }
    }

    /// Returns when no preview is waiting or being meshed. `abandon` drops the waiting ones.
    fn settle(&self, abandon: bool) {
        if abandon {
            self.stop.store(true, Ordering::Relaxed);
            let mut state = self.lock();
            state.abandoned = true;
            state.waiting.clear();
            crate::mesh::field::Volume::drop_handed_over();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.queue.changed.notify_all();
            let mut state = self.lock();
            while state.meshing || !state.waiting.is_empty() {
                state = self.queue.changed.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        #[cfg(target_arch = "wasm32")]
        loop {
            let next = self.lock().waiting.pop_front();
            let Some((volume, label)) = next else { break };
            self.mesh(&volume, &label);
        }
    }

    fn close(&self) {
        self.lock().closed = true;
        self.queue.changed.notify_all();
    }
}

struct Driver {
    output: PathBuf,
    events: EventLog,
    report: Value,
    started: Instant,
    cancel: Option<Arc<AtomicBool>>,
    finished: bool,
    previews: Arc<PreviewMesher>,
}

/// A stage that has started: its events, its control and its clock.
struct Stage {
    name: String,
    command: Vec<String>,
    events: EventLog,
    control: Control,
    started: Instant,
}

impl Driver {
    fn write_report(&self) -> anyhow::Result<()> {
        crate::storage::write(self.output.join("pipeline.json"), serde_json::to_string_pretty(&self.report)? + "\n")?;
        Ok(())
    }

    /// Closes the run: `pipeline.json`, then `run_finished` as the last event (previews are settled first).
    fn finish(&mut self, status: &str) -> anyhow::Result<()> {
        self.finished = true;
        self.previews.settle(status != "complete");
        self.report["status"] = json!(status);
        self.report["seconds"] = json!(self.started.elapsed().as_secs_f64());
        self.write_report()?;
        let cancelled = Control::new(self.cancel.clone(), Some(self.output.join("cancel")), None, None)?.cancelled();
        let outcome = if status == "complete" {
            "complete"
        } else if cancelled {
            "cancelled"
        } else {
            "failed"
        };
        self.events.emit("run_finished", json!({"status": outcome, "seconds": self.report["seconds"]}))
    }

    /// Starts a stage: `stage_started`, its log file, its deadline.
    fn begin(&mut self, name: &str, command: Vec<String>, timeout: Option<f64>) -> anyhow::Result<Stage> {
        println!("[{name}] ...");
        let events = self.events.stage(name);
        events.emit("stage_started", json!({}))?;
        let deadline = timeout.map(|seconds| Instant::now() + Duration::from_secs_f64(seconds.max(0.0)));
        let log = self.output.join(format!("{name}.log"));
        let control = Control::new(self.cancel.clone(), Some(self.output.join("cancel")), deadline, Some(&log))?;
        Ok(Stage { name: name.to_string(), command, events, control, started: Instant::now() })
    }

    /// Ends a stage with its result: its entry in `pipeline.json`, `stage_finished` or `error` and the end of the run.
    fn end<T>(&mut self, stage: Stage, result: anyhow::Result<T>) -> anyhow::Result<T> {
        let Stage { name, command, events, control, started } = stage;
        let seconds = started.elapsed().as_secs_f64();
        let stopped = result.as_ref().err().and_then(|e| e.downcast_ref::<Stopped>().copied());
        if let Err(error) = &result {
            control.log(format!("{error:#}"));
        }
        self.report["stages"][&name] = json!({
            "command": command,
            "exit_code": match (&result, stopped) { (Ok(_), _) => json!(0), (_, Some(_)) => Value::Null, _ => json!(1) },
            "timed_out": stopped == Some(Stopped::Deadline),
            "cancelled": stopped == Some(Stopped::Cancelled),
            "seconds": seconds,
            "log": self.output.join(format!("{name}.log")).display().to_string(),
        });
        self.write_report()?;
        match result {
            Ok(value) => {
                events.emit("stage_finished", json!({"seconds": seconds}))?;
                println!("[{name}] {seconds:.1}s");
                Ok(value)
            }
            Err(error) => {
                let message = match stopped {
                    Some(Stopped::Cancelled) => "cancelled on request".to_string(),
                    Some(Stopped::Deadline) => format!("deadline: {error:#}"),
                    None => format!("{error:#}"),
                };
                events.emit("error", json!({"message": message}))?;
                self.finish(&format!("failed in {name}"))?;
                Err(error.context(format!("stage {name} failed")))
            }
        }
    }
}

fn text(path: &Path) -> String {
    path.display().to_string()
}

/// Runs the pipeline into the fresh directory `options.output` and returns the
/// content of `pipeline.json`. Every event is appended to `events.jsonl` there
/// and passed to `observer`; setting `cancel` (or creating the `cancel` file)
/// stops the run at the next view while matching, or between stages otherwise.
///
/// Blocks the calling thread until the run is over; call it on a worker thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(options: &RunOptions, observer: Option<Observer>, cancel: Option<Arc<AtomicBool>>) -> anyhow::Result<Value> {
    crate::gpu::block_on(run_async(options, observer, cancel))
}

/// [`run`] as a future, for hosts that cannot block (a browser). Paths may
/// name files of the in-memory tree (`crate::storage`); in a browser they all do.
pub async fn run_async(options: &RunOptions, observer: Option<Observer>, cancel: Option<Arc<AtomicBool>>) -> anyhow::Result<Value> {
    anyhow::ensure!(options.attribution.as_ref().is_none_or(|text| text.len() <= 65536), "attribution exceeds 64 KiB");
    let config = options.configuration()?;
    if options.output.as_os_str().is_empty() {
        bail!("an output directory is required");
    }
    let output = crate::storage::absolute(&options.output)?;
    if crate::storage::exists(&output) {
        bail!("output directory exists: {}", output.display());
    }
    let from_scene = options.scene.is_some() && options.prepared.is_some() && options.raw_masks.is_some();
    if options.inputs.is_none() && !from_scene && options.photos.is_none() {
        bail!("give photos, or inputs, or all of scene, prepared and raw_masks");
    }
    let photo_stage = match &options.photos {
        Some(photos) => Some(photo_stage_options(options, photos, &output)?),
        None => None,
    };
    #[cfg(not(target_arch = "wasm32"))]
    if !crate::storage::is_memory(&output) {
        let parent = output.parent().filter(|p| crate::storage::exists(p)).map(Path::to_path_buf).unwrap_or(std::env::current_dir()?);
        let free = fs4::available_space(&parent).with_context(|| parent.display().to_string())? as f64 / (1u64 << 30) as f64;
        if free < options.minimum_free_gib {
            bail!("only {free:.1} GiB free; need {} (see minimum_free_gib)", options.minimum_free_gib);
        }
    }
    crate::storage::create_dir_all(&output).with_context(|| output.display().to_string())?;
    if let Some(text) = &options.attribution {
        crate::storage::write_new(output.join("ATTRIBUTION.txt"), text.as_bytes().to_vec())?;
    }
    // Event paths are relative to the run directory as the file system names it.
    let output = crate::storage::canonicalize(&output)?;
    crate::storage::write(output.join("config.json"), serde_json::to_string_pretty(&config)? + "\n")?;

    // Preview volumes announced by the stereo stage are queued for coarse meshing.
    let queue = Arc::new(PreviewQueue::default());
    let (announced, run_directory) = (queue.clone(), output.clone());
    // Without a second thread a volume is meshed as soon as it is announced, so that it does not stay in memory.
    let inline: Arc<std::sync::OnceLock<Arc<PreviewMesher>>> = Arc::new(std::sync::OnceLock::new());
    let meshing_inline = inline.clone();
    let watching: Observer = Arc::new(move |event: &Value| {
        if event["type"] == "artifact" && event["kind"] == "preview_volume" {
            if let (Some(path), Some(label)) = (event["path"].as_str(), event["label"].as_str()) {
                if let Some(mesher) = meshing_inline.get() {
                    if let Some(observer) = &observer {
                        observer(event);
                    }
                    mesher.mesh(&run_directory.join(path), label);
                    return;
                }
                let mut state = announced.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if !state.abandoned {
                    state.waiting.push_back((run_directory.join(path), label.to_string()));
                }
                drop(state);
                announced.changed.notify_all();
            }
        }
        if let Some(observer) = &observer {
            observer(event);
        }
    });
    let events = EventLog::for_run(&output, Some(watching));
    let stop = Arc::new(AtomicBool::new(false));
    let mesher = Arc::new(PreviewMesher {
        control: Control::new(Some(stop.clone()), Some(output.join("cancel")), None, None)?,
        stop,
        queue,
        inspector: std::sync::OnceLock::new(),
        events: events.stage("stereo"),
        config: config.clone(),
        step: options.preview_step.max(1),
        threads: options.threads.max(1),
    });
    if cfg!(target_arch = "wasm32") {
        let _ = inline.set(mesher.clone());
    }
    let mut driver = Driver {
        output: output.clone(),
        events: events.clone(),
        report: json!({"output": text(&output), "device": "wgpu", "stages": {}, "status": "running"}),
        started: Instant::now(),
        cancel,
        finished: false,
        previews: mesher.clone(),
    };
    let source = options.photos.as_ref().or(options.inputs.as_ref()).or(options.scene.as_ref()).expect("checked above");
    let source_name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

    // With threads, previews are meshed on a second one while matching continues.
    #[cfg(not(target_arch = "wasm32"))]
    let worker = {
        let mesher = mesher.clone();
        std::thread::Builder::new().name("crispmesh-previews".into()).spawn(move || mesher.work())?
    };
    let outcome = stages(options, photo_stage.as_ref(), &config, &output, &mut driver, source_name).await;
    let outcome = close(outcome, &events, &mut driver);
    mesher.close();
    #[cfg(not(target_arch = "wasm32"))]
    let _ = worker.join();
    // Nothing handed over may outlive the run (a stopped run leaves volumes nobody meshes).
    crate::mesh::field::Volume::drop_handed_over();
    for entry in crate::storage::list(output.join("stereo/preview")).unwrap_or_default() {
        if entry.extension().is_some_and(|e| e == "npz") {
            let _ = crate::storage::remove_file(entry);
        }
    }
    outcome?;
    Ok(driver.report)
}

/// Anything that failed outside a stage must still close the event log.
fn close(outcome: anyhow::Result<()>, events: &EventLog, driver: &mut Driver) -> anyhow::Result<()> {
    if let Err(error) = &outcome {
        if !driver.finished {
            let _ = events.emit("error", json!({"message": format!("{error:#}")}));
            let _ = driver.finish("failed before or between stages");
        }
    }
    outcome
}

/// The photos stage's options for this run: its command line with the run's directories filled in.
/// Resolved before anything is written, so that a bad option or a missing program fails early.
fn photo_stage_options(options: &RunOptions, photos: &Path, output: &Path) -> anyhow::Result<crate::photos::options::Options> {
    use crate::photos::{cameras::camera_provider, masks::mask_provider, options::resolve};
    let mut arguments: Vec<String> =
        ["--threads", &options.threads.to_string(), "--minimum-free-gib", &options.minimum_free_gib.to_string()].map(String::from).to_vec();
    arguments.extend(options.photo_options.iter().cloned());
    for (flag, value) in [("--photos", photos), ("--output", &output.join("frontend")), ("--events", &output.join("events.jsonl"))] {
        arguments.extend([flag.to_string(), text(value)]);
    }
    let resolved = resolve(&arguments, &|name| std::env::var(name).ok()).context("photos stage options")?;
    if resolved.stop_after_masks {
        bail!("--stop-after masks leaves no scene to reconstruct; use the photos command for that");
    }
    let (masks, cameras) = (mask_provider(&resolved), camera_provider(&resolved));
    // External programs need a process and a real directory; ask the provider table which do.
    if cfg!(target_arch = "wasm32") || crate::storage::is_memory(output) {
        for provider in [masks.info(), cameras.info()] {
            if !provider.external.is_empty() {
                bail!(
                    "--{} {} starts external programs ({}); in a browser or with an in-memory run directory only providers without \
                     one can run (see --list-providers)",
                    provider.module,
                    provider.name,
                    provider.external.join(", ")
                );
            }
        }
    }
    masks.check(&resolved)?;
    cameras.check(&resolved)?;
    Ok(resolved)
}

async fn stages(
    options: &RunOptions,
    photo_stage: Option<&crate::photos::options::Options>,
    config: &DenseConfig,
    output: &Path,
    driver: &mut Driver,
    source_name: String,
) -> anyhow::Result<()> {
    let gpu = Gpu::request().await;
    let device = match &gpu {
        Ok(gpu) => format!("wgpu: {}", gpu.describe()),
        Err(_) => "wgpu".to_string(),
    };
    driver.report["device"] = json!(device);
    driver.events.emit(
        "run_started",
        json!({"schema": SCHEMA, "configuration": serde_json::to_value(config)?, "base_surface": options.base_surface, "device": device, "inputs": source_name}),
    )?;

    // From plain photos: masks and cameras first. That stage writes its own `masks` and `cameras` stage events.
    let from_photos = match photo_stage {
        Some(photo_options) => {
            println!("[photos] ...");
            let started = Instant::now();
            let events_path = output.join("events.jsonl");
            let finished = crate::photos::run::run(photo_options, &driver.events, &events_path, driver.cancel.clone());
            let seconds = started.elapsed().as_secs_f64();
            let cancelled = Control::new(driver.cancel.clone(), Some(output.join("cancel")), None, None)?.cancelled();
            let code = finished.as_ref().map(|f| f.code).unwrap_or(1);
            driver.report["stages"]["photos"] = json!({
                "command": std::iter::once("crispmesh-dense photos".to_string()).chain(options.photo_options.iter().cloned()).collect::<Vec<_>>(),
                "exit_code": if cancelled { Value::Null } else { json!(code) },
                "timed_out": false, "cancelled": cancelled, "seconds": seconds,
                "log": text(&output.join("frontend/logs")),
            });
            driver.write_report()?;
            if code != 0 {
                let reason = match &finished {
                    Ok(finished) => {
                        format!("photos stage ended with status {} ({})", finished.report["status"], finished.report["reasons"])
                    }
                    Err(error) => format!("{error:#}"),
                };
                if cancelled {
                    driver.events.stage("cameras").emit("error", json!({"message": "cancelled on request"}))?;
                } else if finished.is_err() {
                    driver.events.stage("cameras").emit("error", json!({"message": reason}))?;
                }
                driver.finish("failed in photos")?;
                bail!("stage photos failed: {reason}");
            }
            println!("[photos] {seconds:.1}s");
            Some(output.join("frontend/inputs"))
        }
        None => None,
    };
    let given = from_photos.as_ref().or(options.inputs.as_ref());
    let inputs = match given {
        Some(inputs) => {
            let inputs = crate::storage::absolute(inputs)?;
            if !crate::storage::is_file(inputs.join("cameras.json")) {
                bail!("{} has no cameras.json; it is not an inputs directory", inputs.display());
            }
            inputs
        }
        None => {
            let (scene, prepared, raw_masks) = (
                options.scene.clone().expect("checked by run"),
                options.prepared.clone().expect("checked by run"),
                options.raw_masks.clone().expect("checked by run"),
            );
            let inputs = output.join("inputs");
            let command =
                ["crispmesh-dense", "inputs", "--scene", &text(&scene), "--prepared", &text(&prepared), "--raw-masks", &text(&raw_masks)]
                    .map(String::from)
                    .to_vec();
            let stage = driver.begin("inputs", command, Some(600.0))?;
            let result = stage.control.check().and_then(|_| scene::run(&scene, &prepared, &raw_masks, &inputs));
            if let Ok(report) = &result {
                stage.control.log(report.to_string());
            }
            driver.end(stage, result)?;
            inputs
        }
    };
    let inspector = (config.inspection_sheets && crate::inspect::memory_allows())
        .then(|| crate::inputs::load_views(&inputs).and_then(|rows| crate::inspect::Inspector::new(&rows, &output.join("inspect"))))
        .and_then(|made| made.ok())
        .map(Arc::new);
    if let Some(inspector) = &inspector {
        let _ = driver.previews.inspector.set(inspector.clone());
    }
    if input_sheet(&inputs, &output.join("input-sheet.png")).is_ok() {
        driver.events.stage("inputs").artifact("input_sheet", &output.join("input-sheet.png"), "Photos with mask outlines", json!({}))?;
    }
    driver.report["base_surface"] = serde_json::to_value(options.base_surface)?;
    driver.report["native_stages"] = json!(if from_photos.is_some() {
        vec!["photos", if options.silhouette_only() { "hull" } else { "stereo" }, "mesh", "check"]
    } else if options.inputs.is_some() {
        vec![if options.silhouette_only() { "hull" } else { "stereo" }, "mesh", "check"]
    } else {
        vec!["inputs", if options.silhouette_only() { "hull" } else { "stereo" }, "mesh", "check"]
    });

    let gpu = gpu?;
    let arguments = stereo::Arguments {
        inputs: inputs.clone(),
        output: output.join("stereo"),
        only: options.silhouette_only().then(|| "hull".into()),
        reuse_depths: options.reuse_depths.clone(),
        previews: options.live_previews,
        ..Default::default()
    };
    let mut command = vec!["crispmesh-dense".to_string(), "stereo".to_string(), "--inputs".to_string(), text(&inputs)];
    if let Some(depths) = &options.reuse_depths {
        command.extend(["--reuse-depths".to_string(), text(depths)]);
    }
    let base_stage = if options.silhouette_only() { "hull" } else { "stereo" };
    if options.silhouette_only() {
        command.extend(["--only".into(), "hull".into()]);
    }
    let stage = driver.begin(base_stage, command, Some(options.stereo_timeout))?;
    // The fused volume goes to the surface stage in memory; `volume.npz` is written only when it is
    // kept, and `depths.npz` (for --reuse-depths) not in a browser, where no later run could read it.
    let files =
        stereo::run::Files { volume: options.keep_volume, depths: cfg!(not(target_arch = "wasm32")), preview_volumes_in_memory: true };
    let mut fused = None;
    let result = match stage.control.check() {
        Ok(()) => stereo::run::run_fused(&arguments, config, &gpu, &stage.events, &stage.control, files).await.map(|(_, volume)| {
            fused = volume;
        }),
        Err(stopped) => Err(stopped),
    };
    driver.end(stage, result)?;
    drop(gpu);
    // Without a second thread the previews are meshed here.
    #[cfg(target_arch = "wasm32")]
    driver.previews.settle(false);

    let volume = output.join("stereo/volume.npz");
    let command = ["crispmesh-dense", "mesh", "--volume", &text(&volume)].map(String::from).to_vec();
    let threads = options.threads.max(1);
    let stage = driver.begin("mesh", command, Some(900.0))?;
    // The silhouette came first when the stereo stage says so (it knows the cameras) or the setting asks for it.
    let silhouette_first = fused.as_ref().map_or(config.silhouette_first == 1, |volume| volume.silhouette_first);
    let result = match fused {
        Some(fused) => crate::mesh::run_volume(fused, &output.join("mesh"), config, 1, &stage.events, None, threads, &stage.control),
        None => crate::mesh::run_with(
            &volume,
            &output.join("mesh"),
            config,
            1,
            &stage.events,
            options.silhouette_only().then_some(if options.splats.is_some() || options.surface.is_some() {
                "Silhouette starting surface"
            } else {
                "Silhouette surface"
            }),
            threads,
            &stage.control,
        ),
    };
    // Refinement against the photos, part of the surface stage: inward only from a silhouette hull.
    let result = result.and_then(|report| {
        if config.photo_refine == 1 || (config.photo_refine == 2 && silhouette_first) {
            let refine =
                crate::refine::Options { peak: config.photo_refine_peak as f32, inward_only: silhouette_first, ..Default::default() };
            let refined = crate::refine::run_in_place(&inputs, &output.join("mesh"), &refine, &|| stage.control.check())?;
            stage.control.log(format!("photo refinement {refined}"));
            let path = output.join("mesh/mesh.stl");
            stage.events.artifact(
                "final_mesh",
                &crate::storage::canonicalize(&path).unwrap_or(path),
                "Final surface, refined against the photos",
                json!({"triangles": report.triangles}),
            )?;
        }
        Ok(report)
    });
    if let Ok(report) = &result {
        stage.control.log(serde_json::to_string_pretty(report)?);
    }
    let mesh = driver.end(stage, result)?;
    if let Some(inspector) = &inspector {
        let events = driver.events.stage("mesh");
        // A sheet is a picture for people; one that cannot be drawn does not fail the run.
        let _ = inspector.add(
            "90-final",
            if options.silhouette_only() { "Silhouette surface" } else { "Final surface" },
            &output.join("mesh/mesh.stl"),
            &events,
        );
        let _ = inspector.overview(&events);
    }

    for (name, method) in [("surface", &options.surface), ("splats", &options.splats)] {
        let Some(method) = method else { continue };
        let command = ["crispmesh-dense", name, &text(&inputs), &text(&output.join(name))]
            .map(String::from)
            .into_iter()
            .chain(method.arguments())
            .collect();
        let stage = driver.begin(name, command, None)?;
        let mut result = optimised_surface(name, &inputs, output, method, &stage.control).await;
        if result.is_ok() {
            let (kind, label) =
                if name == "surface" { ("neural_surface", "Neural surface (experimental)") } else { ("splats", "Gaussian surface") };
            stage.events.artifact(
                kind,
                &output.join(name).join("mesh.stl"),
                label,
                json!({"triangles": result.as_ref().ok().map(|r| &r["triangles"])}),
            )?;
            if let Some(inspector) = &inspector {
                let _ = inspector.add(&format!("95-{name}"), label, &output.join(name).join("mesh.stl"), &stage.events);
                let _ = inspector.overview(&stage.events);
            }
        }
        // With --texture the method's mesh gets the run's photo texture too, beside it in NAME/texture.
        if let (true, Ok(report)) = (options.texture && name != "splats" && !(name == "surface" && options.splats.is_none()), &mut result) {
            let texture_output = output.join(name).join("texture");
            match crate::texture::run_with(&inputs, &output.join(name).join("mesh.stl"), &texture_output, 12, 1024, &stage.control) {
                Ok(_) => {
                    stage.events.artifact("textured_mesh", &texture_output.join("mesh.glb"), "Textured mesh (GLB)", json!({}))?;
                    report["textured_mesh"] = json!(text(&texture_output.join("mesh.glb")));
                }
                Err(error) => result = Err(error),
            }
        }
        driver.report[name] = driver.end(stage, result)?;
    }
    let gaussian_primary = options.splats.is_some();
    let primary_mesh = output.join(if gaussian_primary {
        "splats/mesh.stl"
    } else if options.surface.is_some() {
        "surface/mesh.stl"
    } else {
        "mesh/mesh.stl"
    });
    if options.check {
        let stl = primary_mesh.clone();
        let repaired = output.join("stereo/masks-repaired");
        let command = ["crispmesh-dense", "check", "--inputs", &text(&inputs), "--mesh", &text(&stl)].map(String::from).to_vec();
        let stage = driver.begin("check", command, Some(900.0))?;
        let check_options = check::Options {
            inputs: &inputs,
            mesh: &stl,
            output: &output.join("check"),
            repaired_masks: Some(&repaired),
            preview_views: if options.preview { 3 } else { 0 },
            check_views: 24,
        };
        let result = check::run_with(&check_options, &stage.events, &stage.control);
        if let Ok(report) = &result {
            stage.control.log(serde_json::to_string_pretty(report)?);
        }
        driver.report["photo_check"] = driver.end(stage, result)?;
    }
    if options.texture {
        let texture_output = output.join("texture");
        let stl = primary_mesh.clone();
        let command = ["crispmesh-dense", "texture", "--inputs", &text(&inputs), "--mesh", &text(&stl)].map(String::from).to_vec();
        let stage = driver.begin("texture", command, Some(900.0))?;
        let result = crate::texture::run_with(&inputs, &stl, &texture_output, 12, 1024, &stage.control);
        if result.is_ok() {
            stage.events.artifact("textured_mesh", &texture_output.join("mesh.glb"), "Textured mesh (GLB)", json!({}))?;
            stage.events.artifact("report", &texture_output.join("result.json"), "Texture coverage", json!({}))?;
        }
        driver.report["texture"] = driver.end(stage, result)?;
    }
    if !options.keep_volume && crate::storage::exists(&volume) {
        crate::storage::remove_file(&volume)?;
    }
    driver.report["mesh"] = json!(text(&primary_mesh));
    driver.report["closed"] = if options.splats.is_some() || options.surface.is_some() { Value::Null } else { json!(mesh.closed) };
    driver.report["triangles"] = if gaussian_primary {
        driver.report["splats"]["triangles"].clone()
    } else if options.surface.is_some() {
        driver.report["surface"]["triangles"].clone()
    } else {
        json!(mesh.triangles)
    };
    driver.report["genus"] = if options.splats.is_some() || options.surface.is_some() { Value::Null } else { json!(mesh.genus) };
    driver.report["reference_used"] = json!(false);
    driver.report["physical_scale_established"] = json!(false);
    driver.events.stage("mesh").artifact(
        "report",
        &output.join("mesh/result.json"),
        if options.splats.is_some() || options.surface.is_some() { "Starting surface report" } else { "Mesh report" },
        json!({}),
    )?;
    driver.finish("complete")
}

/// The optional photo-optimised surfaces of a run (`RunOptions::surface`, `RunOptions::splats`), for a front end:
/// `id`, `label`, `meaning`, whether this build runs it (`available`), and its quality levels and single settings.
pub fn methods() -> Value {
    let qualities = |fast: &str, standard: &str, best: &str| {
        json!([
            {"id": "fast", "label": "Fast", "meaning": fast},
            {"id": "standard", "label": "Standard", "meaning": standard},
            {"id": "best", "label": "Best", "meaning": best},
        ])
    };
    let common = json!([
        {"name": "crop", "kind": "integer", "meaning": "Target width in pixels of the photo window around the object (480 fast, 720 standard, larger for best); the photos are reduced by the whole factor that reaches it."},
        {"name": "mesh", "kind": "integer", "meaning": "Grid of the final mesh (cells per side)."},
        {"name": "steps", "kind": "integer", "meaning": "Optimisation steps (time grows with them)."},
        {"name": "seed", "kind": "integer", "meaning": "Random seed."},
    ]);
    let mut gaussian_qualities = qualities(
        "10 000 steps, about 480 px, 384 grid",
        "30 000 steps, about 720 px, 512 grid (the reference run's)",
        "30 000 steps, about 1080 px, 768 grid",
    );
    gaussian_qualities.as_array_mut().unwrap().insert(
        0,
        json!({
            "id": "capture", "label": "Capture — 3,000 steps",
            "meaning": "3,000 steps, about 320 px, 192 grid. A bounded starting point for real captures; more steps remain optional."
        }),
    );
    json!([
        {
            "id": "surface",
            "label": "Neural surface",
            "meaning": "A signed-distance network trained by rendering the photos (NeuS-facto), started from the run's mesh: clean surfaces with fine relief. GPU, minutes to hours.",
            "available": true,
            "qualities": qualities("6 000 steps, photos at about 480 px across the object, 384 grid", "20 000 steps, about 720 px, 512 grid (the reference run's)", "30 000 steps, twice the rays, about 960 px, 768 grid"),
            "settings": common,
            "output": "surface/mesh.stl",
        },
        {
            "id": "splats",
            "label": "Gaussian surface",
            "meaning": "Flat Gaussian disks trained on the photos with surface terms (2DGS), fused into a mesh; also a renderable model. GPU, minutes to hours.",
            "available": true,
            "qualities": gaussian_qualities,
            "settings": common,
            "output": "splats/mesh.stl",
        },
    ])
}

/// The neural surface or the splats of a finished run: its photos and masks (grown, else repaired, else the inputs'),
/// started from its mesh, trained on the GPU with the method's quality level and settings.
async fn optimised_surface(name: &str, inputs: &Path, output: &Path, method: &Method, control: &Control) -> anyhow::Result<Value> {
    use crate::surface::options::{split, Source};
    let (quality, items) = split(&method.arguments())?;
    let mut source = Source::for_quality(quality, if name == "surface" { 960 } else { 1080 });
    source.hull_mesh = Some(output.join("mesh/mesh.stl"));
    source.masks = ["stereo/masks-grown", "stereo/masks-repaired"].iter().map(|m| output.join(m)).find(|m| crate::storage::is_dir(m));
    let mut surface = crate::surface::train::Settings::for_quality(quality);
    let mut splats = crate::splats::train::Settings::for_quality(quality);
    for (key, value) in &items {
        let known = if name == "surface" { surface.set(key, value)? } else { splats.set(key, value)? };
        if !known && !source.set(key, value)? {
            anyhow::bail!("unknown {name} setting {key}");
        }
    }
    // Use the same initialization seed as the standalone method, including overrides.
    let scene = source.scene(inputs, if name == "surface" { surface.seed } else { splats.seed })?;
    control.log(format!("{} quality, {} views {}x{}", quality.name(), scene.views.len(), scene.views[0].width, scene.views[0].height));
    let gpu = Gpu::request().await?;
    let out = output.join(name);
    let say = |line: &str| control.log(line);
    let check = || control.check();
    if name == "surface" {
        crate::surface::train::train_with(&gpu, &scene, &out, &surface, &say, &check).await?;
    } else {
        crate::splats::train::train_with(&gpu, &scene, &out, &splats, &say, &check).await?;
    }
    let summary: Value = serde_json::from_slice(&crate::storage::read(out.join("summary.json"))?)?;
    Ok(
        json!({"mesh": text(&out.join("mesh.stl")), "quality": quality.name(), "triangles": summary["triangles"], "seconds": summary["seconds"]}),
    )
}

/// How a run may start on this platform, for a front end that builds its form
/// from data: `{"schema", "platform", "start_points": [...]}`. Each start point
/// has `id`, `label`, `meaning`, `fields` (`key` is the field of [`RunOptions`];
/// `kind` is `inputs`, `folder` or `file`) and `providers`: per module of the
/// photos stage the implementations this build can run, with `id`, `label`,
/// `meaning`, `platforms`, `external` programs, `license`, and the `option`
/// that selects it in `photo_options`.
pub fn describe() -> Value {
    describe_with(&[])
}

/// [`describe`] with the locations of the external programs: `photo_options`
/// are words of the photos stage's command line, of which the tool options are
/// read (`--colmap`, `--alicevision`, `--python`, `--sam-python`,
/// `--sam-source`, `--sam-checkpoint`, `--sam-repository`; then their
/// environment variables). Every provider of the photos start point says
/// whether it can run with them (`available`, `reason`, `version`) and lists
/// its options (`settings`: flag, kind, default, choices, meaning, from the
/// table the command-line parser reads); options shared by a module are in the
/// module's `settings`, the rest in the start point's `option_groups`. Every
/// start point lists the `stages` a run from it goes through.
pub fn describe_with(photo_options: &[String]) -> Value {
    let _ = photo_options;
    let field =
        |key: &str, label: &str, kind: &str, help: &str| json!({"key": key, "label": label, "kind": kind, "required": true, "help": help});
    let mut points = vec![
        json!({
            "id": "inputs", "label": "Inputs folder",
            "meaning": "A folder with cameras.json, the undistorted photos and their masks.",
            "fields": [field("inputs", "Inputs folder", "inputs", "A folder with cameras.json, the photos and their masks.")],
            "providers": [],
            "stages": ["stereo", "mesh", "check"],
        }),
        json!({
            "id": "scene", "label": "Camera solution, images and masks",
            "meaning": "An AliceVision solution with its prepared images and one raw mask per photo.",
            "fields": [
                field("scene", "Camera solution (.sfm)", "file", "AliceVision SfM file with poses and one radialk3 lens."),
                field("prepared", "Prepared images", "folder", "Undistorted images named <viewId>.png."),
                field("raw_masks", "Raw masks", "folder", "One 0/255 mask per source photo, named like the photo."),
            ],
            "providers": [],
            "stages": ["inputs", "stereo", "mesh", "check"],
        }),
    ];
    let table = crate::photos::providers::listing();
    // Only providers that can run on this platform are offered (in a browser: none with an external program).
    {
        use crate::photos::{availability, option_table};
        let tools = availability::ToolLocations::from_words(photo_options, &|name| std::env::var(name).ok());
        let choice = |module: &str, label: &str| {
            let options: Vec<Value> = table[module]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| row["available"] == true)
                .map(|row| {
                    let name = row["name"].as_str().unwrap_or_default();
                    let state = availability::check(module, name, &tools);
                    json!({
                        "id": row["name"], "label": row["name"], "meaning": row["summary"], "option": row["selector"],
                        "platforms": row["platforms"], "external": row["external"], "license": row["license"],
                        "available": state.available, "reason": state.reason, "version": state.version,
                        "settings": option_table::of_provider(module, name),
                    })
                })
                .collect();
            let default = table[module].as_array().into_iter().flatten().find(|row| row["default"] == true && row["available"] == true);
            json!({
                "module": module, "label": label, "options": options, "default": default.map(|row| row["name"].clone()),
                "settings": option_table::of_group(module),
            })
        };
        let group = |id: &str, label: &str| json!({"id": id, "label": label, "settings": option_table::of_group(id)});
        // `auto` instead of a file: the turntable solver finds the focal length (`--calibration auto`).
        let mut lens_field = field(
            "calibration",
            "Lens calibration (.json)",
            "file",
            "Focal length, principal point and radial distortion of the lens, or automatic: the focal length found from the turn, or fov:DEG: the camera's horizontal field of view.",
        );
        lens_field["automatic"] = json!(true);
        points.push(json!({
            "id": "photos", "label": "Turntable photos",
            "meaning": "A folder of photos of an object on a turntable and the calibration of the lens; masks and cameras are made first.",
            "fields": [
                field("photos", "Photos", "folder", "One photo per turntable position, named in capture order."),
                lens_field,
            ],
            "providers": [choice("masks", "Masks"), choice("cameras", "Cameras")],
            // The scene is written by the `cameras` stage: a run from photos has no `inputs` stage.
            "stages": ["masks", "cameras", "stereo", "mesh", "check"],
            "option_groups": [
                group("tools", "Tools"), group("machine", "Machine"), group("deadlines", "Deadlines"), group("gates", "Quality gates"),
            ],
        }));
    }
    let presets: Vec<Value> =
        PRESETS.iter().map(|(name, settings, meaning)| json!({"name": name, "settings": settings, "meaning": meaning})).collect();
    json!({"schema": "crispmesh_start_points_v1", "platform": table["platform"], "start_points": points, "presets": presets, "base_surfaces": ["silhouette", "stereo"], "methods": methods()})
}

pub const USAGE: &str = "usage: crispmesh-dense run --output DIR (--photos DIR --calibration JSON [--masks PROVIDER] [--cameras PROVIDER] \
[options of `crispmesh-dense photos`] | --inputs DIR | --scene FILE --prepared DIR --raw-masks DIR) \
[--config FILE] [--preset detail] [--set KEY=VALUE]... [--threads N] [--stereo-timeout SECONDS] [--minimum-free-gib G] [--reuse-depths FILE] \
[--base-surface silhouette|stereo] [--texture] [--surface QUALITY] [--surface-set NAME=VALUE]... [--splats QUALITY] [--splats-set NAME=VALUE]... [--no-live-previews] [--preview-step N] [--skip-check] [--no-preview] [--keep-volume]";

/// Options from the command line of `dense_pipeline.py` (the parts this driver covers).
pub fn parse(arguments: &[String]) -> anyhow::Result<RunOptions> {
    let mut options = RunOptions::default();
    let mut rest = arguments.iter();
    while let Some(flag) = rest.next() {
        let (flag, inline) = match flag.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (flag.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| rest.next().cloned()).ok_or_else(|| anyhow!("{flag} needs a value\n{USAGE}"));
        let number = |text: String| text.parse::<f64>().map_err(|_| anyhow!("{flag} needs a number, found {text:?}"));
        match flag {
            "--output" => options.output = PathBuf::from(value()?),
            "--inputs" => options.inputs = Some(PathBuf::from(value()?)),
            "--scene" => options.scene = Some(PathBuf::from(value()?)),
            "--prepared" => options.prepared = Some(PathBuf::from(value()?)),
            "--raw-masks" => options.raw_masks = Some(PathBuf::from(value()?)),
            "--config" => options.config = Some(PathBuf::from(value()?)),
            "--set" => options.overrides.push(value()?),
            "--preset" => options.preset = Some(value()?),
            "--threads" => options.threads = number(value()?)? as usize,
            "--stereo-timeout" => options.stereo_timeout = number(value()?)?,
            "--minimum-free-gib" => options.minimum_free_gib = number(value()?)?,
            "--reuse-depths" => options.reuse_depths = Some(PathBuf::from(value()?)),
            "--preview-step" => options.preview_step = number(value()?)? as usize,
            "--no-live-previews" => options.live_previews = false,
            "--skip-check" => options.check = false,
            "--no-preview" => options.preview = false,
            "--keep-volume" => options.keep_volume = true,
            "--texture" => options.texture = true,
            "--surface" => options.surface = Some(Method::parse(&value()?)?),
            "--splats" => options.splats = Some(Method::parse(&value()?)?),
            "--base-surface" => options.base_surface = match value()?.as_str() {
                "silhouette" => BaseSurface::Silhouette,
                "stereo" => BaseSurface::Stereo,
                _ => bail!("--base-surface expects silhouette or stereo"),
            },
            "--surface-set" => options.surface.get_or_insert_with(|| Method { quality: "standard".into(), settings: Vec::new() }).settings.push(value()?),
            "--splats-set" => options.splats.get_or_insert_with(|| Method { quality: "standard".into(), settings: Vec::new() }).settings.push(value()?),
            // Accepted for command lines written for the Python driver; they select nothing here.
            "--device" | "--torch-python" | "--native" | "--photos-timeout" => drop(value()?),
            "--photos" => options.photos = Some(PathBuf::from(value()?)),
            "--photos-option" => options.photo_options.push(value()?),
            "--reference" => bail!("--reference: scoring against a scan is done by scripts/turntable_mesh/scan_evaluate.py; run it on mesh/mesh.stl afterwards"),
            // Everything else belongs to the photos stage (`crispmesh-dense photos --help`); it checks the names.
            other => {
                options.photo_options.push(match &inline {
                    Some(value) => format!("{other}={value}"),
                    None => other.to_string(),
                });
                if inline.is_none() && rest.clone().next().is_some_and(|next| !next.starts_with("--")) {
                    options.photo_options.push(rest.next().cloned().expect("peeked"));
                }
            }
        }
    }
    if options.photos.is_none() && !options.photo_options.is_empty() {
        bail!("unknown argument {} (options of the photos stage need --photos)\n{USAGE}", options.photo_options[0]);
    }
    if options.output.as_os_str().is_empty() {
        bail!("--output is required\n{USAGE}");
    }
    Ok(options)
}

/// `crispmesh-dense run ...`: prints the summary the Python driver prints.
#[cfg(not(target_arch = "wasm32"))]
pub fn main(arguments: &[String]) -> anyhow::Result<()> {
    if arguments.iter().any(|a| a == "--describe") {
        let rest: Vec<String> = arguments.iter().filter(|a| *a != "--describe").cloned().collect();
        println!("{}", serde_json::to_string_pretty(&describe_with(&rest))?);
        return Ok(());
    }
    if arguments.iter().any(|a| a == "--list-settings") {
        let defaults = serde_json::to_value(DenseConfig::default())?;
        for (key, value) in defaults.as_object().into_iter().flatten() {
            println!("{key} = {value}");
        }
        return Ok(());
    }
    let report = run(&parse(arguments)?, None, None)?;
    let summary: serde_json::Map<String, Value> =
        ["status", "mesh", "closed", "triangles", "genus", "seconds"].iter().map(|k| (k.to_string(), report[*k].clone())).collect();
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Photos to STL entirely in the in-memory tree: photos and lens copied into `mem:`, the run directory in
    /// `mem:`, the mesh and the reports written out to `CRISPMESH_MEMORY_RUN_OUT`. Runs only when
    /// `CRISPMESH_MEMORY_RUN_PHOTOS` (a photo folder) and `CRISPMESH_MEMORY_RUN_CALIBRATION` are set.
    #[test]
    fn a_photos_run_in_memory() {
        let (Ok(photos), Ok(calibration), Ok(out)) = (
            std::env::var("CRISPMESH_MEMORY_RUN_PHOTOS"),
            std::env::var("CRISPMESH_MEMORY_RUN_CALIBRATION"),
            std::env::var("CRISPMESH_MEMORY_RUN_OUT"),
        ) else {
            return;
        };
        for path in crate::storage::list(&photos).unwrap() {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            crate::storage::write(format!("mem:/given/photos/{name}"), std::fs::read(&path).unwrap()).unwrap();
        }
        crate::storage::write("mem:/given/lens.json", std::fs::read(&calibration).unwrap()).unwrap();
        // An external provider is refused before anything runs.
        let refused = RunOptions {
            output: "mem:/refused".into(),
            photos: Some("mem:/given/photos".into()),
            photo_options: ["--calibration", "mem:/given/lens.json", "--cameras", "colmap"].map(String::from).to_vec(),
            ..Default::default()
        };
        assert!(run(&refused, None, None).unwrap_err().to_string().contains("starts external programs"));
        let options = RunOptions {
            output: "mem:/run".into(),
            photos: Some("mem:/given/photos".into()),
            photo_options: ["--calibration", "mem:/given/lens.json"].map(String::from).to_vec(),
            threads: 4,
            live_previews: false,
            ..Default::default()
        };
        let report = run(&options, None, None).unwrap();
        assert_eq!(report["status"], "complete");
        std::fs::create_dir_all(&out).unwrap();
        for (from, to) in [
            ("mesh/mesh.stl", "mesh.stl"),
            ("pipeline.json", "pipeline.json"),
            ("check/preview.png", "preview.png"),
            ("frontend/frontend.json", "frontend.json"),
            ("frontend/inputs/cameras.json", "cameras.json"),
        ] {
            std::fs::write(Path::new(&out).join(to), crate::storage::read(format!("mem:/run/{from}")).unwrap()).unwrap();
        }
        let files = crate::storage::memory_files("mem:/run");
        std::fs::write(Path::new(&out).join("memory-files.json"), serde_json::to_string(&files).unwrap()).unwrap();
        assert!(!Path::new("mem:").exists(), "nothing was written to a directory named mem:");
    }

    #[test]
    fn silhouette_default_and_gaussian_capture_are_independent_of_stereo() {
        let default = RunOptions::default();
        assert_eq!(default.base_surface, BaseSurface::Silhouette);
        assert!(default.splats.is_none() && default.surface.is_none());
        let config = default.configuration().unwrap();
        assert_eq!(config.grid, 192);
        assert!(!config.repair_masks && !config.mask_grow && !config.mesh_open_unseen);
        assert_eq!((config.silhouette_first, config.photo_refine), (0, 0));
        let old = RunOptions { base_surface: BaseSurface::Stereo, ..Default::default() };
        assert_eq!(serde_json::to_value(old.configuration().unwrap()).unwrap(), serde_json::to_value(DenseConfig::default()).unwrap());
        let capture: RunOptions = serde_json::from_value(json!({"output":"out", "inputs":"in", "splats":{}})).unwrap();
        assert_eq!(capture.splats.as_ref().unwrap().quality, "capture");
        let quality = capture.splats.unwrap().quality.parse().unwrap();
        let splats = crate::splats::train::Settings::for_quality(quality);
        let source = crate::surface::options::Source::for_quality(quality, 1080);
        assert_eq!((splats.steps, source.crop, splats.mesh), (3000, 320, 192));
        let words = ["--inputs", "in", "--output", "out", "--base-surface", "stereo"].map(String::from);
        assert_eq!(parse(&words).unwrap().base_surface, BaseSurface::Stereo);
    }

    #[test]
    fn optional_surfaces_from_the_command_line_and_from_json() {
        let words = |text: &str| text.split_whitespace().map(String::from).collect::<Vec<_>>();
        let options = parse(&words("--inputs in --output out --surface fast --surface-set crop=600 --splats 5000")).unwrap();
        assert_eq!(options.surface, Some(Method { quality: "fast".into(), settings: vec!["crop=600".into()] }));
        assert_eq!(options.splats, Some(Method { quality: "standard".into(), settings: vec!["steps=5000".into()] }));
        assert!(parse(&words("--inputs in --output out --surface quick")).is_err());
        let options: RunOptions =
            serde_json::from_value(json!({"output": "out", "inputs": "in", "splats": {"quality": "best", "settings": ["mesh=640"]}}))
                .unwrap();
        assert_eq!(options.splats.unwrap().settings, vec!["mesh=640".to_string()]);
        let methods = methods();
        assert_eq!(methods.as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect::<Vec<_>>(), ["surface", "splats"]);
        assert_eq!(describe()["methods"], methods);
    }

    #[test]
    fn options_from_the_command_line_and_from_json() {
        let words = "--output run --inputs in --set grid=96 --no-live-previews --threads 3 --device mps --keep-volume --set=sizes=64,128";
        let options = parse(&words.split(' ').map(String::from).collect::<Vec<_>>()).unwrap();
        assert_eq!((options.output.clone(), options.inputs.clone()), (PathBuf::from("run"), Some(PathBuf::from("in"))));
        assert!(!options.live_previews && options.keep_volume && options.check && options.threads == 3);
        let config = options.configuration().unwrap();
        assert_eq!((config.grid, config.sizes), (96, vec![64, 128]));
        let from_photos = parse(
            &"--output run --photos shots --calibration lens.json --masks threshold --colmap-masks=off --no-sam-multimask --threads 3"
                .split(' ')
                .map(String::from)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(from_photos.photos, Some(PathBuf::from("shots")));
        assert_eq!(
            from_photos.photo_options,
            ["--calibration", "lens.json", "--masks", "threshold", "--colmap-masks=off", "--no-sam-multimask"]
        );
        assert_eq!(from_photos.threads, 3);
        for refused in ["--reference scan.ply", "--output run --inputs in --bogus", "--inputs in"] {
            assert!(parse(&refused.split(' ').map(String::from).collect::<Vec<_>>()).is_err(), "{refused}");
        }
        let json =
            r#"{"output": "run", "inputs": "in", "settings": {"grid": 320, "sizes": [256, 512], "repair_masks": false}, "check": false}"#;
        let options: RunOptions = serde_json::from_str(json).unwrap();
        let config = options.configuration().unwrap();
        assert_eq!((config.grid, config.sizes, config.repair_masks, options.check), (320, vec![256, 512], false, false));
        assert!(serde_json::from_str::<RunOptions>(r#"{"output": "run", "typo": 1}"#).is_err());
        let invalid: RunOptions = serde_json::from_str(r#"{"output": "run", "inputs": "in", "settings": {"windows": [4]}}"#).unwrap();
        assert!(invalid.configuration().is_err());
    }

    #[test]
    fn start_points_are_described_as_the_form_reads_them() {
        let described = describe();
        let points = described["start_points"].as_array().unwrap();
        let ids: Vec<&str> = points.iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["inputs", "scene", "photos"]);
        for point in points {
            assert!(point["label"].is_string() && !point["fields"].as_array().unwrap().is_empty());
            for field in point["fields"].as_array().unwrap() {
                assert!(["inputs", "folder", "file"].contains(&field["kind"].as_str().unwrap()));
                // Every field is a field of RunOptions or an option of the photos stage.
                let key = field["key"].as_str().unwrap();
                assert!(
                    serde_json::to_value(key).is_ok()
                        && ["inputs", "scene", "prepared", "raw_masks", "photos", "calibration"].contains(&key)
                );
            }
        }
        assert_eq!(points[0]["stages"], json!(["stereo", "mesh", "check"]));
        assert_eq!(points[2]["stages"], json!(["masks", "cameras", "stereo", "mesh", "check"]));
        let groups: Vec<&str> = points[2]["option_groups"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap()).collect();
        assert_eq!(groups, ["tools", "machine", "deadlines", "gates"]);
        let cameras = &points[2]["providers"][1];
        assert!(cameras["settings"].as_array().unwrap().iter().any(|o| o["flag"] == "--open-turn"));
        let colmap = cameras["options"].as_array().unwrap().iter().find(|o| o["id"] == "colmap").unwrap();
        assert_eq!(colmap["settings"][0]["kind"], "executable");
        // With a location that does not exist the provider says why it cannot run; our own solver always can.
        let described_with = describe_with(&["--colmap".to_string(), "/no/such/colmap".to_string()]);
        let options = described_with["start_points"][2]["providers"][1]["options"].as_array().unwrap().clone();
        let state = |id: &str| options.iter().find(|o| o["id"] == id).unwrap().clone();
        assert_eq!(state("colmap")["available"], false);
        assert!(state("colmap")["reason"].as_str().unwrap().contains("not found"));
        assert_eq!(
            (state("turntable")["available"].as_bool(), state("turntable")["version"].as_str()),
            (Some(true), Some(env!("CARGO_PKG_VERSION")))
        );
        let providers = points[2]["providers"].as_array().unwrap();
        assert_eq!(providers.iter().map(|c| c["module"].as_str().unwrap()).collect::<Vec<_>>(), ["masks", "cameras"]);
        for choice in providers {
            let options = choice["options"].as_array().unwrap();
            assert!(options.iter().any(|o| o["id"] == choice["default"]));
            assert!(options.iter().all(|o| o["license"].is_string() && o["option"].as_str().unwrap().starts_with("--")));
        }
    }

    #[test]
    fn refuses_before_anything_is_written() {
        let root = std::env::temp_dir().join(format!("crispmesh-run-refuse-{}", std::process::id()));
        let _ = crate::storage::remove_dir_all(&root);
        let options = RunOptions { output: root.clone(), ..Default::default() };
        assert!(run(&options, None, None).is_err());
        assert!(!root.exists());
        let options = RunOptions { output: root.clone(), inputs: Some(root.join("in")), minimum_free_gib: 1e9, ..Default::default() };
        assert!(run(&options, None, None).unwrap_err().to_string().contains("GiB free"));
        assert!(!root.exists());
    }

    /// A directory without a camera table fails after `run_started`, and the log is still closed (no GPU needed).
    #[test]
    fn a_failure_between_stages_closes_the_event_log() {
        let root = std::env::temp_dir().join(format!("crispmesh-run-fail-{}", std::process::id()));
        let _ = crate::storage::remove_dir_all(&root);
        crate::storage::create_dir_all(root.join("in")).unwrap();
        let options = RunOptions { output: root.join("run"), inputs: Some(root.join("in")), ..Default::default() };
        assert!(run(&options, None, None).is_err());
        let log = crate::storage::read_to_string(root.join("run/events.jsonl")).unwrap();
        let types: Vec<String> =
            log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["type"].as_str().unwrap().to_string()).collect();
        assert_eq!(types, ["run_started", "error", "run_finished"]);
        let last: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(last["status"], "failed");
        let pipeline: Value = serde_json::from_str(&crate::storage::read_to_string(root.join("run/pipeline.json")).unwrap()).unwrap();
        assert_eq!(pipeline["status"], "failed before or between stages");
        assert!(root.join("run/config.json").is_file());
        crate::storage::remove_dir_all(&root).unwrap();
    }

    fn kinds(events: &[Value], kind: &str) -> usize {
        events.iter().filter(|e| e["type"] == "artifact" && e["kind"] == kind).count()
    }

    /// The default base and optional Gaussian must select the same mesh for checking and reporting.
    #[test]
    fn gpu_silhouette_and_gaussian_choose_the_requested_final_mesh() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        let root = crate::stereo::synthetic::temporary("run-silhouette", 4, 64).unwrap();
        for gaussian in [false, true] {
            let output = root.join(if gaussian { "gaussian" } else { "silhouette" });
            let options = RunOptions {
                output: output.clone(),
                inputs: Some(root.join("inputs")),
                overrides: vec!["grid=64".into()],
                splats: gaussian
                    .then(|| Method { quality: "capture".into(), settings: vec!["steps=2".into(), "crop=64".into(), "mesh=32".into()] }),
                ..Default::default()
            };
            let report = run(&options, None, None).unwrap();
            assert_eq!(report["status"], "complete");
            assert_eq!(report["base_surface"], "silhouette");
            assert!(output.join("mesh/mesh.stl").is_file());
            assert!(!output.join("stereo/depths.npz").exists());
            let final_mesh = output.join(if gaussian { "splats/mesh.stl" } else { "mesh/mesh.stl" });
            assert_eq!(Path::new(report["mesh"].as_str().unwrap()), crate::storage::canonicalize(final_mesh).unwrap());
            assert!(report["triangles"].as_u64().unwrap() > 0);
            assert!(output.join("check/result.json").is_file());
            if gaussian {
                assert_eq!(report["triangles"], report["splats"]["triangles"]);
                assert!(report["closed"].is_null());
            }
        }
        crate::storage::remove_dir_all(&root).unwrap();
    }

    /// The whole pipeline on the analytic sphere, then a cancelled run (CRISPMESH_GPU_TESTS=1).
    #[test]
    fn gpu_run_completes_and_can_be_cancelled() {
        if !crate::gpu::tests_enabled() {
            return;
        }
        use crate::stereo::synthetic;
        let root = synthetic::temporary("run", 24, 128).unwrap();
        let overrides: Vec<String> = synthetic::SMALL.iter().map(|s| s.to_string()).collect();
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = seen.clone();
        let observer: Observer = Arc::new(move |event: &Value| sink.lock().unwrap().push(event.clone()));
        let options = RunOptions {
            output: root.join("run"),
            inputs: Some(root.join("inputs")),
            overrides: overrides.clone(),
            base_surface: BaseSurface::Stereo,
            texture: true,
            attribution: Some("Synthetic research capture".into()),
            ..Default::default()
        };
        let report = run(&options, Some(observer), None).unwrap();
        assert_eq!(report["status"], "complete");
        assert_eq!(report["closed"], true);
        assert!(report["triangles"].as_u64().unwrap() > 10_000);
        assert!(report["photo_check"]["silhouette_iou_input_masks"]["median"].as_f64().unwrap() > 0.9);
        for stage in ["stereo", "mesh", "check", "texture"] {
            assert_eq!(report["stages"][stage]["exit_code"], 0, "{stage}");
            assert!(Path::new(report["stages"][stage]["log"].as_str().unwrap()).is_file());
        }
        let run_directory = crate::storage::canonicalize(root.join("run")).unwrap();
        for file in
            ["config.json", "pipeline.json", "events.jsonl", "input-sheet.png", "mesh/mesh.stl", "check/preview.png", "stereo/depths.npz"]
        {
            assert!(run_directory.join(file).is_file(), "{file}");
        }
        assert!(run_directory.join("texture/mesh.glb").is_file());
        assert_eq!(report["texture"]["attribution_embedded"], true);
        assert!(report["texture"]["untextured_area_fraction"].as_f64().unwrap() < 0.4);
        assert!(!run_directory.join("stereo/volume.npz").exists());
        assert!(std::fs::read_dir(run_directory.join("stereo/preview"))
            .unwrap()
            .flatten()
            .all(|e| e.path().extension().is_none_or(|x| x != "npz")));
        let events = seen.lock().unwrap().clone();
        assert_eq!(crate::storage::read_to_string(run_directory.join("events.jsonl")).unwrap().lines().count(), events.len());
        assert_eq!((events[0]["type"].as_str(), events[0]["schema"].as_str()), (Some("run_started"), Some(SCHEMA)));
        assert_eq!(
            (events.last().unwrap()["type"].as_str(), events.last().unwrap()["status"].as_str()),
            (Some("run_finished"), Some("complete"))
        );
        let stages: Vec<(String, String)> = events
            .iter()
            .filter(|e| e["type"] == "stage_started" || e["type"] == "stage_finished")
            .map(|e| (e["type"].as_str().unwrap().to_string(), e["stage"].as_str().unwrap().to_string()))
            .collect();
        let expected = [
            ("stage_started", "stereo"),
            ("stage_finished", "stereo"),
            ("stage_started", "mesh"),
            ("stage_finished", "mesh"),
            ("stage_started", "check"),
            ("stage_finished", "check"),
            ("stage_started", "texture"),
            ("stage_finished", "texture"),
        ];
        assert_eq!(stages, expected.map(|(a, b)| (a.to_string(), b.to_string())));
        for (kind, count) in [
            ("input_sheet", 1),
            ("mask_repair_sheet", 1),
            ("hull_mask_sheet", 1),
            ("depth_sheet", 3),
            ("preview_volume", 3),
            ("preview_mesh", 3),
            ("final_mesh", 1),
            ("photo_overlay", 1),
            ("preview_render", 1),
            ("report", 3),
            ("textured_mesh", 1),
        ] {
            assert_eq!(kinds(&events, kind), count, "{kind}");
        }
        for event in events.iter().filter(|e| e["type"] == "artifact" && e["kind"] != "preview_volume") {
            assert!(run_directory.join(event["path"].as_str().unwrap()).is_file(), "{event}");
        }

        // The same run entirely in the in-memory tree: same result, nothing on disk.
        let memory = PathBuf::from(format!("mem:/run-test-{}", std::process::id()));
        synthetic::write(&memory.join("inputs"), 24, 128).unwrap();
        let options = RunOptions {
            output: memory.join("run"),
            base_surface: BaseSurface::Stereo,
            texture: true,
            inputs: Some(memory.join("inputs")),
            overrides: overrides.clone(),
            ..Default::default()
        };
        let in_memory = run(&options, None, None).unwrap();
        assert_eq!(in_memory["triangles"], report["triangles"]);
        assert_eq!(in_memory["photo_check"]["silhouette_iou_input_masks"], report["photo_check"]["silhouette_iou_input_masks"]);
        for file in ["run/events.jsonl", "run/pipeline.json", "run/mesh/mesh.stl", "run/check/preview.png", "run/stereo.log"] {
            assert!(crate::storage::is_file(memory.join(file)), "{file}");
        }
        assert!(!Path::new("mem:").exists(), "a memory run must not touch the disk");
        assert_eq!(
            crate::storage::read_to_string(memory.join("run/events.jsonl")).unwrap().lines().count(),
            crate::storage::read_to_string(run_directory.join("events.jsonl")).unwrap().lines().count()
        );
        crate::storage::remove_dir_all(&memory).unwrap();
        assert!(crate::storage::memory_files(&memory).is_empty());

        // Cancelled from the observer as soon as matching reports progress.
        let cancel = Arc::new(AtomicBool::new(false));
        let (flag, seen) = (cancel.clone(), Arc::new(Mutex::new(Vec::<Value>::new())));
        let sink = seen.clone();
        let observer: Observer = Arc::new(move |event: &Value| {
            if event["type"] == "progress" && event["message"].as_str().is_some_and(|m| m.starts_with("Matching")) {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            sink.lock().unwrap().push(event.clone());
        });
        let options = RunOptions {
            base_surface: BaseSurface::Stereo,
            output: root.join("cancelled"),
            inputs: Some(root.join("inputs")),
            overrides,
            ..Default::default()
        };
        let error = run(&options, Some(observer), Some(cancel)).unwrap_err();
        assert_eq!(error.downcast_ref::<Stopped>(), Some(&Stopped::Cancelled));
        let events = seen.lock().unwrap().clone();
        assert_eq!(events.last().unwrap()["status"], "cancelled");
        assert!(events.iter().any(|e| e["type"] == "error" && e["message"] == "cancelled on request" && e["stage"] == "stereo"));
        let pipeline: Value = serde_json::from_str(&crate::storage::read_to_string(root.join("cancelled/pipeline.json")).unwrap()).unwrap();
        assert_eq!(pipeline["status"], "failed in stereo");
        assert_eq!(pipeline["stages"]["stereo"]["cancelled"], true);
        assert!(pipeline["stages"]["stereo"]["exit_code"].is_null());
        crate::storage::remove_dir_all(&root).unwrap();
    }
}
