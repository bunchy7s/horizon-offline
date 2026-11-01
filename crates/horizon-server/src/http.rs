//! Just enough HTTP/1.1 for the control API on 127.0.0.1 and the web panel (docs/server.md, sections 6 and
//! 8): one request per connection, bodies with a Content-Length. No async runtime and no dependency: a
//! thread per connection is plenty for a CLI, a panel tab and an MCP client.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    time::Duration,
};

/// The largest request body the API takes (a console command, a settings change).
const MAX_BODY: usize = 1 << 20;
/// The longest request line or header line taken.
const MAX_LINE: usize = 8 << 10;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    /// One `key=value` from the query string (no percent-decoding: the API's values are numbers and
    /// tokens).
    pub fn param(&self, key: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }
    /// One cookie the browser sent.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .flat_map(|(_, v)| v.split(';'))
            .filter_map(|c| c.trim().split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v)
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
    /// Writes the body itself instead of `body`, after the head (a file, a stream of events); the
    /// connection's end ends the body unless it sets a Content-Length.
    pub stream: Option<Box<dyn FnOnce(&TcpStream) + Send>>,
}

impl Response {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self::text(status, "application/json", value.to_string())
    }
    pub fn error(status: u16, message: &str) -> Self {
        Self::json(status, &serde_json::json!({ "error": message }))
    }
    pub fn text(status: u16, content_type: &'static str, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type,
            headers: Vec::new(),
            body: body.into(),
            stream: None,
        }
    }
    /// A file sent as it is read, for a download (`name` is what the browser saves it as).
    pub fn file(path: std::path::PathBuf, name: &str) -> Self {
        let Ok(length) = std::fs::metadata(&path).map(|m| m.len()) else {
            return Self::error(404, "no such file");
        };
        let mut response = Self::text(200, "application/octet-stream", "")
            .with("Content-Length", length.to_string())
            .with(
                "Content-Disposition",
                format!("attachment; filename=\"{name}\""),
            );
        response.stream = Some(Box::new(move |mut stream: &TcpStream| {
            if let Ok(mut file) = std::fs::File::open(&path) {
                let _ = std::io::copy(&mut file, &mut stream);
            }
        }));
        response
    }
    pub fn with(mut self, header: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((header, value.into()));
        self
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    }
}

/// `read_line` under a limit: more than MAX_LINE without a '\n' is an error
/// (InvalidInput), not a line that grows for as long as the client streams.
fn read_line_bounded(reader: &mut impl BufRead, line: &mut String) -> std::io::Result<usize> {
    let n = reader.by_ref().take(MAX_LINE as u64 + 1).read_line(line)?;
    if n > MAX_LINE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "line too long",
        ));
    }
    Ok(n)
}

/// Reads one request from a connection, plain or TLS (the management protocol's `websocket::Link`).
pub fn read_request(stream: impl Read) -> Result<Request, Response> {
    fn bad(m: &str) -> Response {
        Response::error(400, m)
    }
    fn read_line(
        reader: &mut impl BufRead,
        line: &mut String,
        what: &str,
    ) -> Result<usize, Response> {
        read_line_bounded(reader, line).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidInput {
                bad("line too long")
            } else {
                bad(what)
            }
        })
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    read_line(&mut reader, &mut line, "no request")?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(bad("malformed request line"));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if read_line(&mut reader, &mut line, "headers")? == 0 || line.trim().is_empty() {
            break;
        }
        if headers.len() > 64 {
            return Err(bad("too many headers"));
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map_or(Ok(0), |(_, v)| v.parse::<usize>())
        .map_err(|_| bad("bad Content-Length"))?;
    if length > MAX_BODY {
        return Err(Response::error(413, "the body is too large"));
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| bad("the body is shorter than its Content-Length"))?;
    Ok(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        headers,
        body,
    })
}

fn write_response(mut stream: &TcpStream, mut response: Response) {
    let _ = write_head(stream, &response);
    if let Some(body) = response.stream.take() {
        body(stream);
        let _ = stream.flush();
    }
}

/// The status line, the headers and `body`: all of a response but its `stream`.
pub fn write_head(mut out: impl Write, response: &Response) -> std::io::Result<()> {
    // A WebSocket upgrade: only the headers the handshake needs, then the socket is the stream's.
    let mut head = if response.status == 101 {
        "HTTP/1.1 101 Switching Protocols\r\n".to_owned()
    } else {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
            response.status,
            reason(response.status),
            response.content_type,
        )
    };
    if response.stream.is_none() && response.status != 101 {
        head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    }
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(response.body.as_bytes())?;
    out.flush()
}

/// Serves `handle` on `listener` until the process ends, a thread per connection.
pub fn serve(
    listener: TcpListener,
    handle: impl Fn(&Request) -> Response + Send + Sync + 'static,
) -> std::io::Result<()> {
    let handle = std::sync::Arc::new(handle);
    std::thread::Builder::new()
        .name("api".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let handle = handle.clone();
                let _ = std::thread::Builder::new()
                    .name("api-client".into())
                    .spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                        let response = match read_request(&stream) {
                            Ok(request) => handle(&request),
                            Err(response) => response,
                        };
                        write_response(&stream, response);
                    });
            }
        })?;
    Ok(())
}

/// One request to `address`, with a bearer token; the status and the body.
pub fn request(
    address: SocketAddr,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
    timeout: Duration,
) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(timeout))?;
    let body = body.map(ToString::to_string).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    let mut text = String::new();
    stream.read_to_string(&mut text)?;
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::other("not an HTTP response"))?;
    Ok((status, body.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        serve(listener, |r| {
            Response::json(
                200,
                &serde_json::json!({
                    "method": r.method,
                    "path": r.path,
                    "since": r.param("since"),
                    "auth": r.header("authorization"),
                    "body": String::from_utf8_lossy(&r.body),
                }),
            )
        })
        .unwrap();
        let (status, body) = request(
            address,
            "t0k",
            "POST",
            "/api/v1/command?since=7",
            Some(&serde_json::json!({ "line": "list" })),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["method"], "POST");
        assert_eq!(v["path"], "/api/v1/command");
        assert_eq!(v["since"], "7");
        assert_eq!(v["auth"], "Bearer t0k");
        assert_eq!(v["body"], r#"{"line":"list"}"#);
    }

    #[test]
    fn a_line_past_the_limit_is_a_400() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        serve(listener, |_| Response::text(200, "text/plain", "ok")).unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // A request line that never ends, sent at once so the refused reply
        // (and the close) can't reset the write side first.
        let mut flood = b"GET /".to_vec();
        flood.extend_from_slice(&[b'a'; 100_000]);
        let _ = stream.write_all(&flood);
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut text = String::new();
        let _ = stream.read_to_string(&mut text);
        assert!(text.starts_with("HTTP/1.1 400"), "{text}");
    }
}
