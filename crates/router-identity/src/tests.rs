use super::*;
use crate::{test_support::TestAuthority as Fixture, wire::IssuedCertificate};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::time::SystemTime;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use uuid::Uuid;

const NOW: i64 = 1_792_670_400;

#[tokio::test]
#[ignore = "driven by the real nginx Go integration harness"]
async fn external_https_enrollment() {
    let Some(root_file) = std::env::var_os("MTLS_TEST_NATIVE_CLIENT_ROOT") else {
        return;
    };
    let client_root = fs::read_to_string(root_file).unwrap();
    let server_root =
        fs::read_to_string(std::env::var_os("MTLS_TEST_NATIVE_SERVER_ROOT").unwrap()).unwrap();
    let enroll = std::env::var("MTLS_TEST_NATIVE_ENROLL").unwrap();
    let relay = std::env::var("MTLS_TEST_NATIVE_RELAY").unwrap();
    let dir = Temp::new();
    let store = IdentityStore::open(&dir.0, IdentityTrust::new(client_root).unwrap()).unwrap();
    let client = NativeClient::https_fixture(
        store.clone(),
        server_root.clone(),
        enroll.clone(),
        relay.clone(),
    );
    let started = client
        .connect("linux_cli", "actual Rust native client")
        .await
        .unwrap();
    let parsed = reqwest::Url::parse(&enroll).unwrap();
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], parsed.port().unwrap()));
    let roots = reqwest::Certificate::from_pem_bundle(server_root.as_bytes()).unwrap();
    let browser = reqwest::Client::builder()
        .tls_backend_rustls()
        .tls_certs_only(roots.clone())
        .no_proxy()
        .https_only(true)
        .resolve("enroll.codeasier.top", address)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = browser
        .get(started.verification_uri.replace(ENROLL_ORIGIN, &enroll))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let page = response.text().await.unwrap();
    fn csrf(page: &str) -> String {
        page.split("name=\"csrf\" value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .into()
    }
    let body = format!("transaction={}&csrf={}", started.transaction, csrf(&page));
    let response = browser
        .post(format!("{enroll}/enroll/native/authorize"))
        .header("Cookie", &cookie)
        .header("Origin", ENROLL_ORIGIN)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 303);
    let location = reqwest::Url::parse(
        response
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let state = location
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let response = browser
        .get(format!(
            "{enroll}/enroll/gitcode/callback?state={state}&code=rust-fixture-code"
        ))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let page = response.text().await.unwrap();
    let body = format!(
        "transaction={}&csrf={}&decision=approve",
        started.transaction,
        csrf(&page)
    );
    let response = browser
        .post(format!("{enroll}/enroll/native/approve"))
        .header("Cookie", &cookie)
        .header("Origin", ENROLL_ORIGIN)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let summary = client.poll().await.unwrap();
    assert_eq!(summary.phase, "connected");
    let active = store
        .snapshot(
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        )
        .unwrap()
        .unwrap()
        .1;
    let material = zeroize::Zeroizing::new(format!(
        "{}{}{}",
        active.certificate.certificate_pem,
        active.certificate.chain_pem,
        active.private_key_pem()
    ));
    let parsed = reqwest::Url::parse(&relay).unwrap();
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], parsed.port().unwrap()));
    let models = reqwest::Client::builder()
        .tls_backend_rustls()
        .tls_certs_only(roots)
        .identity(reqwest::Identity::from_pem(material.as_bytes()).unwrap())
        .no_proxy()
        .https_only(true)
        .resolve("relay.codeasier.top", address)
        .build()
        .unwrap();
    let response = models
        .post(format!("{relay}/v1/models"))
        .header("Authorization", "Bearer business-canary")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let summary = client.logout().await.unwrap();
    assert_eq!(summary.phase, "revocation_pending");
    assert!(store
        .snapshot(
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        )
        .unwrap()
        .is_none());
}

