use std::{env, fs, path::PathBuf};

use object::{Architecture, BinaryFormat, Object, ObjectKind};
use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-env-changed=DEPLOYMENT_ID");
    println!("cargo:rerun-if-env-changed=VERSION");
    println!("cargo:rerun-if-env-changed=MANAGEMENT_PROTOCOL_VERSION");

    let target = env::var("TARGET").expect("Cargo did not provide TARGET");
    let manager_target = manager_target(&target).unwrap_or_else(|| {
        panic!("unsupported desktop target triple: {target}");
    });
    let expected_arch = object_arch(&target).unwrap_or_else(|| {
        panic!("unsupported desktop target architecture: {target}");
    });

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    for (name, key) in [
        ("mtls-router-manager", "MTLS_MANAGER_SHA256"),
        ("mtls-router", "MTLS_ROUTER_SHA256"),
    ] {
        let path = manifest
            .join("binaries")
            .join(format!("{name}-{target}{suffix}"));
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "required {name} sidecar is missing at {} ({error}); run npm run sidecars:build",
                path.display()
            )
        });
        let file = object::File::parse(bytes.as_slice()).unwrap_or_else(|error| {
            panic!(
                "{name} sidecar at {} is not a native executable: {error}",
                path.display()
            )
        });
        assert_eq!(
            file.architecture(),
            expected_arch,
            "{name} sidecar architecture does not match target {target}"
        );
        assert_eq!(
            file.format(),
            object_format(&target),
            "{name} sidecar binary format does not match target {target}"
        );
        assert_eq!(
            file.kind(),
            ObjectKind::Executable,
            "{name} sidecar is not an executable image"
        );
        println!("cargo:rustc-env={key}={:x}", Sha256::digest(&bytes));
    }

    println!("cargo:rustc-env=MTLS_TARGET_TRIPLE={target}");
    println!("cargo:rustc-env=MTLS_MANAGER_TARGET={manager_target}");
    println!(
        "cargo:rustc-env=MTLS_DEPLOYMENT_ID={}",
        env::var("DEPLOYMENT_ID").unwrap_or_else(|_| "dev".into())
    );
    println!(
        "cargo:rustc-env=MTLS_MANAGER_VERSION={}",
        env::var("VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into())
    );
    println!(
        "cargo:rustc-env=MTLS_MANAGEMENT_PROTOCOL_VERSION={}",
        env::var("MANAGEMENT_PROTOCOL_VERSION").unwrap_or_else(|_| "4".into())
    );

    tauri_build::build()
}

fn object_format(target: &str) -> BinaryFormat {
    if target.contains("windows") {
        BinaryFormat::Pe
    } else if target.contains("apple-darwin") {
        BinaryFormat::MachO
    } else {
        BinaryFormat::Elf
    }
}

fn object_arch(target: &str) -> Option<Architecture> {
    match target.split('-').next()? {
        "aarch64" => Some(Architecture::Aarch64),
        "x86_64" => Some(Architecture::X86_64),
        _ => None,
    }
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
