//! "Let other devices use this computer": the built-in engine over HTTP on the local network, as
//! docs/ENGINE-CONTRACT.md section 3 describes it, plus `POST /api/uploads` (photos taken on a phone
//! are saved into this computer's data folder, then a run starts here and the phone follows it).
//!
//! Every request needs `Authorization: Bearer <token>`; the token is random per start and shown
//! only in this app. Plain HTTP on the local network, no TLS: for a trusted network. One thread per
//! request, `Connection: close`; uploads are one file per request and limited in size.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::native::{Native, StartBody};

/// Largest request body: one photo of a phone or a camera.
const MAX_BODY: usize = 96 * 1024 * 1024;
/// Largest request head (request line and headers).
const MAX_HEAD: usize = 32 * 1024;

/// The running server: its port and token, and how to stop it.
pub struct Sharing {
    pub port: u16,
    pub token: String,
    stop: Arc<AtomicBool>,
}

impl Sharing {
    /// Listens on every interface at `port` (0: any free port) and serves `engine` until stopped.
    pub fn start(engine: Arc<Native>, port: u16, token: String) -> Result<Sharing, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|error| format!("cannot listen on port {port}: {error}"))?;
        let port = listener.local_addr().map_err(|error| error.to_string())?.port();
        listener.set_nonblocking(true).map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, secret) = (stop.clone(), token.clone());
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (engine, secret) = (engine.clone(), secret.clone());
                        std::thread::spawn(move || serve(stream, &engine, &secret));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        Ok(Sharing { port, token, stop })
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// What the app shows: addresses another device can type, and the token.
    pub fn describe(&self) -> Value {
        let addresses: Vec<String> = local_address().into_iter().map(|ip| format!("http://{ip}:{}", self.port)).collect();
        json!({"running": true, "port": self.port, "token": self.token, "addresses": addresses})
    }
}

impl Drop for Sharing {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The app's sharing state (none while off).
#[derive(Default)]
pub struct Shared(pub Mutex<Option<Sharing>>);

/// This computer's address on the local network: the one the system would use to reach the
/// internet (no packet is sent; a UDP socket only picks its route).
fn local_address() -> Option<std::net::IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }
}

/// `%E2%82%AC` -> `€`, `+` stays (paths and encodeURIComponent never write it for a space).
fn decode(text: &str) -> Option<String> {
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

fn read_request(stream: &mut TcpStream) -> Result<Request, (u16, String)> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| (500, error.to_string()))?);
    let mut head = Vec::new();
    loop {
        let mut line = Vec::new();
        let read = reader.read_until(b'\n', &mut line).map_err(|error| (400, error.to_string()))?;
        if read == 0 {
            return Err((400, "connection closed".into()));
        }
        head.extend_from_slice(&line);
        if head.len() > MAX_HEAD {
            return Err((431, "request head too large".into()));
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
    }
    let text = String::from_utf8(head).map_err(|_| (400, "request head is not UTF-8".to_string()))?;
    let mut lines = text.lines();
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or_default().to_string());
    let headers: Vec<(String, String)> =
        lines.filter_map(|line| line.split_once(':')).map(|(key, value)| (key.trim().to_string(), value.trim().to_string())).collect();
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let path = decode(raw_path).ok_or((400, "bad path".to_string()))?;
    let query = raw_query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((decode(key)?, decode(value)?))
        })
        .collect();
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse::<usize>().map_err(|_| (400, "bad Content-Length".to_string())))
        .transpose()?
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err((413, format!("body larger than {} MB", MAX_BODY / 1024 / 1024)));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(|error| (400, error.to_string()))?;
    Ok(Request { method, path, query, headers, body })
}

/// The status line's reason for the codes used here.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    }
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8], head_only: bool) {
    // Open CORS: the phone app's page has its own origin; the token, not the origin, is the guard.
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: Authorization, Content-Type\r\nAccess-Control-Allow-Methods: GET, HEAD, POST, OPTIONS\r\n\
         Access-Control-Expose-Headers: Content-Length\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    if !head_only {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}

fn json_answer(stream: &mut TcpStream, status: u16, value: &Value) {
    respond(stream, status, "application/json", value.to_string().as_bytes(), false);
}

/// Equal tokens, compared without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "json" => "application/json",
        "jsonl" => "application/x-ndjson",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "stl" => "model/stl",
        "glb" => "model/gltf-binary",
        "txt" | "log" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn serve(mut stream: TcpStream, engine: &Arc<Native>, token: &str) {
    // On macOS and the BSDs an accepted socket inherits the listener's non-blocking mode: a large answer (a 50 MB
    // mesh) or upload would then stop at the first full buffer.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(300)));
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err((status, message)) => return json_answer(&mut stream, status, &json!({"error": message})),
    };
    if request.method == "OPTIONS" {
        return respond(&mut stream, 204, "text/plain", b"", false);
    }
    let given = request.header("authorization").and_then(|value| value.strip_prefix("Bearer ")).unwrap_or_default();
    if !same(given.trim(), token) {
        return json_answer(&mut stream, 401, &json!({"error": "this engine needs its token (shown on the computer)"}));
    }
    let (status, answer) = route(&request, engine, &mut stream);
    if let Some(value) = answer {
        json_answer(&mut stream, status, &value);
    }
}

