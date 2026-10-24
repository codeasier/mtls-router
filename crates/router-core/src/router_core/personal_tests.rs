use super::test_support::{
    boxed_bytes, boxed_channel, start_mtls_optional_handler_with_certs, PeerCertificate, TestCerts,
};
use super::*;
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::TokioIo;
use router_identity::{test_support::TestAuthority, IdentityStore};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, Notify},
};

use super::connect::{open_mtls_http1, ConnectError};

async fn request(address: &str, path: &str) -> Response<hyper::body::Incoming> {
    let tcp = TcpStream::connect(address).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender
        .send_request(
            Request::builder()
                .uri(path)
                .header("Host", "127.0.0.1")
                .header("Authorization", "Bearer model-canary")
                .body(Empty::<Bytes>::new())
                .unwrap(),
        )
        .await
        .unwrap()
}
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn personal_management_without_certificate_and_atomic_live_generations() {
    let authority = TestAuthority::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let dir =
        Temp(std::env::temp_dir().join(format!("router-personal-{}-{}", std::process::id(), now)));
    let store = IdentityStore::open(&dir.0, authority.trust.clone()).unwrap();
    let peers = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let captured = peers.clone();
    let release = Arc::new(Notify::new());
    let stream_release = release.clone();
    let certs = TestCerts {
        ca: authority.root.pem(),
        client_cert: String::new(),
        client_key: String::new(),
        server_cert: authority.server_certificate.clone(),
        server_key: authority.server_key.clone(),
    };
    let server = start_mtls_optional_handler_with_certs(certs, move |request| {
        let peers = captured.clone();
        let release = stream_release.clone();
        async move {
            if request.uri().path().starts_with("/v1/") {
                peers.lock().unwrap().push(
                    request
                        .extensions()
                        .get::<PeerCertificate>()
                        .unwrap()
                        .0
                        .clone(),
                );
                assert_eq!(
                    request.headers().get("Authorization").unwrap(),
                    "Bearer model-canary"
                );
            }
            if request.uri().path() == "/v1/stream" {
                let (tx, rx) = mpsc::channel(2);
                tokio::spawn(async move {
                    tx.send(Bytes::from_static(b"data: first\n\n"))
                        .await
                        .unwrap();
                    release.notified().await;
                    let _ = tx.send(Bytes::from_static(b"data: done\n\n")).await;
                });
                Some(
                    Response::builder()
                        .header("Content-Type", "text/event-stream")
                        .body(boxed_channel(rx))
                        .unwrap(),
                )
            } else {
                Some(Response::new(boxed_bytes("model-ok")))
            }
        }
    })
    .await;
    let supervisor = RouterSupervisor::spawn(SupervisorConfig {
        defaults: RouterDefaults {
            listen_addr: "127.0.0.1:0".into(),
            upstream_url: server.url.clone(),
            ..RouterDefaults::default()
        },
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials: TlsMaterials::from_identity(store.clone(), authority.root.pem()),
        identity: BuildInfo::default().with_personal_auth(),
        limits: SupervisorLimits::default(),
        logger: None,
    });
    let address = supervisor.start().await.unwrap();
    let missing = request(&address, "/v1/models").await;
    assert_eq!(missing.status(), 401);
    let body = missing.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("CLIENT_CERT_REQUIRED"));
    assert_eq!(request(&address, "/health").await.status(), 200);
    let version = request(&address, "/version")
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let version: serde_json::Value = serde_json::from_slice(&version).unwrap();
    assert_eq!(version["upstream_auth"], "per_user_mtls");
    assert_eq!(version["auth_protocol_version"], "1");
    assert!(peers.lock().unwrap().is_empty());
    let (gen, pending) = store.begin_login("linux_cli", "one", now).unwrap();
    let first = authority.issue_pending(&pending, now, 42);
    let first_der = parse_pem_certificates(first.certificate_pem.as_bytes()).unwrap()[0]
        .as_ref()
        .to_vec();
    store.activate_login(gen, first, now).unwrap();
    let mut stream = request(&address, "/v1/stream").await.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(1), stream.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(frame.into_data().unwrap(), "data: first\n\n");
    let (gen, pending) = store.begin_login("linux_cli", "two", now).unwrap();
    let second = authority.issue_pending(&pending, now, 43);
    let second_der = parse_pem_certificates(second.certificate_pem.as_bytes()).unwrap()[0]
        .as_ref()
        .to_vec();
    store.activate_login(gen, second, now).unwrap();
    assert_eq!(request(&address, "/v1/models").await.status(), 200);
    assert_eq!(*peers.lock().unwrap(), vec![first_der, second_der]);
    store.disable_local().unwrap();
    assert_eq!(request(&address, "/v1/models").await.status(), 401);
    assert_eq!(request(&address, "/health").await.status(), 200);
    release.notify_one();
    let rest = stream.collect().await.unwrap().to_bytes();
    assert_eq!(rest, "data: done\n\n");
    supervisor.shutdown().await.unwrap();
}

