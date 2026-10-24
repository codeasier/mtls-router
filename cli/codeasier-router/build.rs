use std::{env, path::PathBuf, process::Command};
mod public_trust {
    include!("../../scripts/public-trust.rs");
}
fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.parent().unwrap().parent().unwrap();
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let release = env::var("RELEASE_BUILD").ok().as_deref() == Some("1");
    for key in ["VERSION", "DEPLOYMENT_ID", "RELEASE_BUILD"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let version = env::var("VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into());
    let deployment = env::var("DEPLOYMENT_ID").unwrap_or_else(|_| "dev".into());
    assert!(
        !release || !matches!(deployment.as_str(), "" | "dev" | "unknown"),
        "release DEPLOYMENT_ID must be non-default"
    );
    let trust = public_trust::embed(repo, &out, release);
    let commit = Command::new("git")
        .args([
            "-C",
            &repo.to_string_lossy(),
            "rev-parse",
            "--short",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|v| v.status.success())
        .map(|v| String::from_utf8_lossy(&v.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ROUTER_VERSION={version}");
    println!("cargo:rustc-env=ROUTER_DEPLOYMENT_ID={deployment}");
    println!("cargo:rustc-env=ROUTER_COMMIT={commit}");
    println!(
        "cargo:rustc-env=ROUTER_TARGET={}",
        env::var("TARGET").unwrap()
    );
    println!("cargo:rustc-env=ROUTER_TRUST_DIR={}", trust.display());
}
