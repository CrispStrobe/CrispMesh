//! Run event log, the same contract as `scripts/turntable_mesh/dense_events.py`
//! and `docs/ENGINE-CONTRACT.md`: one JSON object per line, appended.
//!
//! An event goes to the log file, to an observer (a callback of an embedding
//! application), or to both. Stages hold a log with their stage name; the run
//! driver hands each stage a view of one shared log.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use web_time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

pub const SCHEMA: &str = "crispmesh_dense_events_v1";

/// Receives every event as it is emitted, on the thread that emits it.
pub type Observer = Arc<dyn Fn(&Value) + Send + Sync>;

/// Append-only writer. `EventLog::none()` discards everything.
#[derive(Clone)]
pub struct EventLog {
    path: Option<PathBuf>,
    /// The run directory; artifact paths are stored relative to it.
    root: Option<PathBuf>,
    stage: Option<String>,
    observer: Option<Observer>,
}

impl EventLog {
    pub fn new(path: Option<&Path>, stage: &str) -> Self {
        let root = path.map(|p| p.parent().unwrap_or(Path::new(".")).to_path_buf());
        EventLog { path: path.map(Path::to_path_buf), root, stage: Some(stage.to_string()), observer: None }
    }

    pub fn none() -> Self {
        EventLog { path: None, root: None, stage: None, observer: None }
    }

    /// Run-level log (stage null) in `run_directory/events.jsonl`, also reported to `observer`.
    pub fn for_run(run_directory: &Path, observer: Option<Observer>) -> Self {
        EventLog { path: Some(run_directory.join("events.jsonl")), root: Some(run_directory.to_path_buf()), stage: None, observer }
    }

    /// The same log for another stage.
    pub fn stage(&self, name: &str) -> Self {
        EventLog { stage: Some(name.to_string()), ..self.clone() }
    }

    /// One write call per line, so appends from several stage processes never interleave.
    pub fn emit(&self, event_type: &str, fields: Value) -> anyhow::Result<()> {
        if self.path.is_none() && self.observer.is_none() {
            return Ok(());
        }
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
        let mut record = Map::new();
        record.insert("type".into(), json!(event_type));
        record.insert("time".into(), json!(time));
        record.insert("stage".into(), json!(self.stage));
        if let Value::Object(extra) = fields {
            record.extend(extra);
        }
        let record = Value::Object(record);
        if let Some(path) = &self.path {
            let mut line = serde_json::to_string(&record)?;
            line.push('\n');
            crate::storage::append(path, line.as_bytes())?;
        }
        if let Some(observer) = &self.observer {
            observer(&record);
        }
        Ok(())
    }

    pub fn progress(&self, fraction: f64, message: &str) -> anyhow::Result<()> {
        let fraction = (fraction.clamp(0.0, 1.0) * 1e4).round() / 1e4;
        self.emit("progress", json!({"fraction": fraction, "message": message}))
    }

    pub fn metric(&self, name: &str, value: f64) -> anyhow::Result<()> {
        self.emit("metric", json!({"name": name, "value": value}))
    }

    /// `path` must lie under the run directory (the directory of the event log).
    pub fn artifact(&self, kind: &str, path: &Path, label: &str, extra: Value) -> anyhow::Result<()> {
        let Some(root) = &self.root else { return Ok(()) };
        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative = relative.to_string_lossy().replace('\\', "/");
        let mut fields = json!({"kind": kind, "path": relative, "label": label});
        if let (Value::Object(target), Value::Object(more)) = (&mut fields, extra) {
            target.extend(more);
        }
        self.emit("artifact", fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_json_with_stage_and_relative_paths() {
        let dir = std::env::temp_dir().join(format!("crispmesh-events-{}", std::process::id()));
        crate::storage::create_dir_all(dir.join("mesh")).unwrap();
        let log_path = dir.join("events.jsonl");
        let _ = crate::storage::remove_file(&log_path);
        let log = EventLog::new(Some(&log_path), "mesh");
        log.progress(0.33333, "working").unwrap();
        log.artifact("final_mesh", &dir.join("mesh/mesh.stl"), "Final surface", json!({"triangles": 12})).unwrap();
        let text = crate::storage::read_to_string(&log_path).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["stage"], "mesh");
        assert_eq!(lines[0]["fraction"], 0.3333);
        assert_eq!(lines[1]["path"], "mesh/mesh.stl");
        assert_eq!(lines[1]["triangles"], 12);
        EventLog::none().progress(0.5, "ignored").unwrap();
        crate::storage::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_run_log_reaches_the_file_and_the_observer_under_every_stage() {
        let dir = std::env::temp_dir().join(format!("crispmesh-events-run-{}", std::process::id()));
        let _ = crate::storage::remove_dir_all(&dir);
        crate::storage::create_dir_all(&dir).unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let sink = seen.clone();
        let log = EventLog::for_run(&dir, Some(Arc::new(move |event: &Value| sink.lock().unwrap().push(event.clone()))));
        log.emit("run_started", json!({"schema": SCHEMA})).unwrap();
        log.stage("stereo").artifact("depth_sheet", &dir.join("stereo/depth-level-0.png"), "Depth", json!({"level": 0})).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0]["stage"].is_null());
        assert_eq!((seen[1]["stage"].as_str(), seen[1]["path"].as_str()), (Some("stereo"), Some("stereo/depth-level-0.png")));
        assert_eq!(crate::storage::read_to_string(dir.join("events.jsonl")).unwrap().lines().count(), 2);
        crate::storage::remove_dir_all(&dir).unwrap();
    }
}