fn active_personal_store(authority: &TestAuthority, label: &str) -> (Temp, IdentityStore) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let dir = Temp(std::env::temp_dir().join(format!(
        "router-personal-{}-{label}-{now}",
        std::process::id()
    )));
    let store = IdentityStore::open(&dir.0, authority.trust.clone()).unwrap();
    let (generation, pending) = store.begin_login("linux_cli", label, now).unwrap();
    let issued = authority.issue_pending(&pending, now, 4242);
    store.activate_login(generation, issued, now).unwrap();
    (dir, store)
}

fn state_error(dir: &Path) -> Option<String> {
    let data = std::fs::read(dir.join("identity").join("state.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&data).ok()?;
    value
        .get("last_error")
        .and_then(|error| error.as_str())
        .map(ToOwned::to_owned)
}

async fn personal_transport_connect(
    server_url: &str,
    store: IdentityStore,
    upstream_ca_pem: String,
) -> Result<(), ConnectError> {
    let transport = MtlsTransport::new(
        &TlsMaterials::from_identity(store, upstream_ca_pem),
        TlsMinVersion::default(),
    )
    .unwrap();
    let url: http::Uri = server_url.parse().unwrap();
    open_mtls_http1::<Empty<Bytes>>(&transport, &url)
        .await
        .map(|_| ())
}

#[tokio::test]
async fn untrusted_server_certificate_is_transport_and_keeps_feedback_clean() {
    let server_authority = TestAuthority::new();
    let client_authority = TestAuthority::new();
    let server = start_mtls_optional_handler_with_certs(
        TestCerts {
            ca: server_authority.root.pem(),
            client_cert: String::new(),
            client_key: String::new(),
            server_cert: server_authority.server_certificate.clone(),
            server_key: server_authority.server_key.clone(),
        },
        |_request| async { Some(Response::new(boxed_bytes("unused"))) },
    )
    .await;
    let (dir, store) = active_personal_store(&client_authority, "untrusted");
    let result = personal_transport_connect(&server.url, store, client_authority.root.pem()).await;
    assert!(matches!(result, Err(ConnectError::Transport)));
    assert_eq!(state_error(&dir.0), None);
}

// Windows can reset the connection before the peer certificate alert is read,
// so the supervisor observes a transport failure there. The classifier unit
// tests still cover the alert mapping on every platform.
#[cfg(not(windows))]
#[tokio::test]
async fn request_time_client_certificate_rejection_is_forbidden_and_recorded() {
    let server_authority = TestAuthority::new();
    let client_authority = TestAuthority::new();
    let server = start_mtls_optional_handler_with_certs(
        TestCerts {
            ca: server_authority.root.pem(),
            client_cert: String::new(),
            client_key: String::new(),
            server_cert: server_authority.server_certificate.clone(),
            server_key: server_authority.server_key.clone(),
        },
        |_request| async { Some(Response::new(boxed_bytes("unused"))) },
    )
    .await;
    let (dir, store) = active_personal_store(&client_authority, "request-rejected");
    let supervisor = RouterSupervisor::spawn(SupervisorConfig {
        defaults: RouterDefaults {
            listen_addr: "127.0.0.1:0".into(),
            upstream_url: server.url.clone(),
            ..RouterDefaults::default()
        },
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials: TlsMaterials::from_identity(store, server_authority.root.pem()),
        identity: BuildInfo::default().with_personal_auth(),
        limits: SupervisorLimits::default(),
        logger: None,
    });
    let address = supervisor.start().await.unwrap();
    let rejected = request(&address, "/v1/models").await;
    assert_eq!(rejected.status(), 403);
    let body = rejected.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("CLIENT_CERT_REJECTED"));
    assert_eq!(request(&address, "/health").await.status(), 200);
    assert_eq!(state_error(&dir.0).as_deref(), Some("CLIENT_CERT_REJECTED"));
    supervisor.shutdown().await.unwrap();
}
