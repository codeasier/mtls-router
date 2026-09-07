use crate::protocol::{ManagerInfoResult, MANAGEMENT_PROTOCOL_VERSION};

/// Artifact identity used by production packaging checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactIdentity {
    pub deployment_id: String,
    pub management_protocol_version: String,
}

/// Code-owned manager handshake metadata, matching Go `metadata.Info()`.
///
/// `management_protocol_version` is the protocol constant `"4"` and is never
/// taken from build-time env injection.
pub fn info() -> ManagerInfoResult {
    ManagerInfoResult {
        version: env!("MTLS_MANAGER_VERSION").to_owned(),
        commit: option_env!("MTLS_ROUTER_COMMIT")
            .unwrap_or("unknown")
            .to_owned(),
        build_date: option_env!("MTLS_ROUTER_BUILD_DATE")
            .unwrap_or("unknown")
            .to_owned(),
        target: env!("MTLS_MANAGER_TARGET").to_owned(),
        deployment_id: env!("MTLS_DEPLOYMENT_ID").to_owned(),
        management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.to_owned(),
    }
}

/// Rejects development/default identities and mixed artifact generations.
pub fn validate_production(artifacts: &[ArtifactIdentity]) -> Result<(), String> {
    let Some(want) = artifacts.first() else {
        return Err("no artifact identities supplied".to_owned());
    };
    if default_value(&want.deployment_id) {
        return Err("production deployment ID is empty or default".to_owned());
    }
    if want.management_protocol_version.trim().is_empty() {
        return Err("management protocol version is empty".to_owned());
    }
    if want.management_protocol_version != MANAGEMENT_PROTOCOL_VERSION {
        return Err("management protocol version is not code-owned version".to_owned());
    }
    if artifacts.iter().skip(1).any(|artifact| artifact != want) {
        return Err("artifact identity mismatch".to_owned());
    }
    Ok(())
}

fn default_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "dev" | "unknown"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_reports_nonempty_code_owned_protocol() {
        let got = info();
        assert_eq!(got.management_protocol_version, MANAGEMENT_PROTOCOL_VERSION);
        assert!(!got.deployment_id.is_empty());
        assert!(got.target.contains('/'));
    }

    #[test]
    fn validate_production_matches_go_contract() {
        let valid = ArtifactIdentity {
            deployment_id: "service-prod-a".into(),
            management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
        };
        assert!(validate_production(&[valid.clone(), valid.clone(), valid.clone()]).is_ok());
        for artifacts in [
            Vec::new(),
            vec![ArtifactIdentity {
                deployment_id: "dev".into(),
                management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
            }],
            vec![ArtifactIdentity {
                deployment_id: "unknown".into(),
                management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
            }],
            vec![ArtifactIdentity {
                deployment_id: "service-prod-a".into(),
                management_protocol_version: String::new(),
            }],
            vec![ArtifactIdentity {
                deployment_id: "service-prod-a".into(),
                management_protocol_version: "different".into(),
            }],
            vec![
                valid.clone(),
                ArtifactIdentity {
                    deployment_id: "service-prod-b".into(),
                    management_protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
                },
            ],
        ] {
            assert!(
                validate_production(&artifacts).is_err(),
                "expected rejection for {artifacts:?}"
            );
        }
    }
}
