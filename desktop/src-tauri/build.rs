//! Compile-time identity and embedded router credentials for the desktop.
//!
//! The Go sidecars received these through `-ldflags -X`; the embedded
//! router-core receives the same inputs here. Credentials come from
//! `secrets/` at the repository root or from `CLIENT_CERT_PEM` /
//! `CLIENT_KEY_PEM` / `UPSTREAM_CA_PEM` (all three or none, never both
//! sources). Without either a one-day placeholder pair is generated so
//! development builds never contain real material. `RELEASE_BUILD=1` rejects
//! placeholders and default identities.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const PLACEHOLDER_UPSTREAM: &str = "https://upstream.placeholder.invalid";

fn main() {
    for key in [
        "DEPLOYMENT_ID",
        "VERSION",
        "MANAGEMENT_PROTOCOL_VERSION",
        "UPSTREAM_URL",
        "CLIENT_CERT_PEM",
        "CLIENT_KEY_PEM",
        "UPSTREAM_CA_PEM",
        "RELEASE_BUILD",
        "AGENT_MODEL_PRESET_BASE64",
        "SIMPLIFY",
        "COMMIT",
        "BUILD_DATE",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }

    let target = env::var("TARGET").expect("Cargo did not provide TARGET");
    let manager_target = manager_target(&target).unwrap_or_else(|| {
        panic!("unsupported desktop target triple: {target}");
    });
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest
        .parent()
        .and_then(Path::parent)
        .expect("desktop/src-tauri lives two levels below the repository root")
        .to_path_buf();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));

    let release = env::var("RELEASE_BUILD").is_ok_and(|value| value == "1");
    let version = nonempty_env("VERSION").unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let deployment_id = nonempty_env("DEPLOYMENT_ID").unwrap_or_else(|| "dev".to_owned());
    let protocol = nonempty_env("MANAGEMENT_PROTOCOL_VERSION").unwrap_or_else(|| "4".to_owned());
    assert_eq!(
        protocol, "4",
        "unsupported MANAGEMENT_PROTOCOL_VERSION: {protocol}"
    );
    let upstream_url =
        nonempty_env("UPSTREAM_URL").unwrap_or_else(|| PLACEHOLDER_UPSTREAM.to_owned());
    let simplify = normalize_simplify(env::var("SIMPLIFY").ok().as_deref());
    let preset = env::var("AGENT_MODEL_PRESET_BASE64").unwrap_or_default();
    assert!(
        !preset.chars().any(char::is_whitespace),
        "AGENT_MODEL_PRESET_BASE64 must not contain whitespace"
    );

    let credentials = embed_credentials(&repo, &out_dir);
    if release {
        assert!(!is_default(&version), "release VERSION must be non-default");
        assert!(
            !is_default(&deployment_id),
            "release DEPLOYMENT_ID must be non-default"
        );
        assert!(
            credentials != CredentialSource::Placeholder,
            "release credentials are required"
        );
        assert!(
            upstream_url.starts_with("https://"),
            "release UPSTREAM_URL must use HTTPS"
        );
    }

    println!("cargo:rustc-env=MTLS_TARGET_TRIPLE={target}");
    println!("cargo:rustc-env=MTLS_MANAGER_TARGET={manager_target}");
    println!("cargo:rustc-env=MTLS_DEPLOYMENT_ID={deployment_id}");
    println!("cargo:rustc-env=MTLS_MANAGER_VERSION={version}");
    println!("cargo:rustc-env=MTLS_MANAGEMENT_PROTOCOL_VERSION={protocol}");
    println!("cargo:rustc-env=MTLS_UPSTREAM_URL={upstream_url}");
    println!("cargo:rustc-env=MTLS_ROUTER_COMMIT={}", commit(&repo));
    println!("cargo:rustc-env=MTLS_ROUTER_BUILD_DATE={}", build_date());
    println!("cargo:rustc-env=MTLS_AGENT_MODEL_PRESET_BASE64={preset}");
    println!(
        "cargo:rustc-env=MTLS_SIMPLIFY={}",
        if simplify { "true" } else { "false" }
    );
    println!(
        "cargo:rustc-env=MTLS_ROUTER_CREDENTIALS_DIR={}",
        out_dir.join("router-credentials").display()
    );

    tauri_build::build()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialSource {
    Files,
    Environment,
    Placeholder,
}

