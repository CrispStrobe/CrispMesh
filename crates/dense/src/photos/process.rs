//! External tools as bounded child processes: own process group, deadline and
//! cancel, output into a log file (`bounded` and `stop_group` of `dense_pipeline.py`).

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use web_time::Instant;

use anyhow::{anyhow, Context};
use serde_json::{json, Value};

/// One external step: its name, command line and the variables added to the inherited environment.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalCommand {
    pub name: String,
    pub command: Vec<String>,
    pub environment: Vec<(String, String)>,
}

/// How a bounded step ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub command: Vec<String>,
    /// `None` when the step was stopped; a negative number is a signal on Unix.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub seconds: f64,
    pub log: String,
}

impl Outcome {
    pub fn succeeded(&self) -> bool {
        !self.timed_out && !self.cancelled && self.exit_code == Some(0)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "command": self.command, "exit_code": self.exit_code, "timed_out": self.timed_out, "cancelled": self.cancelled,
            "seconds": self.seconds, "log": self.log,
        })
    }
}

#[cfg(unix)]
fn own_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn own_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn own_group(_command: &mut Command) {}

/// Ends a step and everything it started.
#[cfg(unix)]
fn stop_group(child: &mut Child, force: bool) {
    // SAFETY: plain system call on the process group this module created (its id is the child's pid).
    unsafe {
        libc::killpg(child.id() as libc::pid_t, if force { libc::SIGKILL } else { libc::SIGTERM });
    }
}

#[cfg(windows)]
fn stop_group(child: &mut Child, _force: bool) {
    // No process groups to signal: end the process tree.
    let tree =
        Command::new("taskkill").args(["/T", "/F", "/PID", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    if tree.is_err() {
        let _ = child.kill();
    }
}

#[cfg(not(any(unix, windows)))]
fn stop_group(child: &mut Child, _force: bool) {
    let _ = child.kill();
}

#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.code().or_else(|| status.signal().map(|signal| -signal))
}

#[cfg(not(unix))]
fn exit_code(status: std::process::ExitStatus) -> Option<i32> {
    status.code()
}

/// Runs `command` in its own process group with standard output and error in
/// `log`. `watch` is called about four times a second; when it returns `Some`
/// the whole group is stopped (terminated, then killed after five seconds) and
/// the outcome says why: `true` for a cancel, `false` for the deadline, which
/// is also enforced here. Variables in `environment` are added to the
/// inherited ones.
pub fn bounded(
    command: &[String],
    log: &Path,
    timeout: Duration,
    environment: &[(String, String)],
    directory: &Path,
    watch: &mut dyn FnMut() -> Option<bool>,
) -> anyhow::Result<Outcome> {
    let started = Instant::now();
    let (program, arguments) = command.split_first().ok_or_else(|| anyhow!("empty command"))?;
    let output = std::fs::File::create(log).with_context(|| log.display().to_string())?;
    let mut process = Command::new(program);
    process.args(arguments).stdin(Stdio::null()).stdout(output.try_clone()?).stderr(output).current_dir(directory);
    for (name, value) in environment {
        process.env(name, value);
    }
    own_group(&mut process);
    let mut child = process.spawn().map_err(|error| anyhow!("cannot start {program}: {error}"))?;
    let (mut timed_out, mut cancelled) = (false, false);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        match watch() {
            Some(true) => cancelled = true,
            Some(false) => timed_out = true,
            None => timed_out = started.elapsed() > timeout,
        }
        if timed_out || cancelled {
            stop_group(&mut child, false);
            let patience = Instant::now();
            while child.try_wait()?.is_none() {
                if patience.elapsed() > Duration::from_secs(5) {
                    stop_group(&mut child, true);
                    child.wait()?;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            break None;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    watch();
    Ok(Outcome {
        command: command.to_vec(),
        exit_code: status.and_then(exit_code),
        timed_out,
        cancelled,
        seconds: started.elapsed().as_secs_f64(),
        log: log.to_string_lossy().to_string(),
    })
}

/// The end of a log file for an error message: at most `limit` characters.
pub fn log_tail(log: &Path, limit: usize) -> String {
    let text = String::from_utf8_lossy(&std::fs::read(log).unwrap_or_default()).to_string();
    let skip = text.chars().count().saturating_sub(limit);
    text.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split(' ').map(String::from).collect()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("crispmesh-process-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn a_missing_program_is_a_clear_error() {
        let folder = scratch("missing");
        let error =
            bounded(&words("/no/such/aliceVision_cameraInit --x"), &folder.join("log"), Duration::from_secs(5), &[], &folder, &mut || None)
                .unwrap_err();
        assert!(error.to_string().starts_with("cannot start /no/such/aliceVision_cameraInit: "), "{error}");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn output_exit_code_environment_deadline_and_cancel() {
        let folder = scratch("bounded");
        let shell = |script: &str| vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()];
        let environment = [("CRISPMESH_TEST_VALUE".to_string(), "seven".to_string())];
        let log = folder.join("one.log");
        let outcome = bounded(
            &shell("echo out $CRISPMESH_TEST_VALUE; pwd; echo err >&2; exit 3"),
            &log,
            Duration::from_secs(20),
            &environment,
            &folder,
            &mut || None,
        )
        .unwrap();
        assert_eq!((outcome.exit_code, outcome.timed_out, outcome.cancelled, outcome.succeeded()), (Some(3), false, false, false));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("out seven") && text.contains("err"), "{text}");
        assert!(text.contains(folder.canonicalize().unwrap().file_name().unwrap().to_str().unwrap()));
        assert_eq!(log_tail(&log, 4), "err\n");
        assert!(bounded(&shell("true"), &log, Duration::from_secs(20), &[], &folder, &mut || None).unwrap().succeeded());
        // The deadline stops the whole group: the grandchild must not write its file afterwards.
        let marker = folder.join("late");
        let script = format!("(sleep 2; touch {}) & sleep 30", marker.display());
        let started = Instant::now();
        let outcome = bounded(&shell(&script), &log, Duration::from_millis(300), &[], &folder, &mut || None).unwrap();
        assert_eq!((outcome.exit_code, outcome.timed_out, outcome.cancelled), (None, true, false));
        assert!(started.elapsed() < Duration::from_secs(10));
        let mut polls = 0;
        let outcome = bounded(&shell("sleep 30"), &log, Duration::from_secs(60), &[], &folder, &mut || {
            polls += 1;
            (polls > 2).then_some(true)
        })
        .unwrap();
        assert_eq!((outcome.exit_code, outcome.timed_out, outcome.cancelled), (None, false, true));
        std::thread::sleep(Duration::from_millis(2300));
        assert!(!marker.exists(), "the process group survived its deadline");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