#[test]
fn shared_go_rust_protocol_fixture_is_consumed() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../testdata/router-mtls-v1/protocol.json"
    ))
    .unwrap();
    assert_eq!(fixture["version"], AUTH_PROTOCOL_VERSION);
    assert_eq!(fixture["origins"]["enroll"], ENROLL_ORIGIN);
    assert_eq!(fixture["origins"]["relay"], RELAY_ORIGIN);
    let input = fixture["native"]["digest_vector"]["input_utf8"]
        .as_str()
        .unwrap();
    assert_eq!(
        crate::crypto::digest(input.as_bytes()),
        fixture["native"]["digest_vector"]["sha256"]
    );
    assert_eq!(
        crate::crypto::challenge(input),
        fixture["native"]["digest_vector"]["s256"]
    );
    let dir = Temp::new();
    let authority = Fixture::new();
    let store = IdentityStore::open(&dir.0, authority.trust).unwrap();
    let (_, pending) = store.begin_login("linux_cli", "fixture", NOW).unwrap();
    let value = serde_json::to_value(pending.request()).unwrap();
    let keys: std::collections::BTreeSet<_> = value.as_object().unwrap().keys().cloned().collect();
    let expected: std::collections::BTreeSet<_> = fixture["native"]["start_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(keys, expected);
    for forbidden in fixture["native"]["start_forbidden_fields"]
        .as_array()
        .unwrap()
    {
        assert!(value.get(forbidden.as_str().unwrap()).is_none());
    }
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("router-identity-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn key_csr_and_bound_certificate_validation() {
    let fixture = Fixture::new();
    let key = DeviceKey::generate().unwrap();
    key.validate().unwrap();
    let device = "b5c2fb58-d5e8-4dc2-9b04-38209df797d0";
    let issued = fixture.issue(&key, device);
    fixture.trust.validate(&issued, &key, None, NOW).unwrap();
    let another = DeviceKey::generate().unwrap();
    assert_eq!(
        fixture.trust.validate(&issued, &another, None, NOW),
        Err(IdentityError::InvalidCertificate)
    );
    let mut mismatch = issued.clone();
    mismatch.identity.user_id = "other-user".into();
    assert_eq!(
        fixture.trust.validate(&mismatch, &key, None, NOW),
        Err(IdentityError::InvalidCertificate)
    );
    let other = Fixture::new();
    assert_eq!(
        other.trust.validate(&issued, &key, None, NOW),
        Err(IdentityError::InvalidCertificate)
    );
    assert_eq!(
        fixture
            .trust
            .validate(&issued, &key, None, issued.not_after),
        Err(IdentityError::InvalidCertificate)
    );
    assert!(!format!("{key:?}").contains("PRIVATE"));
}

#[test]
fn atomic_identity_reload_and_generation_guards_preserve_other_files() {
    let dir = Temp::new();
    let fixture = Fixture::new();
    let agent = dir.0.join("agent-preserved.json");
    fs::write(&agent, b"existing-agent-config").unwrap();
    let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    let (generation, pending) = store.begin_login("linux_cli", "device", NOW).unwrap();
    let issued = fixture.issue(&pending.key, &pending.device_id);
    store
        .activate_login(generation, issued.clone(), NOW)
        .unwrap();
    let reopened = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    let (active_generation, active) = reopened.snapshot(NOW).unwrap().unwrap();
    assert_eq!(active_generation, generation);
    assert_eq!(active.certificate.serial, "2a");
    let state_file = dir.0.join("identity/state.json");
    let before = fs::read(&state_file).unwrap();
    let result = reopened.change(|state| {
        state.active.as_mut().unwrap().key = DeviceKey::generate()?;
        Ok(())
    });
    assert_eq!(result, Err(IdentityError::InvalidCertificate));
    assert_eq!(fs::read(&state_file).unwrap(), before);
    let next_generation = reopened.disable_local().unwrap();
    assert!(next_generation > generation);
    assert!(reopened.snapshot(NOW).unwrap().is_none());
    assert_eq!(reopened.read().unwrap().revocations.len(), 1);
    assert_eq!(
        reopened.activate_login(generation, issued, NOW),
        Err(IdentityError::Conflict)
    );
    assert_eq!(fs::read(&agent).unwrap(), b"existing-agent-config");
    let stage = dir.0.join("identity/.identity-stage-leftover");
    fs::write(&stage, b"interrupted-write").unwrap();
    assert!(reopened.snapshot(NOW).unwrap().is_none());
    assert!(stage.exists());
    assert!(!format!("{:?} {:?}", reopened, reopened.read().unwrap()).contains("PRIVATE"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(dir.0.join("identity"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for file in [state_file, dir.0.join("identity/identity.lock")] {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[cfg(windows)]
    {
        assert!(router_contract::windows_security::private_permissions_ok(
            &state_file
        ));
    }
}

#[test]
fn corrupt_or_symlink_state_is_not_reset() {
    let dir = Temp::new();
    let fixture = Fixture::new();
    let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    store.begin_login("desktop", "device", NOW).unwrap();
    let file = dir.0.join("identity/state.json");
    fs::write(&file, b"{corrupt").unwrap();
    assert!(matches!(
        IdentityStore::open(&dir.0, fixture.trust.clone()),
        Err(IdentityError::InvalidState)
    ));
    assert_eq!(fs::read(&file).unwrap(), b"{corrupt");
    #[cfg(unix)]
    {
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(dir.0.join("elsewhere"), &file).unwrap();
        assert!(IdentityStore::open(&dir.0, fixture.trust).is_err());
        assert!(!dir.0.join("elsewhere").exists());
    }
}

#[test]
fn account_switch_rejects_old_activation_and_transport_feedback_without_touching_credentials() {
    let dir = Temp::new();
    let authority = Fixture::new();
    let store = IdentityStore::open(&dir.0, authority.trust.clone()).unwrap();
    let credential = dir.0.join("credentials.json");
    fs::write(&credential, b"external-file-marker").unwrap();
    let (old_gen, old) = store.begin_login("desktop", "old", NOW).unwrap();
    let old_certificate = authority.issue(&old.key, &old.device_id);
    let old_serial = old_certificate.serial.clone();
    store
        .activate_login(old_gen, old_certificate.clone(), NOW)
        .unwrap();
    let (new_gen, new) = store.begin_login("desktop", "new", NOW).unwrap();
    let second = authority.issue_for(&new.key, &new.device_id, "second-member", NOW, 43);
    store.activate_login(new_gen, second, NOW).unwrap();
    assert_eq!(
        store.activate_login(old_gen, old_certificate, NOW),
        Err(IdentityError::Conflict)
    );
    assert_eq!(
        store.certificate_feedback(old_gen, &old_serial, true),
        Err(IdentityError::Conflict)
    );
    let state = store.read().unwrap();
    assert_eq!(
        state.active.as_ref().unwrap().certificate.identity.user_id,
        "second-member"
    );
    assert!(state.last_error.is_none());
    assert_eq!(state.revocations.len(), 1);
    assert_eq!(fs::read(credential).unwrap(), b"external-file-marker");
}

#[test]
fn cross_process_writer() {
    let Some(dir) = std::env::var_os("IDENTITY_TEST_WRITER_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    let trust =
        IdentityTrust::new(fs::read_to_string(dir.join("public-root.pem")).unwrap()).unwrap();
    let store = IdentityStore::open(dir, trust).unwrap();
    for _ in 0..5 {
        store
            .begin_login("linux_cli", "parallel-process", NOW)
            .unwrap();
    }
}

#[test]
fn independent_processes_share_one_atomic_generation() {
    let dir = Temp::new();
    let fixture = Fixture::new();
    fs::write(dir.0.join("public-root.pem"), fixture.root.pem()).unwrap();
    let store = IdentityStore::open(&dir.0, fixture.trust).unwrap();
    let lock = crate::platform::Lock::acquire(&dir.0.join("identity/identity.lock")).unwrap();
    let mut children = Vec::new();
    for _ in 0..4 {
        children.push(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::cross_process_writer"])
                .env("IDENTITY_TEST_WRITER_DIR", &dir.0)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    for child in &mut children {
        assert!(child.try_wait().unwrap().is_none());
    }
    drop(lock);
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(store.read().unwrap().generation, 20);
}

struct FakeWire<F>(F);
impl<F> crate::client::WireBackend for FakeWire<F>
where
    F: Fn(crate::client::Route, serde_json::Value) -> Result<crate::client::Reply> + Send + Sync,
{
    fn post(
        &self,
        route: crate::client::Route,
        body: zeroize::Zeroizing<Vec<u8>>,
        _identity: Option<crate::store::ActiveIdentity>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<crate::client::Reply>> + Send + '_>,
    > {
        let value = serde_json::from_slice(&body).unwrap();
        let result = (self.0)(route, value);
        Box::pin(async move { result })
    }
}
fn reply(value: impl serde::Serialize) -> Result<crate::client::Reply> {
    Ok(crate::client::Reply {
        status: 200,
        body: serde_json::to_vec(&value).unwrap(),
    })
}
fn started(pending: &crate::PendingIdentity) -> wire::NativeStarted {
    let transaction = URL_SAFE_NO_PAD.encode([7u8; 32]);
    wire::NativeStarted {
        version: AUTH_PROTOCOL_VERSION.into(),
        verification_uri: format!("{ENROLL_ORIGIN}/enroll/native/verify?transaction={transaction}"),
        transaction,
        short_code: "ABCD-EFGH".into(),
        expires_in: 600,
        interval: 5,
        spki_digest: pending.key.spki_digest.clone(),
    }
}

#[tokio::test]
async fn native_client_is_key_private_and_rejects_late_login() {
    use crate::client::Route;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let dir = Temp::new();
    let fixture = Arc::new(Fixture::new());
    let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    let backend_store = store.clone();
    let backend_fixture = fixture.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = cancelled.clone();
    let backend = FakeWire(move |route, body: serde_json::Value| {
        assert!(body.get("private_key").is_none());
        assert!(body.get("api_key").is_none());
        match route {
            Route::Start => {
                assert!(body.get("verifier").is_none());
                reply(started(backend_store.read()?.pending.as_ref().unwrap()))
            }
            Route::Poll => {
                let state = backend_store.read()?;
                let pending = state.pending.as_ref().unwrap();
                let cert = backend_fixture.issue(&pending.key, &pending.device_id);
                if cancel.load(Ordering::SeqCst) {
                    backend_store.disable_local()?;
                }
                reply(serde_json::json!({"status":"issued","interval":5,"certificate":cert}))
            }
            Route::Revoke | Route::Cancel => Err(IdentityError::Unavailable),
            _ => Err(IdentityError::Conflict),
        }
    });
    let client = NativeClient::with_backend(store.clone(), Arc::new(backend), Arc::new(|| NOW));
    let authorization = client.connect("desktop", "device").await.unwrap();
    assert!(authorization.verification_uri.starts_with(ENROLL_ORIGIN));
    let private = store.read().unwrap().pending.unwrap();
    let summary = client.poll().await.unwrap();
    assert_eq!(summary.phase, "connected");
    let encoded = serde_json::to_string(&summary).unwrap();
    for forbidden in [
        private.key.private_key_pem(),
        private.verifier.as_str(),
        private.key.csr_pem.as_str(),
        "PRIVATE KEY",
        "access_token",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    let logout = client.logout().await.unwrap();
    assert_eq!(logout.phase, "revocation_pending");
    assert!(store.snapshot(NOW).unwrap().is_none());
    cancelled.store(true, Ordering::SeqCst);
    client.connect("desktop", "other-account").await.unwrap();
    assert!(matches!(client.poll().await, Err(IdentityError::Conflict)));
    assert!(store.snapshot(NOW).unwrap().is_none());
}

#[tokio::test]
async fn failed_start_reports_service_unavailable_instead_of_waiting_for_browser() {
    use crate::client::Route;
    use std::sync::Arc;
    let dir = Temp::new();
    let fixture = Arc::new(Fixture::new());
    let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    let backend = FakeWire(|route, _body: serde_json::Value| match route {
        Route::Start => Err(IdentityError::Unavailable),
        _ => Err(IdentityError::Conflict),
    });
    let client = NativeClient::with_backend(store.clone(), Arc::new(backend), Arc::new(|| NOW));
    assert!(matches!(
        client.connect("desktop", "device").await,
        Err(IdentityError::Unavailable)
    ));
    let summary = client.status().await.unwrap();
    assert_eq!(summary.phase, "disconnected");
    assert_eq!(
        summary.error_code.as_deref(),
        Some("CERT_SERVICE_UNAVAILABLE")
    );
    assert!(store.read().unwrap().pending.is_some());
}

#[tokio::test]
async fn poll_keeps_membership_reauth_and_outage_distinct() {
    use crate::client::Route;
    use std::sync::Arc;
    for (status, code) in [
        (403, "ORG_MEMBERSHIP_REQUIRED"),
        (401, "CERT_REAUTH_REQUIRED"),
        (503, "CERT_SERVICE_UNAVAILABLE"),
    ] {
        let dir = Temp::new();
        let fixture = Arc::new(Fixture::new());
        let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
        let backend_store = store.clone();
        let backend = FakeWire(move |route, _body: serde_json::Value| match route {
            Route::Start => reply(started(backend_store.read()?.pending.as_ref().unwrap())),
            Route::Poll => Ok(crate::client::Reply {
                status,
                body: serde_json::json!({"code": code, "error": "closed"})
                    .to_string()
                    .into_bytes(),
            }),
            _ => Err(IdentityError::Conflict),
        });
        let client = NativeClient::with_backend(store, Arc::new(backend), Arc::new(|| NOW));
        client.connect("desktop", "device").await.unwrap();
        let err = client.poll().await.unwrap_err();
        assert_eq!(err.code(), code);
        assert_eq!(
            client.status().await.unwrap().error_code.as_deref(),
            Some(code)
        );
    }
}

#[tokio::test]
async fn unreachable_enrollment_is_not_a_server_outage() {
    use crate::client::Route;
    use std::sync::Arc;
    let dir = Temp::new();
    let fixture = Arc::new(Fixture::new());
    let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    let backend = FakeWire(|route, _body: serde_json::Value| match route {
        Route::Start => Err(IdentityError::Network),
        _ => Err(IdentityError::Conflict),
    });
    let client = NativeClient::with_backend(store, Arc::new(backend), Arc::new(|| NOW));
    assert!(matches!(
        client.connect("desktop", "device").await,
        Err(IdentityError::Network)
    ));
    assert_eq!(
        client.status().await.unwrap().error_code.as_deref(),
        Some("IDENTITY_NETWORK_UNAVAILABLE")
    );
}

#[tokio::test]
async fn lost_renewal_response_recovers_after_old_expiry_without_new_issuance() {
    use crate::client::Route;
    use std::sync::{
        atomic::{AtomicI64, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    let dir = Temp::new();
    let fixture = Arc::new(Fixture::new());
    let store = IdentityStore::open(&dir.0, fixture.trust.clone()).unwrap();
    let backend_store = store.clone();
    let backend_fixture = fixture.clone();
    let now = Arc::new(AtomicI64::new(NOW));
    let wire_now = now.clone();
    let committed = Arc::new(Mutex::new(None::<IssuedCertificate>));
    let result = committed.clone();
    let issues = Arc::new(AtomicUsize::new(0));
    let issue_count = issues.clone();
    let backend = FakeWire(move |route, _body: serde_json::Value| {
        let state = backend_store.read()?;
        match route {
            Route::Start => reply(started(state.pending.as_ref().unwrap())),
            Route::Poll => {
                let pending = state.pending.as_ref().unwrap();
                reply(
                    serde_json::json!({"status":"issued","interval":5,"certificate":backend_fixture.issue(&pending.key,&pending.device_id)}),
                )
            }
            Route::Recover => {
                if let Some(value) = result.lock().unwrap().clone() {
                    reply(value)
                } else {
                    Ok(crate::client::Reply {
                        status: 409,
                        body: br#"{"code":"CERT_OPERATION_CONFLICT"}"#.to_vec(),
                    })
                }
            }
            Route::Renew => {
                issue_count.fetch_add(1, Ordering::SeqCst);
                let pending = state.renewal.as_ref().unwrap();
                let old = state.active.as_ref().unwrap();
                let next = backend_fixture.issue_at(
                    &pending.key,
                    &old.certificate.identity.device_id,
                    wire_now.load(Ordering::SeqCst),
                    43,
                );
                *result.lock().unwrap() = Some(next);
                Err(IdentityError::Unavailable)
            }
            _ => Err(IdentityError::Unavailable),
        }
    });
    let client_now = now.clone();
    let client = NativeClient::with_backend(
        store.clone(),
        Arc::new(backend),
        Arc::new(move || client_now.load(Ordering::SeqCst)),
    );
    client.connect("linux_cli", "device").await.unwrap();
    client.poll().await.unwrap();
    now.store(NOW + 30 * 86400 - 3600, Ordering::SeqCst);
    assert!(matches!(
        client.renew().await,
        Err(IdentityError::Unavailable)
    ));
    assert!(store.read().unwrap().renewal.is_some());
    now.fetch_add(7200, Ordering::SeqCst);
    assert!(matches!(
        store.snapshot(now.load(Ordering::SeqCst)),
        Err(IdentityError::ReauthRequired)
    ));
    let summary = client.renew().await.unwrap();
    assert_eq!(summary.phase, "connected");
    assert_eq!(issues.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .snapshot(now.load(Ordering::SeqCst))
            .unwrap()
            .unwrap()
            .1
            .certificate
            .serial,
        "2b"
    );
}
