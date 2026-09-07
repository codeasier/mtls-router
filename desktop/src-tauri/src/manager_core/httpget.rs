#![allow(dead_code)]

//! Bounded HTTP/1.1 GET for loopback catalog and usage fetches.
//!
//! No proxy, no redirects, no TLS, no logging. Errors carry no URL, status
//! text, headers, or body.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub connection_close: bool,
}

pub trait HttpTransport {
    fn get(
        &mut self,
        path: &str,
        authorization: Option<&str>,
        deadline: Instant,
        max_body: usize,
    ) -> Result<HttpResponse, ()>;
}

pub struct DirectTransport {
    pub authority: String,
}

impl HttpTransport for DirectTransport {
    fn get(
        &mut self,
        path: &str,
        authorization: Option<&str>,
        deadline: Instant,
        max_body: usize,
    ) -> Result<HttpResponse, ()> {
        let mut stream = connect(&self.authority, deadline)?;
        request(
            &mut stream,
            &self.authority,
            path,
            authorization,
            deadline,
            max_body,
        )
    }
}

pub fn remaining(deadline: Instant) -> Result<Duration, ()> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| *value > Duration::ZERO)
        .ok_or(())
}

pub fn connect(authority: &str, deadline: Instant) -> Result<TcpStream, ()> {
    let timeout = remaining(deadline)?;
    let addr = authority
        .to_socket_addrs()
        .map_err(|_| ())?
        .next()
        .ok_or(())?;
    let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|_| ())?;
    apply_deadline(&stream, deadline)?;
    stream.set_nodelay(true).map_err(|_| ())?;
    Ok(stream)
}

pub fn request(
    stream: &mut TcpStream,
    host: &str,
    path: &str,
    authorization: Option<&str>,
    deadline: Instant,
    max_body: usize,
) -> Result<HttpResponse, ()> {
    if !valid_token(host) || !valid_path(path) {
        return Err(());
    }
    if let Some(value) = authorization {
        if value.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
            return Err(());
        }
    }
    apply_deadline(stream, deadline)?;
    let mut message = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n");
    if let Some(value) = authorization {
        message.push_str("Authorization: ");
        message.push_str(value);
        message.push_str("\r\n");
    }
    message.push_str("Connection: keep-alive\r\n\r\n");
    stream.write_all(message.as_bytes()).map_err(|_| ())?;
    stream.flush().map_err(|_| ())?;
    read_response(stream, deadline, max_body)
}

/// Rejects anything except an exact `http://host/path` with no userinfo,
/// query, or fragment. Callers append a query only after this check.
pub fn parse_exact_http_url(raw: &str, expected_path: &str) -> Result<String, ()> {
    if raw.contains('#') || raw.contains('?') {
        return Err(());
    }
    let uri: http::Uri = raw.parse().map_err(|_| ())?;
    if uri.scheme_str() != Some("http") {
        return Err(());
    }
    let authority = uri.authority().ok_or(())?;
    if authority.as_str().contains('@') || authority.host().is_empty() {
        return Err(());
    }
    if uri.path() != expected_path || uri.query().is_some() {
        return Err(());
    }
    Ok(authority.to_string())
}

fn apply_deadline(stream: &TcpStream, deadline: Instant) -> Result<(), ()> {
    let timeout = remaining(deadline)?;
    stream.set_read_timeout(Some(timeout)).map_err(|_| ())?;
    stream.set_write_timeout(Some(timeout)).map_err(|_| ())?;
    Ok(())
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && !value
            .bytes()
            .any(|byte| byte == b' ' || byte == b'\r' || byte == b'\n')
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path
            .bytes()
            .any(|byte| matches!(byte, b' ' | b'\r' | b'\n'))
}

fn read_response(
    stream: &mut TcpStream,
    deadline: Instant,
    max_body: usize,
) -> Result<HttpResponse, ()> {
    let header_bytes = read_until_headers(stream, deadline)?;
    let (status, connection_close, content_length, chunked) = parse_headers(&header_bytes)?;
    let body = if chunked {
        read_chunked(stream, deadline, max_body)?
    } else if let Some(length) = content_length {
        if length > max_body {
            return Ok(HttpResponse {
                status,
                body: vec![0; max_body],
                connection_close,
            });
        }
        read_exact(stream, deadline, length).map_err(|_| ())?
    } else if status == 204 || status == 304 {
        Vec::new()
    } else {
        return Err(());
    };
    if body.len() > max_body {
        return Err(());
    }
    Ok(HttpResponse {
        status,
        body,
        connection_close,
    })
}

fn read_until_headers(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, ()> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while buffer.len() < MAX_HEADER_BYTES {
        apply_deadline(stream, deadline)?;
        if stream.read(&mut byte).map_err(|_| ())? != 1 {
            return Err(());
        }
        buffer.push(byte[0]);
        if buffer.ends_with(b"\r\n\r\n") {
            return Ok(buffer);
        }
    }
    Err(())
}

