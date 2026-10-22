#![cfg(unix)]
use router_contract::cli_state::ProcessRecord;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
                "codeasier-cli-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn get(address: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream,"GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer model-canary\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn headless_serve_publishes_own_identity_and_cleans_after_sigterm() {
    let dir = Temp::new();
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = probe.local_addr().unwrap().to_string();
    drop(probe);
    let child = Command::new(env!("CARGO_BIN_EXE_codeasier-router"))
        .args(["serve", "--data-dir"])
        .arg(&dir.0)
        .args(["--listen", &address])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = Running(child);
    let mut line = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["state"], "listening");
    let record: ProcessRecord =
        serde_json::from_slice(&std::fs::read(dir.0.join("router-state.json")).unwrap()).unwrap();
    let live = router_contract::process::inspect(child.0.id() as i32).unwrap();
    assert_eq!(record.pid, live.pid);
    assert_eq!(record.process_started_at, live.started_at);
    assert_eq!(record.process_executable, live.executable);
    assert_eq!(record.upstream_auth, "per_user_mtls");
    assert!(get(&address, "/version").contains("per_user_mtls"));
    assert!(get(&address, "/health").starts_with("HTTP/1.1 200"));
    let response = get(&address, "/v1/models");
    assert!(response.contains("CLIENT_CERT_REQUIRED"));
    assert!(!response.contains("model-canary"));
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) }, 0);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "SIGTERM shutdown exceeded its bound"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!dir.0.join("router-state.json").exists());
    let mut logs = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut logs)
        .unwrap();
    assert!(!logs.contains("model-canary"));
    assert!(!logs.contains("PRIVATE KEY"));
}

#[test]
fn occupied_port_is_not_stolen_and_no_process_record_is_published() {
    let dir = Temp::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_codeasier-router"))
        .args(["serve", "--data-dir"])
        .arg(&dir.0)
        .args(["--listen", &address])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("CLI_LISTEN_FAILED"));
    assert!(!dir.0.join("router-state.json").exists());
    assert!(TcpStream::connect(address).is_ok());
}

#[test]
fn version_needs_no_user_identity_and_unknown_secret_flags_are_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_codeasier-router"))
        .arg("--version")
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(metadata["upstream_auth"], "per_user_mtls");
    let output = Command::new(env!("CARGO_BIN_EXE_codeasier-router"))
        .args(["login", "--api-key", "secret-argument-canary"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-argument-canary"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-argument-canary"));
}
