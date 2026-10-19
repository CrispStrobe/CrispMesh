//! The engine as a child process of the desktop shell: start it on a free localhost port
//! with a random token, wait until it answers, keep the tail of what it prints, and take it
//! down again together with its process group.
//!
//! The shell does not bundle Python. It runs the interpreter it is told about, in the
//! checkout it is told about.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::Resolved;

const LOG_LINES: usize = 200;
const START_TIMEOUT: Duration = Duration::from_secs(45);
const STOP_GRACE: Duration = Duration::from_secs(4);

/// Process group of the running engine, for the signal handler (which may only touch atomics).
static ENGINE_GROUP: AtomicI32 = AtomicI32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// What the web view is told. The token is real: the page needs it to talk to the engine.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub state: State,
    pub url: Option<String>,
    pub token: Option<String>,
    /// Why it is not running, in a sentence.
    pub message: Option<String>,
    /// The command that was started, with the token blanked.
    pub command: Option<String>,
    /// The last lines the engine printed (stdout and stderr).
    pub log: Vec<String>,
    pub device: Option<String>,
}

struct Inner {
    state: State,
    child: Option<Child>,
    url: Option<String>,
    token: Option<String>,
    message: Option<String>,
    command: Option<String>,
    device: Option<String>,
    log: Arc<Mutex<VecDeque<String>>>,
    /// Counts starts, so a start that was overtaken by a newer one gives up quietly.
    generation: u64,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Mutex<Inner>>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            inner: Arc::new(Mutex::new(Inner {
                state: State::Stopped,
                child: None,
                url: None,
                token: None,
                message: None,
                command: None,
                device: None,
                log: Arc::new(Mutex::new(VecDeque::new())),
                generation: 0,
            })),
        }
    }
}

/// The engine's command line. `None` for the token blanks it, for display.
pub fn arguments(resolved: &Resolved, port: u16, token: Option<&str>) -> Vec<String> {
    let mut arguments: Vec<String> = ["-m", "scripts.turntable_mesh.engine_server"].iter().map(|s| s.to_string()).collect();
    let pairs = [
        ("--runs", resolved.runs_dir.value.clone()),
        ("--data", resolved.data_dir.value.clone()),
        ("--host", "127.0.0.1".to_string()),
        ("--port", port.to_string()),
        ("--token", token.unwrap_or("<hidden>").to_string()),
        ("--device", resolved.device.value.clone()),
        ("--python", resolved.python.value.clone()),
        ("--torch-python", resolved.torch_python.value.clone()),
    ];
    for (flag, value) in pairs {
        arguments.push(flag.to_string());
        arguments.push(value);
    }
    if let Ok(tools) = std::env::var("CRISPMESH_CAPTURE_TOOLS") {
        if !tools.is_empty() { arguments.extend(["--capture-tools".into(), tools]); }
    }
    arguments
}

fn display_command(resolved: &Resolved, port: u16) -> String {
    let quote = |text: &String| if text.contains(' ') { format!("\"{text}\"") } else { text.clone() };
    let mut parts = vec![format!("PYTHONPATH={}", quote(&resolved.repo.value)), quote(&resolved.python.value)];
    parts.extend(arguments(resolved, port, None).iter().map(quote));
    parts.join(" ")
}

fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|error| format!("No random numbers from the system: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// True when `response` is the engine's answer to `GET /api/health`.
pub fn is_healthy(response: &str) -> bool {
    let status_ok = response.lines().next().is_some_and(|line| line.split_whitespace().nth(1) == Some("200"));
    status_ok && response.contains("crispmesh_dense_events")
}

fn ask_health(port: u16, token: &str) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    let _ = stream.take(4096).read_to_string(&mut response);
    is_healthy(&response)
}

fn keep_tail(log: &Arc<Mutex<VecDeque<String>>>, stream: impl Read + Send + 'static) {
    let log = Arc::clone(log);
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let mut lines = log.lock().unwrap();
            if lines.len() == LOG_LINES {
                lines.pop_front();
            }
            lines.push_back(line);
        }
    });
}

fn spawn(resolved: &Resolved, port: u16, token: &str) -> std::io::Result<Child> {
    let mut command = Command::new(&resolved.python.value);
    command
        .args(arguments(resolved, port, Some(token)))
        .current_dir(&resolved.repo.value)
        .env("PYTHONPATH", &resolved.repo.value)
        .env("PYTHONUNBUFFERED", "1")
        .env("CRISPMESH_PYTHON", &resolved.python.value)
        .env("CRISPMESH_TORCH_PYTHON", &resolved.torch_python.value)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so the whole group can be signalled without touching ours.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    command.spawn()
}