fn parse_headers(bytes: &[u8]) -> Result<(u16, bool, Option<usize>, bool), ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or(())?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts.next().ok_or(())?;
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(());
    }
    let status: u16 = status_parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let mut connection_close = version == "HTTP/1.0";
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(())?;
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("Connection") {
            connection_close = value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("close"));
        } else if name.eq_ignore_ascii_case("Content-Length") {
            if content_length.is_some() || chunked {
                return Err(());
            }
            content_length = Some(value.parse().map_err(|_| ())?);
        } else if name.eq_ignore_ascii_case("Transfer-Encoding") {
            if value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
            {
                if content_length.is_some() {
                    return Err(());
                }
                chunked = true;
            }
        }
    }
    Ok((status, connection_close, content_length, chunked))
}

fn read_exact(
    stream: &mut TcpStream,
    deadline: Instant,
    length: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let mut body = vec![0u8; length];
    let mut offset = 0;
    while offset < length {
        apply_deadline(stream, deadline)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline"))?;
        let read = stream.read(&mut body[offset..])?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("eof at {offset}/{length}"),
            ));
        }
        offset += read;
    }
    Ok(body)
}

fn read_chunked(stream: &mut TcpStream, deadline: Instant, max_body: usize) -> Result<Vec<u8>, ()> {
    let mut body = Vec::new();
    loop {
        let line = read_line(stream, deadline)?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| ())?;
        if size == 0 {
            let _ = read_line(stream, deadline)?;
            return Ok(body);
        }
        if body.len().saturating_add(size) > max_body {
            return Err(());
        }
        body.extend(read_exact(stream, deadline, size).map_err(|_| ())?);
        let trailer = read_line(stream, deadline)?;
        if !trailer.is_empty() {
            return Err(());
        }
    }
}

fn read_line(stream: &mut TcpStream, deadline: Instant) -> Result<String, ()> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while buffer.len() < 1024 {
        apply_deadline(stream, deadline)?;
        if stream.read(&mut byte).map_err(|_| ())? != 1 {
            return Err(());
        }
        buffer.push(byte[0]);
        if buffer.ends_with(b"\r\n") {
            buffer.truncate(buffer.len() - 2);
            return String::from_utf8(buffer).map_err(|_| ());
        }
    }
    Err(())
}

#[cfg(test)]
pub(crate) struct TestRequest {
    pub path: String,
    pub query: String,
    pub authorization: String,
    pub connection_id: u64,
}

#[cfg(test)]
pub(crate) struct TestResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub connection_close: bool,
}

#[cfg(test)]
impl TestResponse {
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            connection_close: false,
        }
    }
}

#[cfg(test)]
pub(crate) struct TestServer {
    addr: SocketAddr,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[cfg(test)]
impl TestServer {
    pub fn start(handler: impl Fn(&TestRequest) -> TestResponse + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let thread = thread::spawn(move || {
            let handler = Arc::new(handler);
            let next_id = AtomicU64::new(1);
            while flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if !flag.load(Ordering::SeqCst) {
                            break;
                        }
                        let _ = stream.set_nonblocking(false);
                        let id = next_id.fetch_add(1, Ordering::SeqCst);
                        let handler = handler.clone();
                        thread::spawn(move || serve_connection(stream, id, handler.as_ref()));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            running,
            thread: Some(thread),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn authority(&self) -> String {
        self.addr.to_string()
    }
}

#[cfg(test)]
impl Drop for TestServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(50));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
fn serve_connection(
    mut stream: TcpStream,
    connection_id: u64,
    handler: &dyn Fn(&TestRequest) -> TestResponse,
) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    loop {
        let request = match read_test_request(&mut stream) {
            Ok(mut request) => {
                request.connection_id = connection_id;
                request
            }
            Err(_) => break,
        };
        let response = handler(&request);
        if write_test_response(&mut stream, &response).is_err() {
            break;
        }
        if response.connection_close {
            let _ = stream.shutdown(Shutdown::Both);
            break;
        }
    }
}

#[cfg(test)]
fn read_test_request(stream: &mut TcpStream) -> Result<TestRequest, ()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let headers = read_until_headers(stream, deadline)?;
    let text = std::str::from_utf8(&headers).map_err(|_| ())?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(())?;
    let target = parts.next().ok_or(())?;
    if method != "GET" {
        return Err(());
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target.to_owned(), String::new()),
    };
    let mut authorization = String::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Authorization") {
                authorization = value.trim().to_owned();
            }
        }
    }
    Ok(TestRequest {
        path,
        query,
        authorization,
        connection_id: 0,
    })
}

#[cfg(test)]
fn write_test_response(stream: &mut TcpStream, response: &TestResponse) -> Result<(), ()> {
    let connection = if response.connection_close {
        "close"
    } else {
        "keep-alive"
    };
    let header = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: {}\r\n\r\n",
        response.status,
        response.body.len(),
        connection
    );
    stream.write_all(header.as_bytes()).map_err(|_| ())?;
    stream.write_all(&response.body).map_err(|_| ())?;
    stream.flush().map_err(|_| ())
}
