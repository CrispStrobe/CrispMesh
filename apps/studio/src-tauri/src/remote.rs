//! Requests to an engine on another computer at a plain `http://` address, made here instead of in the web
//! view. The app's page has its own (secure) origin; a web view may block or limit plain-HTTP requests from it
//! (mixed content, CORS, local-network rules). From the native side they are ordinary connections. HTTP/1.1,
//! one request per connection, no TLS (`https://` addresses stay with the web view).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Largest answer read: a mesh of a few million triangles.
const MAX_ANSWER: usize = 512 * 1024 * 1024;

/// `http://host:port/path?query` -> (host, port, path with query).
pub fn split_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("http://").ok_or("only http:// addresses go through the app")?;
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err("not a plain host and port".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.ends_with(']') || authority.starts_with('[') => {
            (host.to_string(), port.parse::<u16>().map_err(|_| "bad port".to_string())?)
        }
        _ => (authority.to_string(), 80),
    };
    Ok((host.trim_start_matches('[').trim_end_matches(']').to_string(), port, path.to_string()))
}

/// Sends one request; answers (status, headers, body).
pub fn fetch(method: &str, url: &str, headers: &[(String, String)], body: &[u8]) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    if !matches!(method, "GET" | "HEAD" | "POST" | "PUT" | "DELETE") {
        return Err(format!("method {method} is not sent"));
    }
    let (host, port, path) = split_url(url)?;
    let address = (host.as_str(), port).to_socket_addrs().map_err(|error| error.to_string())?.next().ok_or("no address for that host")?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10)).map_err(|error| format!("could not connect: {error}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(300)));
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        // Only what a request may carry; the connection's own headers are set here.
        let lower = name.to_ascii_lowercase();
        if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) || matches!(lower.as_str(), "host" | "connection" | "content-length" | "transfer-encoding") {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).map_err(|error| error.to_string())?;
    stream.write_all(body).map_err(|error| error.to_string())?;
    let mut answer = Vec::new();
    stream.take(MAX_ANSWER as u64 + 65536).read_to_end(&mut answer).map_err(|error| error.to_string())?;
    let end = answer.windows(4).position(|window| window == b"\r\n\r\n").ok_or("no complete answer")?;
    let text = String::from_utf8_lossy(&answer[..end]).to_string();
    let mut lines = text.split("\r\n");
    let status = lines.next().and_then(|line| line.split_whitespace().nth(1)).and_then(|code| code.parse().ok()).ok_or("no status")?;
    let headers: Vec<(String, String)> =
        lines.filter_map(|line| line.split_once(':')).map(|(name, value)| (name.trim().to_string(), value.trim().to_string())).collect();
    let mut body = answer[end + 4..].to_vec();
    let chunked = headers.iter().any(|(name, value)| name.eq_ignore_ascii_case("transfer-encoding") && value.eq_ignore_ascii_case("chunked"));
    if chunked {
        body = unchunk(&body)?;
    } else if let Some(length) = headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-length")).and_then(|(_, value)| value.parse::<usize>().ok()) {
        if method != "HEAD" && body.len() < length {
            return Err(format!("the answer was cut short ({} of {length} bytes)", body.len()));
        }
        body.truncate(length);
    }
    if method == "HEAD" {
        body.clear();
    }
    Ok((status, headers, body))
}

fn unchunk(data: &[u8]) -> Result<Vec<u8>, String> {
    let (mut out, mut at) = (Vec::new(), 0);
    loop {
        let line_end = data[at..].windows(2).position(|w| w == b"\r\n").ok_or("bad chunk")? + at;
        let size_text = String::from_utf8_lossy(&data[at..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "bad chunk size")?;
        at = line_end + 2;
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(data.get(at..at + size).ok_or("chunk cut short")?);
        at += size + 2;
    }
}

/// The answer as one byte string for the web view: status (2 bytes), length of the header JSON (4 bytes), the
/// header JSON (an object of lower-case names), the body.
pub fn frame(status: u16, headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let map: serde_json::Map<String, serde_json::Value> =
        headers.iter().map(|(name, value)| (name.to_ascii_lowercase(), serde_json::Value::String(value.clone()))).collect();
    let json = serde_json::Value::Object(map).to_string();
    let mut out = Vec::with_capacity(6 + json.len() + body.len());
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(json.as_bytes());
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn splits_addresses() {
        assert_eq!(split_url("http://192.168.1.20:8766/api/health").unwrap(), ("192.168.1.20".into(), 8766, "/api/health".into()));
        assert_eq!(split_url("http://host").unwrap(), ("host".into(), 80, "/".into()));
        assert_eq!(split_url("http://[::1]:8766/x?y=1").unwrap(), ("::1".into(), 8766, "/x?y=1".into()));
        assert!(split_url("https://host/").is_err());
        assert!(split_url("http://user:pass@host/").is_err());
    }

    #[test]
    fn fetches_from_a_server_and_frames_the_answer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
        let served = big.clone();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                // The whole request, body included, before answering (else the connection is reset).
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !(request.windows(4).any(|w| w == b"\r\n\r\n") && (!request.starts_with(b"POST") || request.ends_with(b"abc"))) {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                }
                let text = String::from_utf8_lossy(&request).to_string();
                let body: &[u8] = if text.starts_with("POST") { b"5\r\nhello\r\n0\r\n\r\n" } else { &served };
                let head = if text.starts_with("POST") {
                    assert!(text.contains("Authorization: Bearer t0k3n"), "{text}");
                    "HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n".to_string()
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Type: model/stl\r\nContent-Length: {}\r\n\r\n", served.len())
                };
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let (status, headers, body) = fetch("GET", &format!("http://127.0.0.1:{port}/mesh.stl"), &[], &[]).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, big);
        assert!(headers.iter().any(|(name, value)| name == "Content-Type" && value == "model/stl"));
        let auth = [("Authorization".to_string(), "Bearer t0k3n".to_string()), ("Evil\r\nX".to_string(), "y".to_string())];
        let (status, headers, body) = fetch("POST", &format!("http://127.0.0.1:{port}/api/uploads"), &auth, b"abc").unwrap();
        assert_eq!((status, body.as_slice()), (201, &b"hello"[..]));
        let framed = frame(status, &headers, &body);
        assert_eq!(u16::from_be_bytes([framed[0], framed[1]]), 201);
        let length = u32::from_be_bytes([framed[2], framed[3], framed[4], framed[5]]) as usize;
        let json: serde_json::Value = serde_json::from_slice(&framed[6..6 + length]).unwrap();
        assert_eq!(json["transfer-encoding"], "chunked");
        assert_eq!(&framed[6 + length..], b"hello");
        assert!(fetch("CONNECT", "http://127.0.0.1:1/", &[], &[]).is_err());
    }
}