/// Ends the child and everything in its process group: politely first, then for good.
fn terminate(mut child: Child) {
    let id = child.id();
    #[cfg(unix)]
    {
        // SAFETY: plain signal delivery to a process group this process created.
        unsafe { libc::killpg(id as libc::pid_t, libc::SIGTERM) };
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                // The leader is gone; make sure nothing of its group lingers.
                unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::killpg(id as libc::pid_t, libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // /T takes the child's whole process tree, /F does not wait for it to agree.
        let _ = Command::new("taskkill")
            .args(["/PID", &id.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = STOP_GRACE;
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Engine {
    pub fn status(&self) -> Status {
        let mut inner = self.inner.lock().unwrap();
        // An engine that died on its own is noticed here, the next time anyone asks.
        if inner.state == State::Running {
            if let Some(Ok(Some(exit))) = inner.child.as_mut().map(Child::try_wait) {
                inner.state = State::Failed;
                inner.child = None;
                inner.url = None;
                inner.token = None;
                inner.message = Some(format!("The engine stopped by itself ({exit})."));
                ENGINE_GROUP.store(0, Ordering::SeqCst);
            }
        }
        let log: Vec<String> = inner.log.lock().unwrap().iter().cloned().collect();
        Status {
            state: inner.state,
            url: inner.url.clone(),
            token: inner.token.clone(),
            message: inner.message.clone(),
            command: inner.command.clone(),
            log,
            device: inner.device.clone(),
        }
    }

    /// Stops whatever runs and starts afresh in the background. Returns at once.
    pub fn restart(&self, resolved: Resolved) {
        let engine = self.clone();
        let (generation, previous) = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.state = State::Starting;
            inner.url = None;
            inner.token = None;
            inner.message = None;
            inner.command = None;
            inner.device = Some(resolved.device.value.clone());
            inner.log = Arc::new(Mutex::new(VecDeque::new()));
            (inner.generation, inner.child.take())
        };
        std::thread::spawn(move || {
            if let Some(child) = previous {
                ENGINE_GROUP.store(0, Ordering::SeqCst);
                terminate(child);
            }
            engine.start(generation, resolved);
        });
    }

    fn fail(&self, generation: u64, message: String) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        inner.state = State::Failed;
        inner.message = Some(message);
        inner.url = None;
        inner.token = None;
    }

    fn start(&self, generation: u64, resolved: Resolved) {
        if let Some(problem) = crate::config::problems(&resolved) {
            return self.fail(generation, problem);
        }
        for folder in [&resolved.runs_dir.value, &resolved.data_dir.value] {
            if let Err(error) = std::fs::create_dir_all(folder) {
                return self.fail(generation, format!("Could not create the folder {folder}: {error}"));
            }
        }
        let (port, token) = match (free_port(), random_token()) {
            (Ok(port), Ok(token)) => (port, token),
            (Err(error), _) => return self.fail(generation, format!("No free port on this computer: {error}")),
            (_, Err(error)) => return self.fail(generation, error),
        };
        let command = display_command(&resolved, port);
        let mut child = match spawn(&resolved, port, &token) {
            Ok(child) => child,
            Err(error) => {
                self.inner.lock().unwrap().command = Some(command);
                return self.fail(
                    generation,
                    format!(
                        "Python could not be started ({}): {error}. Check the interpreter on the settings screen.",
                        resolved.python.value
                    ),
                );
            }
        };
        let log = {
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                drop(inner);
                return terminate(child);
            }
            inner.command = Some(command);
            Arc::clone(&inner.log)
        };
        if let Some(stream) = child.stdout.take() {
            keep_tail(&log, stream);
        }
        if let Some(stream) = child.stderr.take() {
            keep_tail(&log, stream);
        }
        ENGINE_GROUP.store(child.id() as i32, Ordering::SeqCst);

        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Ok(Some(exit)) = child.try_wait() {
                ENGINE_GROUP.store(0, Ordering::SeqCst);
                // Give the reader threads a moment to collect the last lines.
                std::thread::sleep(Duration::from_millis(150));
                return self.fail(
                    generation,
                    format!("The engine stopped right after starting ({exit}). Its last output is below."),
                );
            }
            if ask_health(port, &token) {
                break;
            }
            if self.inner.lock().unwrap().generation != generation {
                ENGINE_GROUP.store(0, Ordering::SeqCst);
                return terminate(child);
            }
            if Instant::now() > deadline {
                ENGINE_GROUP.store(0, Ordering::SeqCst);
                terminate(child);
                return self.fail(
                    generation,
                    format!("The engine did not answer within {} seconds.", START_TIMEOUT.as_secs()),
                );
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            drop(inner);
            ENGINE_GROUP.store(0, Ordering::SeqCst);
            return terminate(child);
        }
        inner.state = State::Running;
        inner.url = Some(format!("http://127.0.0.1:{port}"));
        inner.token = Some(token);
        inner.message = None;
        inner.child = Some(child);
    }

    /// Stops the engine and waits until it is gone. Called when the app exits.
    pub fn stop(&self) {
        let child = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.state = State::Stopped;
            inner.url = None;
            inner.token = None;
            inner.child.take()
        };
        ENGINE_GROUP.store(0, Ordering::SeqCst);
        if let Some(child) = child {
            terminate(child);
        }
    }
}

/// If the app itself is told to go by a signal (Ctrl-C under `tauri dev`, `kill`), the normal
/// exit path does not run. Take the engine's group along from the handler.
#[cfg(unix)]
pub fn install_signal_handlers() {
    extern "C" fn on_signal(signal: libc::c_int) {
        let group = ENGINE_GROUP.load(Ordering::SeqCst);
        // SAFETY: killpg and _exit are async-signal-safe; nothing else is done here.
        unsafe {
            if group > 0 {
                libc::killpg(group, libc::SIGTERM);
            }
            libc::_exit(128 + signal);
        }
    }
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: installing a handler that only calls async-signal-safe functions.
        unsafe { libc::signal(signal, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
}

#[cfg(not(unix))]
pub fn install_signal_handlers() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Source, Value};

    fn resolved() -> Resolved {
        let value = |text: &str| Value { value: text.to_string(), source: Source::Setting };
        Resolved {
            repo: value("/src/crispmesh"),
            python: value("/venv/bin/python"),
            torch_python: value("/torch venv/bin/python"),
            runs_dir: value("/runs"),
            data_dir: value("/data"),
            device: value("cpu"),
        }
    }

    #[test]
    fn the_command_line_is_the_documented_one() {
        assert_eq!(
            arguments(&resolved(), 8123, Some("s3cret")),
            [
                "-m", "scripts.turntable_mesh.engine_server", "--runs", "/runs", "--data", "/data", "--host", "127.0.0.1",
                "--port", "8123", "--token", "s3cret", "--device", "cpu", "--python", "/venv/bin/python", "--torch-python",
                "/torch venv/bin/python",
            ]
        );
    }

    #[test]
    fn the_displayed_command_hides_the_token_and_quotes_spaces() {
        let shown = display_command(&resolved(), 8123);
        assert!(shown.starts_with("PYTHONPATH=/src/crispmesh /venv/bin/python -m scripts.turntable_mesh.engine_server"));
        assert!(shown.contains("--token <hidden>"));
        assert!(shown.contains("\"/torch venv/bin/python\""));
    }

    #[test]
    fn health_answers_are_recognised() {
        let ok = "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"schema\": \"crispmesh_dense_events_v1\", \"device\": \"cpu\"}";
        assert!(is_healthy(ok));
        assert!(!is_healthy("HTTP/1.0 401 Unauthorized\r\n\r\n{\"error\": \"unauthorised\"}"));
        assert!(!is_healthy("HTTP/1.1 200 OK\r\n\r\n<html>some other server</html>"));
        assert!(!is_healthy(""));
    }

    #[test]
    fn tokens_are_long_and_differ() {
        let (a, b) = (random_token().unwrap(), random_token().unwrap());
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_missing_interpreter_fails_with_a_sentence_and_no_token() {
        let repo = std::env::temp_dir().join(format!("crispmesh-studio-engine-test-{}", std::process::id()));
        std::fs::create_dir_all(repo.join("scripts/turntable_mesh")).unwrap();
        std::fs::write(repo.join(crate::config::ENGINE_MODULE), "").unwrap();
        let mut settings = resolved();
        settings.repo.value = repo.to_string_lossy().into_owned();
        settings.python.value = "/definitely/not/python".into();
        settings.runs_dir.value = repo.join("runs").to_string_lossy().into_owned();
        settings.data_dir.value = repo.join("data").to_string_lossy().into_owned();
        let engine = Engine::default();
        engine.restart(settings);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            let status = engine.status();
            if status.state != State::Starting || Instant::now() > deadline {
                break status;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.state, State::Failed);
        assert!(status.message.unwrap().contains("Python could not be started"));
        assert!(status.token.is_none() && status.url.is_none());
        assert!(status.command.unwrap().contains("--token <hidden>"));
    }

    /// Start a stand-in "engine" that spawns a grandchild, then stop it: both must be gone.
    #[cfg(unix)]
    #[test]
    fn stopping_takes_the_whole_process_group() {
        use std::os::unix::process::CommandExt;
        let marker = std::env::temp_dir().join(format!("crispmesh-studio-group-{}.pid", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("sleep 300 & echo $! > '{}'; wait", marker.display());
        let mut command = Command::new("sh");
        command.args(["-c", &script]).process_group(0);
        let child = command.spawn().unwrap();
        let leader = child.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(5);
        let grandchild: libc::pid_t = loop {
            if let Some(pid) = std::fs::read_to_string(&marker).ok().and_then(|text| text.trim().parse().ok()) {
                break pid;
            }
            assert!(Instant::now() < deadline, "the stand-in never started its child");
            std::thread::sleep(Duration::from_millis(20));
        };
        let alive = |pid: libc::pid_t| unsafe { libc::kill(pid, 0) } == 0;
        assert!(alive(leader) && alive(grandchild));
        terminate(child);
        std::thread::sleep(Duration::from_millis(200));
        assert!(!alive(leader), "the leader survived");
        assert!(!alive(grandchild), "the grandchild survived");
        let _ = std::fs::remove_file(&marker);
    }
}