fn embed_credentials(repo: &Path, out_dir: &Path) -> CredentialSource {
    let files = [
        repo.join("secrets").join("client.pem"),
        repo.join("secrets").join("client.key"),
        repo.join("secrets").join("upstream-ca.pem"),
    ];
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let present = files.iter().filter(|path| path.is_file()).count();
    assert!(
        present == 0 || present == 3,
        "partial secrets set; provide all three files or none"
    );
    let embedded: Vec<Option<String>> = ["CLIENT_CERT_PEM", "CLIENT_KEY_PEM", "UPSTREAM_CA_PEM"]
        .iter()
        .map(|key| nonempty_env(key))
        .collect();
    let embedded_count = embedded.iter().filter(|value| value.is_some()).count();
    assert!(
        embedded_count == 0 || embedded_count == 3,
        "partial embedded secrets set; provide all three values or none"
    );
    assert!(
        present == 0 || embedded_count == 0,
        "provide credential files or environment values, not both"
    );

    let (cert, key, ca, source) = if embedded_count == 3 {
        let mut values = embedded.into_iter().map(Option::unwrap);
        (
            values.next().unwrap(),
            values.next().unwrap(),
            values.next().unwrap(),
            CredentialSource::Environment,
        )
    } else if present == 3 {
        (
            read(&files[0]),
            read(&files[1]),
            read(&files[2]),
            CredentialSource::Files,
        )
    } else {
        let (cert, key) = placeholder_pair();
        (cert.clone(), key, cert, CredentialSource::Placeholder)
    };

    let dir = out_dir.join("router-credentials");
    fs::create_dir_all(&dir).expect("create router credentials dir");
    for (name, contents) in [
        ("client.pem", cert),
        ("client.key", key),
        ("upstream-ca.pem", ca),
    ] {
        fs::write(dir.join(name), contents).expect("write embedded credential");
    }
    source
}

fn placeholder_pair() -> (String, String) {
    use rcgen::{CertificateParams, DnType, KeyPair};

    let key = KeyPair::generate().expect("placeholder key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("placeholder params");
    params
        .distinguished_name
        .push(DnType::CommonName, "mtls-router-placeholder");
    let now = SystemTime::now();
    params.not_before = now.into();
    params.not_after = (now + std::time::Duration::from_secs(24 * 60 * 60)).into();
    let cert = params.self_signed(&key).expect("placeholder cert");
    (cert.pem(), key.serialize_pem())
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read credential {}: {error}", path.display()))
}

fn nonempty_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn is_default(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "dev" | "unknown"
    )
}

/// `scripts/normalize-simplify.sh`: empty or `true` (any case) enables
/// simplification; `false` disables; anything else is a build error.
fn normalize_simplify(value: Option<&str>) -> bool {
    match value.map(|value| value.trim().to_ascii_lowercase()) {
        None => true,
        Some(value) if value.is_empty() || value == "true" => true,
        Some(value) if value == "false" => false,
        Some(value) => panic!("invalid SIMPLIFY value: {value}"),
    }
}

fn commit(repo: &Path) -> String {
    if let Some(commit) = nonempty_env("COMMIT") {
        return commit;
    }
    Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "rev-parse",
            "--short",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn build_date() -> String {
    if let Some(date) = nonempty_env("BUILD_DATE") {
        return date;
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let days = seconds / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let rem = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// Howard Hinnant's days-to-civil algorithm (proleptic Gregorian).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn manager_target(target: &str) -> Option<&'static str> {
    let arch = match target.split('-').next()? {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => return None,
    };
    let os = if target.contains("windows") {
        "windows"
    } else if target.contains("apple-darwin") {
        "darwin"
    } else if target.contains("linux") {
        "linux"
    } else {
        return None;
    };
    match (os, arch) {
        ("windows", "arm64") => Some("windows/arm64"),
        ("windows", "amd64") => Some("windows/amd64"),
        ("darwin", "arm64") => Some("darwin/arm64"),
        ("darwin", "amd64") => Some("darwin/amd64"),
        ("linux", "arm64") => Some("linux/arm64"),
        ("linux", "amd64") => Some("linux/amd64"),
        _ => None,
    }
}
