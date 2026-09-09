use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::manager_core::process::{Identity, Status};

use super::channel::{BoundSession, ChannelHooks, TrustTarget};
use super::error::SafeKind;

#[test]
fn conformance_vectors_never_send_authorization_on_failure() {
    let raw = include_str!(
        "../../../../internal/manager/trustedrouter/testdata/workbench-conformance.json"
    );
    let file: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert!(raw.contains("HTTP_PROXY"));
    let key = file["key_canary"].as_str().unwrap();
    for case in file["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        if case["kind"].as_str() == Some("invariant") {
            continue;
        }
        if case["listen"].as_str() != Some("loopback") {
            let target = TrustTarget {
                authority: case["listen"]
                    .as_str()
                    .unwrap_or("10.66.0.2:19099")
                    .to_owned(),
                pid: 91,
                deployment_id: "prod-a".into(),
                protocol_version: "4".into(),
                started_at: "start".into(),
                executable: "/router".into(),
                binary_path: "/router".into(),
            };
            let err = BoundSession::connect(
                &target,
                &ChannelHooks::default(),
                Instant::now() + Duration::from_millis(50),
                Arc::new(AtomicBool::new(false)),
            )
            .err()
            .expect(id);
            assert_eq!(err.kind, SafeKind::Identity, "{id}");
            continue;
        }
        let seen_auth = Arc::new(AtomicBool::new(false));
        let hang = case["hang"].as_str().unwrap_or("").to_owned();
        let version_status = case["version_status"].as_u64().unwrap_or(200) as u16;
        let connection = case["connection"]
            .as_str()
            .unwrap_or("keep-alive")
            .to_owned();
        let version = case.get("version").cloned();
        let process = case["process"].as_str().unwrap_or("genuine").to_owned();
        let server = TestHttp::start({
            let seen_auth = seen_auth.clone();
            let hang = hang.clone();
            move |req| {
                if !req.authorization.is_empty() {
                    seen_auth.store(true, Ordering::SeqCst);
                }
                if req.path == "/version" {
                    if hang == "version" {
                        thread::sleep(Duration::from_millis(80));
                    }
                    return version_response(version_status, &connection, version.as_ref());
                }
                if req.path == "/health" {
                    return (200, "keep-alive", br#"{"status":"ok"}"#.to_vec());
                }
                (
                    200,
                    "keep-alive",
                    br#"{"data":[{"id":"model-a"}]}"#.to_vec(),
                )
            }
        });
        let target = TrustTarget {
            authority: server.authority(),
            pid: 91,
            deployment_id: "prod-a".into(),
            protocol_version: "4".into(),
            started_at: "start".into(),
            executable: "/router".into(),
            binary_path: "/router".into(),
        };
        let validate = if process == "stale" {
            Some(Arc::new(|_identity: &Identity, _path: &str| Ok(Status::Stale)) as _)
        } else {
            Some(Arc::new(|_identity: &Identity, _path: &str| Ok(Status::Genuine)) as _)
        };
        let hooks = ChannelHooks {
            dial: None,
            validate,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        if case["id"].as_str() == Some("cancel_during_version") {
            cancel.store(true, Ordering::SeqCst);
        }
        let deadline = Instant::now()
            + if hang == "version" {
                Duration::from_millis(20)
            } else {
                Duration::from_secs(2)
            };
        let result =
            BoundSession::connect(&target, &hooks, deadline, cancel).and_then(|mut session| {
                session.get(
                    "/v1/models",
                    &format!("Bearer {key}"),
                    Instant::now() + Duration::from_secs(1),
                    1024,
                )
            });
        let expect_auth = case["expect_authorization"].as_bool().unwrap_or(false);
        if expect_auth {
            assert!(result.is_ok(), "{id} {result:?}");
            assert!(seen_auth.load(Ordering::SeqCst), "{id} missing auth");
        } else {
            assert!(result.is_err(), "{id} succeeded");
            assert!(
                !seen_auth.load(Ordering::SeqCst),
                "{id} leaked Authorization"
            );
        }
    }
}

fn version_response(
    status: u16,
    connection: &str,
    version: Option<&serde_json::Value>,
) -> (u16, &'static str, Vec<u8>) {
    if status == 302 {
        return (302, "keep-alive", Vec::new());
    }
    if status == 101 || connection == "upgrade" {
        return (101, "upgrade", Vec::new());
    }
    let body = if let Some(version) = version {
        serde_json::to_vec(&serde_json::json!({
            "pid": version["pid"],
            "deployment_id": version["deployment_id"],
            "management_protocol_version": version["management_protocol_version"]
        }))
        .unwrap()
    } else {
        br#"{"pid":91,"deployment_id":"prod-a","management_protocol_version":"4"}"#.to_vec()
    };
    let close = if connection == "close" {
        "close"
    } else {
        "keep-alive"
    };
    (200, close, body)
}

struct TestReq {
    path: String,
    authorization: String,
}

struct TestHttp {
    addr: std::net::SocketAddr,
    running: Arc<AtomicBool>,
}

impl TestHttp {
    fn start(
        handler: impl Fn(&TestReq) -> (u16, &'static str, Vec<u8>) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        thread::spawn(move || {
            let handler = Arc::new(handler);
            while flag.load(Ordering::SeqCst) {
                if let Ok((stream, _)) = listener.accept() {
                    let handler = handler.clone();
                    thread::spawn(move || serve(stream, handler.as_ref()));
                } else {
                    break;
                }
            }
        });
        Self { addr, running }
    }

    fn authority(&self) -> String {
        self.addr.to_string()
    }
}

impl Drop for TestHttp {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(20));
    }
}

fn serve(mut stream: TcpStream, handler: &dyn Fn(&TestReq) -> (u16, &'static str, Vec<u8>)) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    loop {
        let mut headers = Vec::new();
        let mut byte = [0u8; 1];
        while headers.len() < 64 * 1024 {
            if stream.read(&mut byte).ok() != Some(1) {
                return;
            }
            headers.push(byte[0]);
            if headers.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&headers);
        let mut lines = text.split("\r\n");
        let request_line = lines.next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let _method = parts.next();
        let target = parts.next().unwrap_or("/");
        let path = target.split('?').next().unwrap_or(target).to_owned();
        let mut authorization = String::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("Authorization") {
                    authorization = value.trim().to_owned();
                }
            }
        }
        let (status, connection, body) = handler(&TestReq {
            path,
            authorization,
        });
        if status == 302 {
            let _ = stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return;
        }
        if status == 101 {
            let _ = stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n");
            return;
        }
        let header = format!(
            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n",
            body.len()
        );
        if stream.write_all(header.as_bytes()).is_err() || stream.write_all(&body).is_err() {
            return;
        }
        if connection == "close" {
            return;
        }
    }
}

#[allow(dead_code)]
static COUNTER: AtomicU64 = AtomicU64::new(0);
