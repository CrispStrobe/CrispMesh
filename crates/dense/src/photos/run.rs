//! Orchestration of the photo front stage (`run` of `photos_to_inputs.py`,
//! reorganised around providers): two stages in the event log, `masks` then
//! `cameras`.
//!
//! ```text
//! masks    stage photos, coarse masks -> mask provider -> hole cleanup -> masks/, sheet
//! cameras  contrast images -> camera provider -> audit and ring gates
//!          -> undistortion and scene (inputs/) -> sparse overlay
//! ```
//!
//! Image processing, gates, undistortion and bookkeeping run in this process;
//! providers may start external programs as bounded child processes. The
//! gates decide the exit code.

use crate::photos::fs::Stored as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::control::Stopped;
use crate::events::EventLog;

use super::audit::{audit_solution, Policy};
use super::calibration::{load_calibration, scale_calibration};
use super::cameras::camera_provider;
use super::masks::mask_provider;
use super::options::{Options, SCHEMA};
use super::process::{bounded, log_tail, ExternalCommand};
use super::ring::{decide_gates, ring_statistics, scene_cameras};
use super::scene_writer::write_scene;
use super::solution::{Lens, Solution};
use super::staging::{capture_name, list_photos, open_photo};
use super::{cleanup, contrast, sheets, staging, util};

/// The report (`frontend.json`) and the exit code: 0 complete, 2 cameras rejected by a gate, 1 a step failed.
pub struct Finished {
    pub report: Value,
    pub code: u8,
}

/// A run in progress: what providers get to start steps and leave notes.
pub struct Run<'a> {
    pub options: &'a Options,
    pub out: &'a Path,
    events: EventLog,
    cancel_file: PathBuf,
    cancel: Option<Arc<AtomicBool>>,
    report: Value,
    started: Instant,
}

/// A handful of progress events per step, not one per photo: the fraction is rounded to two digits and reported in steps of 0.05.
struct Ticker<'a> {
    events: &'a EventLog,
    low: f64,
    high: f64,
    last: f64,
    message: &'a str,
}

impl Ticker<'_> {
    fn tick(&mut self, completion: f64) {
        let fraction = ((self.low + (self.high - self.low) * completion.clamp(0.0, 1.0)) * 100.0).round() / 100.0;
        if fraction >= self.last + 0.05 {
            self.last = fraction;
            let _ = self.events.progress(fraction, self.message);
        }
    }
}

pub fn count_files(folder: &Path, extension: &str) -> usize {
    crate::photos::fs::list(folder)
        .map(|entries| entries.iter().filter(|p| p.extension().is_some_and(|x| x == extension)).count())
        .unwrap_or(0)
}

