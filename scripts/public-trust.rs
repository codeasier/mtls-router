// Shared desktop/CLI build input: public CA certificates only.
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use x509_parser::prelude::*;

pub fn embed(repo: &Path, out: &Path, release: bool) -> PathBuf {
    let names = [
        ("CLIENT_AUTH_CA_PEM", "client-auth-ca.pem"),
        ("UPSTREAM_CA_PEM", "upstream-ca.pem"),
    ];
    let mut values = Vec::new();
    let mut provided = 0;
    for (variable, name) in names {
        println!("cargo:rerun-if-env-changed={variable}");
        let file = repo.join("trust").join(name);
        println!("cargo:rerun-if-changed={}", file.display());
        let environment = env::var(variable).ok().filter(|v| !v.trim().is_empty());
        assert!(
            !(environment.is_some() && file.exists()),
            "provide public trust files or environment values, not both"
        );
        let value = environment.or_else(|| {
            file.is_file()
                .then(|| fs::read_to_string(file).expect("read public trust"))
        });
        if value.is_some() {
            provided += 1;
        }
        values.push((name, value));
    }
    assert!(
        provided == 0 || provided == 2,
        "provide both public trust inputs or neither"
    );
    assert!(
        !release || provided == 2,
        "release public trust is required"
    );
    let directory = out.join("router-trust");
    fs::create_dir_all(&directory).expect("create public trust directory");
    for (name, value) in values {
        let pem = value.unwrap_or_else(placeholder_root);
        assert!(
            !pem.contains("PRIVATE KEY")
                && pem.contains("-----BEGIN CERTIFICATE-----")
                && pem.len() <= 65536,
            "invalid public trust material"
        );
        let mut canonical = String::new();
        let mut count = 0;
        for block in x509_parser::pem::Pem::iter_from_buffer(pem.as_bytes()) {
            let block = block.expect("invalid public trust PEM");
            assert_eq!(
                block.label, "CERTIFICATE",
                "public trust must contain certificates only"
            );
            let (tail, certificate) = X509Certificate::from_der(&block.contents)
                .expect("invalid public trust certificate");
            assert!(
                tail.is_empty() && certificate.is_ca(),
                "public trust must contain CA certificates"
            );
            if release {
                assert!(
                    certificate.validity().is_valid()
                        && !certificate
                            .subject()
                            .to_string()
                            .to_ascii_lowercase()
                            .contains("placeholder"),
                    "release public trust must be current and non-placeholder"
                );
            }
            canonical.push_str(&::pem::encode(&::pem::Pem::new(
                "CERTIFICATE",
                block.contents,
            )));
            count += 1;
        }
        assert!(
            count > 0 && (name != "client-auth-ca.pem" || count == 1),
            "invalid public trust chain count"
        );
        fs::write(directory.join(name), canonical).expect("write public trust");
    }
    directory
}

fn placeholder_root() -> String {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
    let key = KeyPair::generate().expect("temporary public trust generator");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("public trust parameters");
    params
        .distinguished_name
        .push(DnType::CommonName, "codeasier-public-placeholder");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = SystemTime::now();
    params.not_before = (now - Duration::from_secs(300)).into();
    params.not_after = (now + Duration::from_secs(86400)).into();
    // The ephemeral CA key is never serialized, written or embedded.
    params
        .self_signed(&key)
        .expect("public placeholder CA")
        .pem()
}