/// Answers a request; `None` when the answer (a file) was already written.
fn route(request: &Request, engine: &Arc<Native>, stream: &mut TcpStream) -> (u16, Option<Value>) {
    let error = |status: u16, message: String| (status, Some(json!({"error": message})));
    let parts: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["api", "health"]) => (
            200,
            Some(json!({
                "schema": crispmesh_dense::events::SCHEMA,
                "device": "wgpu",
                "can_start_runs": true,
                "start_points": engine.start_points(),
                "methods": crispmesh_dense::run::methods(),
                "uploads": true,
                "label": "Shared computer",
            })),
        ),
        ("GET", ["api", "settings"]) => (200, Some(crate::native::settings_schema())),
        ("GET", ["api", "runs"]) => (200, Some(json!({"runs": engine.list_runs()}))),
        ("POST", ["api", "runs"]) => match serde_json::from_slice::<StartBody>(&request.body) {
            Ok(body) => match engine.start(body) {
                Ok(id) => (201, Some(json!({"id": id}))),
                Err(message) => error(400, message),
            },
            Err(problem) => error(400, format!("JSON body required: {problem}")),
        },
        ("GET", ["api", "runs", id, "events"]) => {
            let since = request.param("since").and_then(|value| value.parse().ok()).unwrap_or(0);
            match engine.events(id, since).map(|page| serde_json::to_value(page).unwrap_or(Value::Null)) {
                Ok(page) => (200, Some(page)),
                Err(message) => error(404, message),
            }
        }
        ("POST", ["api", "runs", id, "cancel"]) => match engine.cancel(id) {
            Ok(_) => (200, Some(json!({"id": id, "cancel_requested": true}))),
            Err(message) => error(404, message),
        },
        (method @ ("GET" | "HEAD"), ["api", "runs", id, "files", rest @ ..]) => {
            let relative = rest.join("/");
            match engine.file(id, &relative).and_then(|path| std::fs::read(path).map_err(|_| "not found".to_string())) {
                Ok(bytes) => {
                    respond(stream, 200, content_type(&relative), &bytes, method == "HEAD");
                    (200, None)
                }
                Err(message) => error(404, message),
            }
        }
        ("GET", ["api", "data"]) => match engine.list_data(request.param("path").unwrap_or("")) {
            Ok(listing) => (200, Some(serde_json::to_value(listing).unwrap_or(Value::Null))),
            Err(message) => error(400, message),
        },
        // One file per request, into <data>/<folder>/<name>; the answer is its path in the data folder.
        ("POST", ["api", "uploads"]) => match (request.param("folder"), request.param("name")) {
            (Some(folder), Some(name)) => match engine.import(folder, name, &request.body) {
                Ok(path) => (201, Some(json!({"path": path}))),
                Err(message) => error(400, message),
            },
            _ => error(400, "folder and name are needed".into()),
        },
        (_, ["api", ..]) => error(404, format!("no {} {}", request.method, request.path)),
        _ => error(404, "not found".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(port: u16, raw: &[u8]) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(raw).unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        let text = String::from_utf8_lossy(&answer).to_string();
        let status = text.split_whitespace().nth(1).and_then(|code| code.parse().ok()).unwrap_or(0);
        (status, text.split("\r\n\r\n").nth(1).unwrap_or_default().to_string())
    }

    #[test]
    fn serves_the_engine_with_its_token_and_takes_uploads() {
        let root = std::env::temp_dir().join(format!("crispmesh-share-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("runs")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let engine = Arc::new(Native::default());
        engine.configure(&root.join("runs"), &root.join("data")).unwrap();
        let sharing = Sharing::start(engine, 0, "secret-token".into()).unwrap();
        let port = sharing.port;
        let auth = "Authorization: Bearer secret-token\r\n";

        // No token, a wrong one: refused. Preflight: allowed without one.
        assert_eq!(call(port, b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n").0, 401);
        assert_eq!(call(port, b"GET /api/health HTTP/1.1\r\nAuthorization: Bearer secret-tokeN\r\n\r\n").0, 401);
        assert_eq!(call(port, b"OPTIONS /api/uploads HTTP/1.1\r\n\r\n").0, 204);

        let (status, body) = call(port, format!("GET /api/health HTTP/1.1\r\n{auth}\r\n").as_bytes());
        assert_eq!(status, 200);
        let health: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(health["uploads"], true);
        assert!(health["start_points"].as_array().is_some_and(|points| !points.is_empty()));

        // A photo goes into the data folder; a path that leaves it does not.
        let photo = b"\x89PNG not really";
        let upload = |folder: &str, name: &str| {
            let mut raw = format!("POST /api/uploads?folder={folder}&name={name} HTTP/1.1\r\n{auth}Content-Length: {}\r\n\r\n", photo.len()).into_bytes();
            raw.extend_from_slice(photo);
            call(port, &raw)
        };
        let (status, body) = upload("capture-1", "capture_0001.png");
        assert_eq!(status, 201, "{body}");
        assert_eq!(std::fs::read(root.join("data/capture-1/capture_0001.png")).unwrap(), photo);
        assert_eq!(upload("..%2Foutside", "x.png").0, 400);
        assert_eq!(upload("capture-1", "..%2Fx.png").0, 400);

        // Large bodies both ways (as an STL of a million triangles): nothing is cut short.
        let big: Vec<u8> = (0..20_000_000u32).map(|i| (i % 251) as u8).collect();
        let mut raw = format!("POST /api/uploads?folder=big&name=big.bin HTTP/1.1\r\n{auth}Content-Length: {}\r\n\r\n", big.len()).into_bytes();
        raw.extend_from_slice(&big);
        assert_eq!(call(port, &raw).0, 201);
        assert_eq!(std::fs::read(root.join("data/big/big.bin")).unwrap(), big);
        std::fs::create_dir_all(root.join("runs/20261016-120000-big")).unwrap();
        std::fs::write(root.join("runs/20261016-120000-big/events.jsonl"), "").unwrap();
        std::fs::write(root.join("runs/20261016-120000-big/mesh.stl"), &big).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(format!("GET /api/runs/20261016-120000-big/files/mesh.stl HTTP/1.1\r\n{auth}\r\n").as_bytes()).unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        let start = answer.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(String::from_utf8_lossy(&answer[..start]).starts_with("HTTP/1.1 200"), "{}", String::from_utf8_lossy(&answer[..start]));
        assert_eq!(answer.len() - start, big.len());
        assert!(answer[start..] == big[..]);

        let (status, body) = call(port, format!("GET /api/data?path= HTTP/1.1\r\n{auth}\r\n").as_bytes());
        assert_eq!(status, 200);
        assert!(body.contains("capture-1"), "{body}");
        assert_eq!(call(port, format!("GET /api/runs HTTP/1.1\r\n{auth}\r\n").as_bytes()).0, 200);
        assert_eq!(call(port, format!("GET /api/runs/nope/events?since=0 HTTP/1.1\r\n{auth}\r\n").as_bytes()).0, 404);
        assert_eq!(call(port, format!("POST /api/runs HTTP/1.1\r\n{auth}Content-Length: 3\r\n\r\n{{}}x").as_bytes()).0, 400);
        assert_eq!(call(port, format!("GET /elsewhere HTTP/1.1\r\n{auth}\r\n").as_bytes()).0, 404);
        sharing.stop();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// For checks from outside (scripts/handoff-check.mjs): serves an engine on folders under CRISPMESH_SHARE_ROOT
    /// at port 8766 with the token in CRISPMESH_SHARE_TOKEN until a file `stop` appears there.
    /// `cargo test --release serve_for_checks -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn serve_for_checks() {
        let root = std::path::PathBuf::from(std::env::var("CRISPMESH_SHARE_ROOT").expect("CRISPMESH_SHARE_ROOT"));
        let token = std::env::var("CRISPMESH_SHARE_TOKEN").expect("CRISPMESH_SHARE_TOKEN");
        for folder in ["runs", "data"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let engine = Arc::new(Native::default());
        engine.configure(&root.join("runs"), &root.join("data")).unwrap();
        let none = || crate::config::Value { value: String::new(), source: crate::config::Source::Default };
        let tools = crate::config::Tools {
            alicevision: none(),
            alicevision_library_path: none(),
            colmap: none(),
            sam_python: none(),
            sam_source: none(),
            sam_checkpoint: none(),
        };
        engine.configure_tools(tools, String::new());
        let sharing = Sharing::start(engine, 8766, token).unwrap();
        println!("serving on port {}", sharing.port);
        while !root.join("stop").exists() {
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    #[test]
    fn decodes_what_encode_uri_component_writes() {
        assert_eq!(decode("capture%20one%2Fa").as_deref(), Some("capture one/a"));
        assert_eq!(decode("%E2%82%AC").as_deref(), Some("€"));
        assert_eq!(decode("%zz"), None);
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