fn round_to(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

/// Audit, ring statistics and the gate decision; writes the three JSON files and returns the gates.
fn judge(options: &Options, out: &Path, solution: &Solution, declared: Option<&Lens>) -> anyhow::Result<Value> {
    let sfm = out.join("sfm");
    crate::photos::fs::create_dir_all(&sfm)?;
    let gates = &options.gates;
    let names: Vec<String> = (0..options.photo_count).map(capture_name).collect();
    let policy = Policy {
        expected_names: Some(&names),
        expected_calibration: declared.map(|lens| (lens.pixels, lens.k)),
        minimum_coverage: gates.minimum_registered_fraction,
        minimum_observations_per_view: gates.minimum_observations_per_view,
        maximum_reprojection_p95: gates.maximum_view_reprojection_p95_pixels,
    };
    let audit = audit_solution(solution, &policy)?;
    util::write_json(&sfm.join("camera-audit.json"), &audit, 1)?;
    let limits = gates.to_json();
    let cameras = scene_cameras(solution)?;
    let rings = if options.cameras == super::options::CameraChoice::Turntable { options.turntable.rings } else { 1 };
    let (ring, passed, reasons) = if rings > 1 {
        // Several turns: every turn is one ring of its own, judged as a capture of that many photos.
        let per = options.photo_count / rings;
        let (mut all, mut passed, mut reasons) = (Vec::new(), true, Vec::new());
        for r in 0..rings {
            let own: Vec<_> = cameras
                .iter()
                .filter(|c| c.index / per == r)
                .map(|c| super::ring::SceneCamera { index: c.index - r * per, ..c.clone() })
                .collect();
            let mut statistics = ring_statistics(&own, gates.duplicate_step_deg);
            // Turns at several heights include steep ones, whose ring radius is small: there the optical
            // axis gate compares the miss with the viewing distance (equal to the radius for level cameras).
            if let Some(scaled) = statistics.get("optical_axis_miss_of_distance_percent").cloned() {
                statistics["optical_axis_miss_percent"] = scaled;
            }
            let mut audit = audit.clone();
            audit["input_images"] = json!(per);
            audit["registered_cameras"] = json!(own.len());
            audit["coverage"] = json!(own.len() as f64 / per as f64);
            let (ok, why) = decide_gates(&audit, &statistics, &limits);
            passed &= ok;
            reasons.extend(why.into_iter().map(|reason| format!("turn {}: {reason}", r + 1)));
            all.push(statistics);
        }
        reasons.sort();
        reasons.dedup();
        let mut ring = all[0].clone();
        ring["rings"] = json!(all);
        (ring, passed, reasons)
    } else {
        let ring = ring_statistics(&cameras, gates.duplicate_step_deg);
        let (passed, reasons) = decide_gates(&audit, &ring, &limits);
        (ring, passed, reasons)
    };
    util::write_json(&sfm.join("ring-sanity.json"), &ring, 1)?;
    let worst = audit["per_view"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|view| view["reprojection_pixels"]["p95"].as_f64())
        .fold(None, |best: Option<f64>, p95| Some(best.map_or(p95, |b| b.max(p95))));
    let result = json!({
        "passed": passed, "reasons": reasons, "limits": limits, "registered_views": audit["registered_cameras"],
        "input_photos": audit["input_images"], "missing": audit["missing_names"], "landmarks": audit["landmarks"],
        "observations": audit["observations"], "reprojection_pixels": audit["reprojection_pixels"],
        "worst_view_reprojection_p95_pixels": worst, "positive_depth_fraction": audit["positive_depth_fraction"], "ring": ring,
    });
    util::write_json(&sfm.join("gates.json"), &result, 1)?;
    Ok(result)
}

impl Run<'_> {
    fn save(&mut self) -> anyhow::Result<()> {
        self.report["seconds"] = json!(self.started.elapsed().as_secs_f64());
        util::write_json(&self.out.join("frontend.json"), &self.report, 1)
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed)) || self.cancel_file.stored()
    }

    pub fn warn(&mut self, text: String) {
        if let Some(warnings) = self.report["warnings"].as_array_mut() {
            warnings.push(json!(text));
        }
    }

    /// Something a provider wants in the report, under `provider_notes`.
    pub fn note(&mut self, key: &str, value: Value) {
        self.report["provider_notes"][key] = value;
    }

    fn step_count(&self) -> usize {
        self.report["steps"].as_array().map(Vec::len).unwrap_or(0)
    }

    /// Free space floor, the opening progress event and the log file of a step.
    fn begin(&self, stage: &str, name: &str, low: f64, message: &str) -> anyhow::Result<PathBuf> {
        // There is no disk to ask in a browser or for the in-memory tree.
        #[cfg(not(target_arch = "wasm32"))]
        if !crate::storage::is_memory(self.out) {
            let free = fs4::available_space(self.out).with_context(|| self.out.display().to_string())? as f64 / (1u64 << 30) as f64;
            if free < self.options.minimum_free_gib {
                bail!("{name}: only {free:.1} GiB free; need {} (--minimum-free-gib)", util::python_float(self.options.minimum_free_gib));
            }
        }
        if self.cancelled() {
            bail!("{name} failed (cancelled)");
        }
        self.events.stage(stage).progress(low, message)?;
        println!("[{stage}] {message} ...");
        Ok(self.out.join("logs").join(format!("{:02}-{name}.log", self.step_count() + 1)))
    }

    fn record(&mut self, stage: &str, name: &str, mut entry: Value) -> anyhow::Result<()> {
        entry["name"] = json!(name);
        entry["stage"] = json!(stage);
        if let Some(steps) = self.report["steps"].as_array_mut() {
            steps.push(entry);
        }
        self.save()
    }

    /// One external program as a bounded process started in `directory`.
    /// `counter` estimates its completion between 0 and 1.
    #[allow(clippy::too_many_arguments)]
    pub fn external(
        &mut self,
        stage: &str,
        step: &ExternalCommand,
        directory: &Path,
        timeout: u64,
        low: f64,
        high: f64,
        message: &str,
        counter: Option<&dyn Fn() -> f64>,
    ) -> anyhow::Result<()> {
        let name = step.name.as_str();
        let log = self.begin(stage, name, low, message)?;
        let threads = self.options.threads.to_string();
        let mut environment: Vec<(String, String)> = ["OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "VECLIB_MAXIMUM_THREADS"]
            .iter()
            .map(|n| (n.to_string(), threads.clone()))
            .collect();
        environment.extend(step.environment.iter().cloned());
        let staged = self.events.stage(stage);
        let mut ticker = Ticker { events: &staged, low, high, last: low, message };
        let outcome = bounded(&step.command, &log, Duration::from_secs(timeout), &environment, directory, &mut || {
            if let Some(counter) = counter {
                ticker.tick(counter());
            }
            self.cancelled().then_some(true)
        })?;
        self.record(stage, name, outcome.to_json())?;
        if !outcome.succeeded() {
            let reason = if outcome.cancelled {
                "cancelled".to_string()
            } else if outcome.timed_out {
                format!("deadline of {timeout} s")
            } else {
                format!("exit code {}", outcome.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into()))
            };
            bail!("{name} failed ({reason}): {}", log_tail(&log, 600));
        }
        self.events.stage(stage).progress(high, &format!("{message}: done"))?;
        Ok(())
    }

    /// One step done in this process. `work` gets a callback to report finished
    /// items (of `total`), which also enforces the deadline and the cancel request.
    #[allow(clippy::too_many_arguments)]
    pub fn internal<T>(
        &mut self,
        stage: &str,
        name: &str,
        timeout: u64,
        low: f64,
        high: f64,
        message: &str,
        total: usize,
        work: impl FnOnce(&mut dyn FnMut(usize) -> anyhow::Result<()>) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let log = self.begin(stage, name, low, message)?;
        let started = Instant::now();
        let staged = self.events.stage(stage);
        let mut ticker = Ticker { events: &staged, low, high, last: low, message };
        let (cancel, cancel_file) = (self.cancel.clone(), self.cancel_file.clone());
        let result = work(&mut |done| {
            if cancel.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed)) || cancel_file.stored() {
                return Err(Stopped::Cancelled.into());
            }
            if started.elapsed() > Duration::from_secs(timeout) {
                return Err(Stopped::Deadline.into());
            }
            ticker.tick(done as f64 / total.max(1) as f64);
            Ok(())
        });
        let stopped = result.as_ref().err().and_then(|e| e.downcast_ref::<Stopped>().copied());
        let text = match &result {
            Ok(_) => "done\n".to_string(),
            Err(error) => format!("{error:#}\n"),
        };
        crate::photos::fs::write(&log, &text).with_context(|| log.display().to_string())?;
        let entry = json!({
            "command": ["crisp3ds-dense", "photos", "(in process)", name],
            "exit_code": if stopped.is_some() { Value::Null } else { json!(result.is_err() as u8) },
            "timed_out": stopped == Some(Stopped::Deadline), "cancelled": stopped == Some(Stopped::Cancelled),
            "seconds": started.elapsed().as_secs_f64(), "log": log.to_string_lossy(),
        });
        self.record(stage, name, entry)?;
        let value = result.map_err(|error| match stopped {
            Some(Stopped::Cancelled) => anyhow!("{name} failed (cancelled)"),
            Some(Stopped::Deadline) => anyhow!("{name} failed (deadline of {timeout} s)"),
            None => anyhow!("{name} failed: {error:#}"),
        })?;
        self.events.stage(stage).progress(high, &format!("{message}: done"))?;
        Ok(value)
    }

    /// Returns the photo map (`photo-map.json`).
    fn masks_stage(&mut self) -> anyhow::Result<Value> {
        let (options, out) = (self.options, self.out);
        let (count, small, threads) = (options.photo_count, options.timeouts.small, options.threads);
        let provider = mask_provider(options);
        let stage_started = Instant::now();
        self.events.stage("masks").emit("stage_started", json!({"provider": provider.info().name}))?;
        // The share of the stage each step gets depends on whether the provider starts a network in between.
        let slow = !provider.info().external.is_empty() || provider.info().name == "sam";
        let (coarse_end, cleanup_start, cleanup_end) = if slow { (0.08, 0.85, 0.92) } else { (0.45, 0.55, 0.85) };
        let photos = list_photos(&options.photos)?;
        let background_prompt = options.masks == super::options::MaskChoice::Background
            || (matches!(options.masks, super::options::MaskChoice::Sam | super::options::MaskChoice::ExternalSam)
                && options.sam.background_prompt);
        let coarse_message =
            if background_prompt { "Reading photos, coarse background masks" } else { "Reading photos, coarse dark-object masks" };
        let photo_map = self.internal("masks", "coarse", small, 0.0, coarse_end, coarse_message, count, |watch| {
            // With a marker mat the dark markers are hidden first (markers/provider.rs).
            let hide = super::markers::provider::mask_preparation(options)?;
            let shadow = options.threshold_shadow;
            if background_prompt {
                return staging::step_coarse_background(out, &photos, &options.envelope, threads, watch);
            }
            staging::step_coarse_shadow(out, &photos, &options.envelope, options.dark_threshold, shadow, threads, hide.as_deref(), watch)
        })?;
        let touching: Vec<&str> = photo_map["photos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row["touches_envelope"] == true)
            .filter_map(|row| row["capture"].as_str())
            .collect();
        if let Some(first) = touching.first() {
            self.warn(format!(
                "coarse object region touches the envelope in {} photos (first {first}): object clipped by the frame or by --threshold-envelope",
                touching.len()
            ));
        }
        let segmented = provider.segment(self, &photo_map, coarse_end, cleanup_start)?;
        let cleaned = out.join("work/mask-cleanup");
        self.internal("masks", "cleanup", small, cleanup_start, cleanup_end, "Filling small dark holes", count, |watch| {
            cleanup::run(&out.join("work/photos"), &segmented, &cleaned, options.hole_cleanup_budget, threads, watch)
        })?;
        let masks =
            self.internal("masks", "publish-masks", small, cleanup_end, 1.0, "Writing masks and contact sheet", count, |watch| {
                sheets::step_publish_masks(out, count, &cleaned.join("masks"), watch)
            })?;
        let mut summary = masks.clone();
        if let Some(object) = summary.as_object_mut() {
            object.remove("views");
        }
        summary["provider"] = json!(provider.info().name);
        self.report["masks"] = summary;
        let dropping = masks["views_dropping_over_3_percent_of_coarse"].as_array().map(Vec::len).unwrap_or(0);
        if let (true, Some(who)) = (dropping > 0, provider.dropped_warning()) {
            self.warn(format!("{who} more than 3% of the coarse dark region in {dropping} photos (thin parts?)"));
        }
        let staged = self.events.stage("masks");
        staged.emit("metric", json!({"name": "mask_area_median_pixels", "value": masks["median_area_pixels"]}))?;
        let fraction = round_to(masks["median_area_fraction"].as_f64().unwrap_or(0.0), 5);
        staged.emit("metric", json!({"name": "mask_area_median_fraction", "value": fraction}))?;
        staged.artifact("mask_sheet", &out.join("mask-contact-sheet.png"), "Masks on photos (red: dark pixels left out)", json!({}))?;
        if provider.info().name == "threshold"
            && masks["views_filling_over_90_percent_of_photo"].as_array().is_some_and(|views| views.len() == count)
        {
            bail!(
                "threshold masks cover more than 90% of every photo ({count} photos); the dark backdrop may have been selected. \
                 Inspect mask-contact-sheet.png; use a light backdrop, --masks sam, or your own masks with --masks import:DIR"
            );
        }
        staged.emit("stage_finished", json!({"seconds": stage_started.elapsed().as_secs_f64()}))?;
        Ok(photo_map)
    }

    /// Returns the exit code: 0 when the cameras passed, 2 when a gate rejected them.
    fn cameras_stage(&mut self, photo_map: &Value) -> anyhow::Result<u8> {
        let (options, out) = (self.options, self.out);
        let (count, threads, small) = (options.photo_count, options.threads, options.timeouts.small);
        let provider = camera_provider(options);
        let stage_started = Instant::now();
        let staged = self.events.stage("cameras");
        staged.emit("stage_started", json!({"provider": provider.info().name}))?;
        // The declared lens at the photo resolution.
        let (width, height) = (photo_map["width"].as_u64().unwrap_or(0) as u32, photo_map["height"].as_u64().unwrap_or(0) as u32);
        let declared = match &options.calibration {
            Some(path) => Some(scale_calibration(&load_calibration(path)?, width, height)?),
            None => None,
        };
        let lens = declared.as_ref().map(|scaled| scaled.lens(width, height));
        if let Some(scaled) = &declared {
            crate::photos::fs::create_dir_all(out.join("sfm"))?;
            let expected = json!({"pixels": [scaled.fx, scaled.fy, scaled.cx, scaled.cy], "k": scaled.k});
            util::write_json(&out.join("sfm/expected-calibration.json"), &expected, 1)?;
        }
        self.internal("cameras", "contrast", small, 0.0, 0.04, "Contrast images for feature detection", count, |watch| {
            let target = out.join("work/contrast");
            crate::photos::fs::create_dir_all(&target)?;
            let work = |index: usize| -> anyhow::Result<()> {
                let name = capture_name(index);
                let photo = open_photo(&out.join("work/photos").join(&name))?;
                let (width, height) = (photo.width(), photo.height());
                let mut rgb = photo.rgb.into_raw();
                contrast::contrast_image(&mut rgb, width, height, options.contrast_gamma, options.clahe_clip, options.clahe_grid);
                util::save_rgb(&target.join(&name), width, height, rgb)
            };
            util::parallel(count, threads, work, watch).map(|_| ())
        })?;
        if !provider.reads_staged_photos() && !options.keep_intermediates {
            let _ = crate::photos::fs::remove_dir_all(out.join("work/photos"));
        }
        let solution = provider.recover(self, lens.as_ref())?;
        let mut used = json!({"fx": solution.lens.pixels[0], "fy": solution.lens.pixels[1], "cx": solution.lens.pixels[2], "cy": solution.lens.pixels[3],
                              "k": solution.lens.k, "width": solution.lens.width, "height": solution.lens.height});
        if let Some(scaled) = &declared {
            used["scale"] = json!(scaled.scale);
        }
        self.report["lens"] = used;
        let gates =
            self.internal("cameras", "audit", small, 0.82, 0.86, "Checking cameras", 1, |_| judge(options, out, &solution, lens.as_ref()))?;
        self.report["gates"] = gates.clone();
        let ring = &gates["ring"];
        let degenerate = ring["degenerate"].as_bool().unwrap_or(true);
        let rounded = |value: &Value, digits: i32| value.as_f64().map(|v| json!(round_to(v, digits))).unwrap_or(Value::Null);
        let metrics = [
            ("registered_views", gates["registered_views"].clone()),
            ("input_photos", gates["input_photos"].clone()),
            ("reprojection_median_pixels", rounded(&gates["reprojection_pixels"]["median"], 4)),
            ("reprojection_p95_pixels", rounded(&gates["reprojection_pixels"]["p95"], 4)),
            ("ring_radius_spread_percent", if degenerate { Value::Null } else { rounded(&ring["radius_spread_percent"], 3) }),
            ("ring_largest_gap_deg", if degenerate { Value::Null } else { rounded(&ring["largest_angular_gap_deg"], 2) }),
        ];
        for (name, value) in metrics {
            if !value.is_null() {
                staged.emit("metric", json!({"name": name, "value": value}))?;
            }
        }
        let duplicates: Vec<String> =
            ring["duplicate_pose_pairs"].as_array().into_iter().flatten().map(|pair| format!("[{}, {}]", pair[0], pair[1])).collect();
        if !degenerate && !duplicates.is_empty() {
            self.warn(format!("consecutive photos with the same pose (turntable did not move?): [{}]", duplicates.join(", ")));
        }
        if gates["passed"] != true {
            let reasons: Vec<String> =
                gates["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
            self.report["status"] = json!("failed");
            self.report["reasons"] = json!(reasons);
            self.save()?;
            staged.artifact("report", &out.join("frontend.json"), "Front stage report (cameras rejected)", json!({}))?;
            staged.emit("error", json!({"message": format!("cameras rejected: {}", reasons.join("; "))}))?;
            return Ok(2);
        }
        let registered = solution.views.len();
        // Re-read original capture files for colour, without retaining a second staged
        // photo set throughout camera recovery (important for browser memory).
        let texture_photos = photo_map["photos"]
            .as_array()
            .context("photo map has no photos")?
            .iter()
            .map(|row| {
                Ok((
                    row["capture"].as_str().context("photo map has no capture")?.to_string(),
                    options.photos.join(row["source"].as_str().context("photo map has no source")?),
                ))
            })
            .collect::<anyhow::Result<std::collections::BTreeMap<_, _>>>()?;
        let scene = self.internal(
            "cameras",
            "scene",
            small,
            0.86,
            0.98,
            "Undistorting photos and masks, writing the scene",
            registered,
            |watch| {
                write_scene(
                    &solution,
                    &out.join("work/contrast"),
                    Some(&texture_photos),
                    &out.join("masks"),
                    &out.join("inputs"),
                    threads,
                    watch,
                )
            },
        )?;
        self.report["scene"] = scene;
        let overlay = self.internal("cameras", "overlay", small, 0.98, 1.0, "Sparse points on photos", registered, |watch| {
            sheets::step_overlay(out, watch)
        })?;
        self.report["sparse_overlay"] = overlay;
        staged.artifact("sparse_overlay", &out.join("sparse-overlay.png"), "Sparse points on undistorted photos and masks", json!({}))?;
        let mut bulky = vec!["work/photos", "work/contrast"];
        bulky.extend(provider.intermediates());
        self.remove_intermediates(&bulky);
        let shown = |path: PathBuf| path.to_string_lossy().to_string();
        self.report["outputs"] = json!({"inputs": shown(out.join("inputs")), "masks": shown(out.join("masks"))});
        self.report["status"] = json!("complete");
        self.save()?;
        staged.artifact("report", &out.join("frontend.json"), "Front stage report", json!({}))?;
        staged.emit("stage_finished", json!({"seconds": stage_started.elapsed().as_secs_f64()}))?;
        Ok(0)
    }

    fn remove_intermediates(&mut self, names: &[&str]) {
        let mut deleted = Vec::new();
        if !self.options.keep_intermediates {
            for name in names {
                let path = self.out.join(name);
                let _ = if path.stored_dir() { crate::photos::fs::remove_dir_all(&path) } else { crate::photos::fs::remove_file(&path) };
                deleted.push(*name);
            }
        }
        self.report["intermediates_deleted"] = json!(deleted);
    }
}

/// Runs the stage into the fresh directory `options.output`. Events go to
/// `events` under the stages `masks` and `cameras` (no `run_started` or
/// `run_finished`: those belong to whatever drives the run). A `cancel` file
/// next to the event log (`events_path`), or `cancel` set, stops the run.
pub fn run(options: &Options, events: &EventLog, events_path: &Path, cancel: Option<Arc<AtomicBool>>) -> anyhow::Result<Finished> {
    let out = options.output.as_path();
    if out.stored() {
        bail!("output exists: {}", out.display());
    }
    // Missing external programs are reported before anything is written.
    mask_provider(options).check(options)?;
    if !options.stop_after_masks {
        camera_provider(options).check(options)?;
    }
    crate::photos::fs::create_dir_all(out.join("logs")).with_context(|| out.display().to_string())?;
    let configuration = options.to_json();
    util::write_json(&out.join("frontend-config.json"), &configuration, 1)?;
    let report = json!({
        "schema": SCHEMA, "status": "running", "reasons": [], "warnings": [], "configuration": configuration,
        "events": events_path.to_string_lossy(), "steps": [], "gates": null, "masks": null,
        "providers": {"masks": options.masks.name(), "cameras": if options.stop_after_masks { Value::Null } else { json!(options.cameras.name()) }},
        "reference_geometry_used": false, "supplied_poses_used": matches!(options.cameras, super::options::CameraChoice::Import(_)), "depth_used": false,
    });
    let mut run = Run {
        options,
        out,
        events: events.clone(),
        cancel_file: events_path.parent().unwrap_or(Path::new(".")).join("cancel"),
        cancel,
        report,
        started: Instant::now(),
    };
    let mut stage = "masks";
    let outcome = (|| -> anyhow::Result<u8> {
        let photo_map = run.masks_stage()?;
        if options.stop_after_masks {
            run.remove_intermediates(&["work/photos"]);
            run.report["outputs"] = json!({"masks": out.join("masks").to_string_lossy()});
            run.report["status"] = json!("complete");
            run.report["stopped_after"] = json!("masks");
            run.save()?;
            return Ok(0);
        }
        stage = "cameras";
        run.cameras_stage(&photo_map)
    })();
    let code = match outcome {
        Ok(code) => code,
        Err(error) => {
            let text = format!("{error:#}");
            run.report["status"] = json!("failed");
            run.report["reasons"] = json!([text]);
            run.save()?;
            let skip = text.chars().count().saturating_sub(600);
            events.stage(stage).emit("error", json!({"message": text.chars().skip(skip).collect::<String>()}))?;
            1
        }
    };
    Ok(Finished { report: run.report, code })
}

/// The few lines the command prints at the end.
pub fn summary(report: &Value) -> Value {
    let gates = &report["gates"];
    json!({
        "status": report["status"], "reasons": report["reasons"], "warnings": report["warnings"], "providers": report["providers"],
        "registered_views": gates["registered_views"], "reprojection_pixels": gates["reprojection_pixels"],
        "seconds": report["seconds"],
        "report": Path::new(report["configuration"]["output"].as_str().unwrap_or_default()).join("frontend.json").to_string_lossy(),
    })
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
