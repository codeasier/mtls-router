use std::collections::HashMap;
use std::path::{Component, Path};

use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

use super::canonical::{canonical_value, marshal_jcs, JsonNumber, JsonValue};
use super::decode::decode_value;
use super::types::Agent;

pub const TOKEN_VERSION: i64 = 1;
pub const CANONICALIZATION_JCS1: &str = "jcs-rfc8785-v1";
pub const MAX_CATALOG_TOKEN_SIZE: usize = 512 << 10;
pub const MAX_REVISION_TOKEN_SIZE: usize = 128 << 10;
pub const MAX_CLEANUP_REVISION_TOKEN_SIZE: usize = 128 << 10;
const MAX_TOKEN_PAYLOAD_SIZE: usize = 384 << 10;
const MAX_TOKEN_STRING_FIELD: usize = 4096;
const MAX_CATALOG_TOKEN_MODELS: usize = 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenError {
    Invalid,
    Limit,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => f.write_str("invalid authenticated token"),
            Self::Limit => f.write_str("authenticated token exceeds limits"),
        }
    }
}

impl std::error::Error for TokenError {}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogClaims {
    pub version: i64,
    pub models: Vec<String>,
    pub agents: Vec<Agent>,
    pub owner: String,
    pub router_base_url: String,
    pub deployment_id: String,
    pub protocol_version: String,
    pub simplify: bool,
    pub canonicalization: String,
    pub key_generation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RevisionClaims {
    pub version: i64,
    pub agent_plans: Vec<RevisionAgent>,
    pub canonical_config: Vec<u8>,
    pub catalog_identity: String,
    pub sidecar_revision: RevisionState,
    pub router_base_url: String,
    pub deployment_id: String,
    pub protocol_version: String,
    pub canonicalization: String,
    pub key_generation: String,
    pub managed_drift: bool,
    pub requires_codex_auth_approval: bool,
    pub drifted_agents: Vec<Agent>,
    pub files: Vec<RevisionFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionAgent {
    pub agent: Agent,
    pub mode: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionState {
    pub exists: bool,
    pub size: i64,
    pub mode: u32,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionFile {
    pub agent: Agent,
    pub role: String,
    pub format: String,
    pub source_path: String,
    pub target_path: String,
    pub operation: String,
    pub backup_required: bool,
    pub backup_source: String,
    pub source_revision: RevisionState,
    pub target_revision: RevisionState,
    pub companion_exists: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupRevisionClaims {
    pub agent: Agent,
    pub files: Vec<CleanupRevisionFile>,
    pub state_operation: String,
    pub state_revision: RevisionState,
    pub managed_config_drift: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupRevisionFile {
    pub role: String,
    pub source_path: String,
    pub target_path: String,
    pub operation: String,
    pub backup_required: bool,
    pub backup_source: String,
    pub source_revision: RevisionState,
    pub target_revision: RevisionState,
    pub removed_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TokenSigner {
    key: [u8; 32],
    generation: String,
}

impl TokenSigner {
    pub fn new(key: &[u8], generation: impl Into<String>) -> Result<Self, TokenError> {
        let generation = generation.into();
        if key.len() != 32 || !valid_token_field(&generation) {
            return Err(TokenError::Invalid);
        }
        let mut material = [0u8; 32];
        material.copy_from_slice(key);
        Ok(Self {
            key: material,
            generation,
        })
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn sign_catalog(&self, mut claims: CatalogClaims) -> Result<String, TokenError> {
        if !valid_agents(&claims.agents) {
            return Err(TokenError::Invalid);
        }
        claims = normalize_catalog_claims(claims);
        if claims.version == 0 {
            claims.version = TOKEN_VERSION;
        }
        if claims.canonicalization.is_empty() {
            claims.canonicalization = CANONICALIZATION_JCS1.to_owned();
        }
        claims.key_generation = self.generation.clone();
        validate_catalog_claims(&claims)?;
        self.sign(
            "catalog-token-v1",
            &catalog_to_value(&claims),
            MAX_CATALOG_TOKEN_SIZE,
        )
    }

    pub fn verify_catalog(&self, token: &str) -> Result<CatalogClaims, TokenError> {
        let value = self.verify("catalog-token-v1", token, MAX_CATALOG_TOKEN_SIZE)?;
        let claims = catalog_from_value(&value)?;
        validate_catalog_claims(&claims)?;
        if claims.key_generation != self.generation {
            return Err(TokenError::Invalid);
        }
        let normalized = normalize_catalog_claims(claims.clone());
        if catalog_to_value(&claims) != catalog_to_value(&normalized) {
            return Err(TokenError::Invalid);
        }
        Ok(claims)
    }

    pub fn sign_revision(&self, mut claims: RevisionClaims) -> Result<String, TokenError> {
        if !valid_revision_agents(&claims.agent_plans) || !valid_agents(&claims.drifted_agents) {
            return Err(TokenError::Invalid);
        }
        claims = normalize_revision_claims(claims);
        if claims.version == 0 {
            claims.version = TOKEN_VERSION;
        }
        if claims.canonicalization.is_empty() {
            claims.canonicalization = CANONICALIZATION_JCS1.to_owned();
        }
        claims.key_generation = self.generation.clone();
        validate_revision_claims(&claims)?;
        self.sign(
            "revision-token-v1",
            &revision_to_value(&claims)?,
            MAX_REVISION_TOKEN_SIZE,
        )
    }

    pub fn verify_revision(&self, token: &str) -> Result<RevisionClaims, TokenError> {
        let value = self.verify("revision-token-v1", token, MAX_REVISION_TOKEN_SIZE)?;
        let claims = revision_from_value(&value)?;
        validate_revision_claims(&claims)?;
        if claims.key_generation != self.generation {
            return Err(TokenError::Invalid);
        }
        let normalized = normalize_revision_claims(claims.clone());
        let want = marshal_jcs(&revision_to_value(&claims)?)?;
        let got = marshal_jcs(&revision_to_value(&normalized)?)?;
        if want != got {
            return Err(TokenError::Invalid);
        }
        Ok(claims)
    }

    pub fn sign_cleanup_revision(
        &self,
        claims: CleanupRevisionClaims,
    ) -> Result<String, TokenError> {
        validate_cleanup_revision_claims(&claims)?;
        let envelope = cleanup_envelope_value(&self.generation, &claims);
        self.sign(
            "cleanup-revision-token-v1",
            &envelope,
            MAX_CLEANUP_REVISION_TOKEN_SIZE,
        )
    }

    pub fn verify_cleanup_revision(
        &self,
        token: &str,
    ) -> Result<CleanupRevisionClaims, TokenError> {
        let value = self.verify(
            "cleanup-revision-token-v1",
            token,
            MAX_CLEANUP_REVISION_TOKEN_SIZE,
        )?;
        let (version, canonicalization, generation, claims) = cleanup_from_envelope(&value)?;
        if version != TOKEN_VERSION
            || canonicalization != CANONICALIZATION_JCS1
            || generation != self.generation
            || validate_cleanup_revision_claims(&claims).is_err()
        {
            return Err(TokenError::Invalid);
        }
        Ok(claims)
    }

    pub fn catalog_identity(claims: &CatalogClaims) -> String {
        let encoded = marshal_jcs(&catalog_to_value(claims)).unwrap_or_default();
        let digest = Sha256::digest(&encoded);
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn revision_mac(
        &self,
        context: &str,
        identity: &str,
        content: &[u8],
    ) -> Result<String, TokenError> {
        if !valid_token_field(context) || !valid_token_field(identity) {
            return Err(TokenError::Invalid);
        }
        let key = self.derived_key(&format!("file-revision-v1\x00{context}"));
        let mut mac_input = identity.as_bytes().to_vec();
        mac_input.push(0);
        mac_input.extend_from_slice(content);
        Ok(URL_SAFE_NO_PAD.encode(hmac_sha256(&key, &mac_input)))
    }

    fn sign(&self, context: &str, claims: &JsonValue, limit: usize) -> Result<String, TokenError> {
        let payload = marshal_jcs(claims).map_err(|_| TokenError::Invalid)?;
        if payload.len() > MAX_TOKEN_PAYLOAD_SIZE {
            return Err(TokenError::Limit);
        }
        let key = self.derived_key(context);
        let token = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(hmac_sha256(&key, &payload))
        );
        if token.len() > limit {
            return Err(TokenError::Limit);
        }
        Ok(token)
    }

    fn verify(&self, context: &str, token: &str, limit: usize) -> Result<JsonValue, TokenError> {
        if token.is_empty() || token.len() > limit || token.matches('.').count() != 1 {
            return Err(TokenError::Invalid);
        }
        let Some((payload_part, signature_part)) = token.split_once('.') else {
            return Err(TokenError::Invalid);
        };
        let payload = URL_SAFE_NO_PAD
            .decode(payload_part)
            .map_err(|_| TokenError::Invalid)?;
        if payload.is_empty() || payload.len() > MAX_TOKEN_PAYLOAD_SIZE {
            return Err(TokenError::Invalid);
        }
        let signature = URL_SAFE_NO_PAD
            .decode(signature_part)
            .map_err(|_| TokenError::Invalid)?;
        if signature.len() != 32 {
            return Err(TokenError::Invalid);
        }
        let key = self.derived_key(context);
        if hmac_sha256(&key, &payload).as_slice() != signature.as_slice() {
            return Err(TokenError::Invalid);
        }
        let value = decode_value(&payload).map_err(|_| TokenError::Invalid)?;
        let canonical = marshal_jcs(&value).map_err(|_| TokenError::Invalid)?;
        if canonical != payload {
            return Err(TokenError::Invalid);
        }
        Ok(value)
    }

    fn derived_key(&self, context: &str) -> [u8; 32] {
        hmac_sha256(
            &self.key,
            format!("mtls-router-agent-modelconfig\x00{context}").as_bytes(),
        )
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut normalized = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        normalized[..digest.len()].copy_from_slice(&digest);
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; BLOCK];
    let mut outer_pad = [0x5cu8; BLOCK];
    for index in 0..BLOCK {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(data);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_hash);
    outer.finalize().into()
}

fn normalize_catalog_claims(mut claims: CatalogClaims) -> CatalogClaims {
    claims.models = normalize_strings(claims.models);
    claims.agents = normalize_agents(claims.agents);
    claims
}

fn normalize_revision_claims(mut claims: RevisionClaims) -> RevisionClaims {
    claims.agent_plans.sort_by_key(|plan| plan.agent.rank());
    claims.drifted_agents = normalize_agents(claims.drifted_agents);
    claims.files.sort_by(|left, right| {
        left.agent
            .rank()
            .cmp(&right.agent.rank())
            .then_with(|| left.role.cmp(&right.role))
            .then_with(|| left.target_path.cmp(&right.target_path))
    });
    claims
}

fn normalize_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn normalize_agents(values: Vec<Agent>) -> Vec<Agent> {
    let seen: std::collections::HashSet<Agent> = values.into_iter().collect();
    Agent::all()
        .into_iter()
        .filter(|agent| seen.contains(agent))
        .collect()
}

fn valid_agents(values: &[Agent]) -> bool {
    values
        .iter()
        .all(|agent| Agent::parse(agent.as_str()).is_some())
}

fn valid_revision_agents(values: &[RevisionAgent]) -> bool {
    valid_agents(&values.iter().map(|value| value.agent).collect::<Vec<_>>())
}

fn validate_catalog_claims(claims: &CatalogClaims) -> Result<(), TokenError> {
    if claims.version != TOKEN_VERSION
        || claims.canonicalization != CANONICALIZATION_JCS1
        || claims.models.is_empty()
        || claims.models.len() > MAX_CATALOG_TOKEN_MODELS
        || claims.agents.is_empty()
        || (claims.owner != "cli" && claims.owner != "desktop")
    {
        return Err(TokenError::Invalid);
    }
    for value in claims.models.iter().cloned().chain([
        claims.owner.clone(),
        claims.router_base_url.clone(),
        claims.deployment_id.clone(),
        claims.protocol_version.clone(),
        claims.canonicalization.clone(),
        claims.key_generation.clone(),
    ]) {
        if !valid_token_field(&value) {
            return Err(TokenError::Invalid);
        }
    }
    Ok(())
}

fn validate_revision_claims(claims: &RevisionClaims) -> Result<(), TokenError> {
    if claims.version != TOKEN_VERSION
        || claims.canonicalization != CANONICALIZATION_JCS1
        || claims.agent_plans.is_empty()
        || claims.agent_plans.len() > 3
        || claims.canonical_config.is_empty()
        || claims.files.len() > 16
    {
        return Err(TokenError::Invalid);
    }
    for value in [
        &claims.catalog_identity,
        &claims.router_base_url,
        &claims.deployment_id,
        &claims.protocol_version,
        &claims.canonicalization,
        &claims.key_generation,
    ] {
        if !valid_token_field(value) {
            return Err(TokenError::Invalid);
        }
    }
    if !valid_revision_state(&claims.sidecar_revision) {
        return Err(TokenError::Invalid);
    }
    let mut planned = HashMap::new();
    let mut files_by_agent: HashMap<Agent, HashMap<String, RevisionFile>> = HashMap::new();
    for plan in &claims.agent_plans {
        if planned.contains_key(&plan.agent) || (plan.mode != "merge" && plan.mode != "rebuild") {
            return Err(TokenError::Invalid);
        }
        planned.insert(plan.agent, true);
        files_by_agent.insert(plan.agent, HashMap::new());
    }
    let mut seen_files = HashMap::new();
    for file in &claims.files {
        let identity = format!(
            "{}\x00{}\x00{}",
            file.agent.as_str(),
            file.role,
            file.target_path
        );
        if !planned.contains_key(&file.agent)
            || seen_files.contains_key(&identity)
            || (file.role != "config" && file.role != "auth")
            || (file.format != "json" && file.format != "jsonc" && file.format != "toml")
            || (file.operation != "create" && file.operation != "replace")
            || !valid_token_field(&file.source_path)
            || !valid_token_field(&file.target_path)
            || file.backup_required != file.source_revision.exists
            || file.backup_required != !file.backup_source.is_empty()
            || (!file.backup_source.is_empty() && !valid_token_field(&file.backup_source))
            || !valid_revision_state(&file.source_revision)
            || !valid_revision_state(&file.target_revision)
            || (file.operation == "create") != !file.target_revision.exists
            || (file.operation == "replace") != file.target_revision.exists
            || (file.backup_required && file.backup_source != file.source_path)
            || (file.source_path == file.target_path
                && file.source_revision != file.target_revision)
            || (file.source_path != file.target_path && file.target_revision.exists)
        {
            return Err(TokenError::Invalid);
        }
        seen_files.insert(identity, true);
        let slots = files_by_agent
            .get_mut(&file.agent)
            .ok_or(TokenError::Invalid)?;
        if slots.contains_key(&file.role) {
            return Err(TokenError::Invalid);
        }
        slots.insert(file.role.clone(), file.clone());
    }
    for (agent, files) in &files_by_agent {
        let config = files.get("config");
        match agent {
            Agent::Claude => {
                if config.is_none()
                    || files.len() != 1
                    || config.is_some_and(|file| file.format != "json" || file.companion_exists)
                {
                    return Err(TokenError::Invalid);
                }
            }
            Agent::OpenCode => {
                if config.is_none()
                    || files.len() != 1
                    || config.is_some_and(|file| {
                        (file.format != "json" && file.format != "jsonc") || file.companion_exists
                    })
                {
                    return Err(TokenError::Invalid);
                }
            }
            Agent::Codex => {
                let auth = files.get("auth");
                if config.is_none()
                    || auth.is_none()
                    || files.len() != 2
                    || config.is_some_and(|file| file.format != "toml")
                    || auth.is_some_and(|file| file.format != "json")
                    || config.is_some_and(|file| {
                        auth.is_some_and(|auth| {
                            file.companion_exists != auth.source_revision.exists
                        })
                    })
                    || auth.is_some_and(|file| {
                        config.is_some_and(|config| {
                            file.companion_exists != config.source_revision.exists
                        })
                    })
                {
                    return Err(TokenError::Invalid);
                }
            }
        }
    }
    let parsed = decode_value(&claims.canonical_config).map_err(|_| TokenError::Invalid)?;
    let canonical = marshal_jcs(&parsed).map_err(|_| TokenError::Invalid)?;
    if claims.canonical_config != canonical {
        return Err(TokenError::Invalid);
    }
    Ok(())
}

fn validate_cleanup_revision_claims(claims: &CleanupRevisionClaims) -> Result<(), TokenError> {
    if !valid_agents(&[claims.agent])
        || claims.files.len() > 16
        || (claims.state_operation != "replace" && claims.state_operation != "delete")
        || !claims.state_revision.exists
        || !valid_revision_state(&claims.state_revision)
    {
        return Err(TokenError::Invalid);
    }
    let mut seen = HashMap::new();
    for file in &claims.files {
        let identity = format!("{}\x00{}", file.role, file.target_path);
        let absent_delete = !file.source_revision.exists
            && file.operation == "delete"
            && !file.backup_required
            && file.backup_source.is_empty()
            && file.removed_paths.is_empty();
        if (absent_delete && !claims.managed_config_drift)
            || seen.contains_key(&identity)
            || (file.role != "config" && file.role != "auth")
            || (file.operation != "replace" && file.operation != "delete")
            || !valid_absolute_path(&file.source_path)
            || !valid_absolute_path(&file.target_path)
            || file.source_path != file.target_path
            || file.source_revision != file.target_revision
            || !valid_revision_state(&file.source_revision)
            || (!absent_delete
                && (!file.source_revision.exists
                    || !file.backup_required
                    || file.backup_source != file.source_path
                    || (file.removed_paths.is_empty() && file.operation != "delete")
                    || (!file.removed_paths.is_empty()
                        && !valid_sorted_unique_token_fields(&file.removed_paths))))
        {
            return Err(TokenError::Invalid);
        }
        seen.insert(identity, true);
    }
    Ok(())
}

fn valid_absolute_path(value: &str) -> bool {
    valid_token_field(value)
        && (Path::new(value).is_absolute()
            || value.starts_with('/')
            || matches!(
                Path::new(value).components().next(),
                Some(Component::RootDir)
            ))
}

fn valid_sorted_unique_token_fields(values: &[String]) -> bool {
    if values.len() > 256 {
        return false;
    }
    values.iter().enumerate().all(|(index, value)| {
        valid_token_field(value) && (index == 0 || values[index - 1] < *value)
    })
}

fn valid_revision_state(value: &RevisionState) -> bool {
    if !value.exists {
        value.size == 0 && value.mode == 0 && value.digest.is_empty()
    } else {
        value.size >= 0 && value.size <= 1 << 30 && valid_token_field(&value.digest)
    }
}

fn valid_token_field(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TOKEN_STRING_FIELD && value.trim() == value
}

fn catalog_to_value(claims: &CatalogClaims) -> JsonValue {
    let mut object = HashMap::new();
    object.insert("version".to_owned(), number(claims.version));
    object.insert(
        "models".to_owned(),
        JsonValue::Array(
            claims
                .models
                .iter()
                .map(|value| JsonValue::String(value.clone()))
                .collect(),
        ),
    );
    object.insert(
        "agents".to_owned(),
        JsonValue::Array(
            claims
                .agents
                .iter()
                .map(|agent| JsonValue::String(agent.as_str().to_owned()))
                .collect(),
        ),
    );
    object.insert("owner".to_owned(), JsonValue::String(claims.owner.clone()));
    object.insert(
        "router_base_url".to_owned(),
        JsonValue::String(claims.router_base_url.clone()),
    );
    object.insert(
        "deployment_id".to_owned(),
        JsonValue::String(claims.deployment_id.clone()),
    );
    object.insert(
        "protocol_version".to_owned(),
        JsonValue::String(claims.protocol_version.clone()),
    );
    if claims.simplify {
        object.insert("simplify".to_owned(), JsonValue::Bool(true));
    }
    object.insert(
        "canonicalization".to_owned(),
        JsonValue::String(claims.canonicalization.clone()),
    );
    object.insert(
        "key_generation".to_owned(),
        JsonValue::String(claims.key_generation.clone()),
    );
    JsonValue::Object(object)
}

fn catalog_from_value(value: &JsonValue) -> Result<CatalogClaims, TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(
        object,
        &[
            "version",
            "models",
            "agents",
            "owner",
            "router_base_url",
            "deployment_id",
            "protocol_version",
            "simplify",
            "canonicalization",
            "key_generation",
        ],
    )?;
    Ok(CatalogClaims {
        version: required_int(object, "version")?,
        models: required_string_array(object, "models")?,
        agents: required_agents(object, "agents")?,
        owner: required_string(object, "owner")?,
        router_base_url: required_string(object, "router_base_url")?,
        deployment_id: required_string(object, "deployment_id")?,
        protocol_version: required_string(object, "protocol_version")?,
        simplify: optional_bool(object, "simplify")?,
        canonicalization: required_string(object, "canonicalization")?,
        key_generation: required_string(object, "key_generation")?,
    })
}

fn revision_to_value(claims: &RevisionClaims) -> Result<JsonValue, TokenError> {
    let canonical = decode_value(&claims.canonical_config).map_err(|_| TokenError::Invalid)?;
    let mut object = HashMap::new();
    object.insert("version".to_owned(), number(claims.version));
    object.insert(
        "agent_plans".to_owned(),
        JsonValue::Array(
            claims
                .agent_plans
                .iter()
                .map(|plan| {
                    JsonValue::object([
                        (
                            "agent".to_owned(),
                            JsonValue::String(plan.agent.as_str().into()),
                        ),
                        ("mode".to_owned(), JsonValue::String(plan.mode.clone())),
                    ])
                })
                .collect(),
        ),
    );
    object.insert("canonical_config".to_owned(), canonical);
    object.insert(
        "catalog_identity".to_owned(),
        JsonValue::String(claims.catalog_identity.clone()),
    );
    object.insert(
        "sidecar_revision".to_owned(),
        revision_state_to_value(&claims.sidecar_revision),
    );
    object.insert(
        "router_base_url".to_owned(),
        JsonValue::String(claims.router_base_url.clone()),
    );
    object.insert(
        "deployment_id".to_owned(),
        JsonValue::String(claims.deployment_id.clone()),
    );
    object.insert(
        "protocol_version".to_owned(),
        JsonValue::String(claims.protocol_version.clone()),
    );
    object.insert(
        "canonicalization".to_owned(),
        JsonValue::String(claims.canonicalization.clone()),
    );
    object.insert(
        "key_generation".to_owned(),
        JsonValue::String(claims.key_generation.clone()),
    );
    object.insert(
        "managed_drift".to_owned(),
        JsonValue::Bool(claims.managed_drift),
    );
    object.insert(
        "requires_codex_auth_approval".to_owned(),
        JsonValue::Bool(claims.requires_codex_auth_approval),
    );
    object.insert(
        "drifted_agents".to_owned(),
        JsonValue::Array(
            claims
                .drifted_agents
                .iter()
                .map(|agent| JsonValue::String(agent.as_str().into()))
                .collect(),
        ),
    );
    object.insert(
        "files".to_owned(),
        JsonValue::Array(claims.files.iter().map(revision_file_to_value).collect()),
    );
    Ok(JsonValue::Object(object))
}

fn revision_from_value(value: &JsonValue) -> Result<RevisionClaims, TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(
        object,
        &[
            "version",
            "agent_plans",
            "canonical_config",
            "catalog_identity",
            "sidecar_revision",
            "router_base_url",
            "deployment_id",
            "protocol_version",
            "canonicalization",
            "key_generation",
            "managed_drift",
            "requires_codex_auth_approval",
            "drifted_agents",
            "files",
        ],
    )?;
    let canonical = object.get("canonical_config").ok_or(TokenError::Invalid)?;
    Ok(RevisionClaims {
        version: required_int(object, "version")?,
        agent_plans: required_array(object, "agent_plans")?
            .iter()
            .map(revision_agent_from_value)
            .collect::<Result<_, _>>()?,
        canonical_config: canonical_value(canonical).map_err(|_| TokenError::Invalid)?,
        catalog_identity: required_string(object, "catalog_identity")?,
        sidecar_revision: revision_state_from_value(object.get("sidecar_revision"))?,
        router_base_url: required_string(object, "router_base_url")?,
        deployment_id: required_string(object, "deployment_id")?,
        protocol_version: required_string(object, "protocol_version")?,
        canonicalization: required_string(object, "canonicalization")?,
        key_generation: required_string(object, "key_generation")?,
        managed_drift: required_bool(object, "managed_drift")?,
        requires_codex_auth_approval: required_bool(object, "requires_codex_auth_approval")?,
        drifted_agents: optional_agents(object, "drifted_agents")?,
        files: required_array(object, "files")?
            .iter()
            .map(revision_file_from_value)
            .collect::<Result<_, _>>()?,
    })
}

fn revision_file_to_value(file: &RevisionFile) -> JsonValue {
    let mut object = HashMap::new();
    object.insert(
        "agent".to_owned(),
        JsonValue::String(file.agent.as_str().into()),
    );
    object.insert("role".to_owned(), JsonValue::String(file.role.clone()));
    object.insert("format".to_owned(), JsonValue::String(file.format.clone()));
    object.insert(
        "source_path".to_owned(),
        JsonValue::String(file.source_path.clone()),
    );
    object.insert(
        "target_path".to_owned(),
        JsonValue::String(file.target_path.clone()),
    );
    object.insert(
        "operation".to_owned(),
        JsonValue::String(file.operation.clone()),
    );
    object.insert(
        "backup_required".to_owned(),
        JsonValue::Bool(file.backup_required),
    );
    if !file.backup_source.is_empty() {
        object.insert(
            "backup_source".to_owned(),
            JsonValue::String(file.backup_source.clone()),
        );
    }
    object.insert(
        "source_revision".to_owned(),
        revision_state_to_value(&file.source_revision),
    );
    object.insert(
        "target_revision".to_owned(),
        revision_state_to_value(&file.target_revision),
    );
    if file.companion_exists {
        object.insert("companion_exists".to_owned(), JsonValue::Bool(true));
    }
    JsonValue::Object(object)
}

fn revision_state_to_value(state: &RevisionState) -> JsonValue {
    let mut object = HashMap::new();
    object.insert("exists".to_owned(), JsonValue::Bool(state.exists));
    if state.exists {
        if state.size != 0 {
            object.insert("size".to_owned(), number(state.size));
        }
        if state.mode != 0 {
            object.insert("mode".to_owned(), number(i64::from(state.mode)));
        }
        if !state.digest.is_empty() {
            object.insert("digest".to_owned(), JsonValue::String(state.digest.clone()));
        }
    }
    JsonValue::Object(object)
}

fn revision_state_from_value(value: Option<&JsonValue>) -> Result<RevisionState, TokenError> {
    let object = value
        .and_then(JsonValue::as_object)
        .ok_or(TokenError::Invalid)?;
    reject_unknown(object, &["exists", "size", "mode", "digest"])?;
    Ok(RevisionState {
        exists: required_bool(object, "exists")?,
        size: optional_int(object, "size")?,
        mode: u32::try_from(optional_int(object, "mode")?).map_err(|_| TokenError::Invalid)?,
        digest: optional_string(object, "digest")?,
    })
}

fn revision_agent_from_value(value: &JsonValue) -> Result<RevisionAgent, TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(object, &["agent", "mode"])?;
    Ok(RevisionAgent {
        agent: Agent::parse(&required_string(object, "agent")?).ok_or(TokenError::Invalid)?,
        mode: required_string(object, "mode")?,
    })
}

fn revision_file_from_value(value: &JsonValue) -> Result<RevisionFile, TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(
        object,
        &[
            "agent",
            "role",
            "format",
            "source_path",
            "target_path",
            "operation",
            "backup_required",
            "backup_source",
            "source_revision",
            "target_revision",
            "companion_exists",
        ],
    )?;
    Ok(RevisionFile {
        agent: Agent::parse(&required_string(object, "agent")?).ok_or(TokenError::Invalid)?,
        role: required_string(object, "role")?,
        format: required_string(object, "format")?,
        source_path: required_string(object, "source_path")?,
        target_path: required_string(object, "target_path")?,
        operation: required_string(object, "operation")?,
        backup_required: required_bool(object, "backup_required")?,
        backup_source: optional_string(object, "backup_source")?,
        source_revision: revision_state_from_value(object.get("source_revision"))?,
        target_revision: revision_state_from_value(object.get("target_revision"))?,
        companion_exists: optional_bool(object, "companion_exists")?,
    })
}

fn cleanup_envelope_value(generation: &str, claims: &CleanupRevisionClaims) -> JsonValue {
    JsonValue::object([
        ("version".to_owned(), number(TOKEN_VERSION)),
        (
            "canonicalization".to_owned(),
            JsonValue::String(CANONICALIZATION_JCS1.to_owned()),
        ),
        (
            "key_generation".to_owned(),
            JsonValue::String(generation.to_owned()),
        ),
        ("claims".to_owned(), cleanup_claims_to_value(claims)),
    ])
}

fn cleanup_claims_to_value(claims: &CleanupRevisionClaims) -> JsonValue {
    JsonValue::object([
        (
            "agent".to_owned(),
            JsonValue::String(claims.agent.as_str().into()),
        ),
        (
            "files".to_owned(),
            JsonValue::Array(claims.files.iter().map(cleanup_file_to_value).collect()),
        ),
        (
            "state_operation".to_owned(),
            JsonValue::String(claims.state_operation.clone()),
        ),
        (
            "state_revision".to_owned(),
            revision_state_to_value(&claims.state_revision),
        ),
        (
            "managed_config_drift".to_owned(),
            JsonValue::Bool(claims.managed_config_drift),
        ),
    ])
}

fn cleanup_file_to_value(file: &CleanupRevisionFile) -> JsonValue {
    let mut object = HashMap::new();
    object.insert("role".to_owned(), JsonValue::String(file.role.clone()));
    object.insert(
        "source_path".to_owned(),
        JsonValue::String(file.source_path.clone()),
    );
    object.insert(
        "target_path".to_owned(),
        JsonValue::String(file.target_path.clone()),
    );
    object.insert(
        "operation".to_owned(),
        JsonValue::String(file.operation.clone()),
    );
    object.insert(
        "backup_required".to_owned(),
        JsonValue::Bool(file.backup_required),
    );
    if !file.backup_source.is_empty() {
        object.insert(
            "backup_source".to_owned(),
            JsonValue::String(file.backup_source.clone()),
        );
    }
    object.insert(
        "source_revision".to_owned(),
        revision_state_to_value(&file.source_revision),
    );
    object.insert(
        "target_revision".to_owned(),
        revision_state_to_value(&file.target_revision),
    );
    object.insert(
        "removed_paths".to_owned(),
        JsonValue::Array(
            file.removed_paths
                .iter()
                .map(|value| JsonValue::String(value.clone()))
                .collect(),
        ),
    );
    JsonValue::Object(object)
}

fn cleanup_from_envelope(
    value: &JsonValue,
) -> Result<(i64, String, String, CleanupRevisionClaims), TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(
        object,
        &["version", "canonicalization", "key_generation", "claims"],
    )?;
    Ok((
        required_int(object, "version")?,
        required_string(object, "canonicalization")?,
        required_string(object, "key_generation")?,
        cleanup_claims_from_value(object.get("claims").ok_or(TokenError::Invalid)?)?,
    ))
}

fn cleanup_claims_from_value(value: &JsonValue) -> Result<CleanupRevisionClaims, TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(
        object,
        &[
            "agent",
            "files",
            "state_operation",
            "state_revision",
            "managed_config_drift",
        ],
    )?;
    Ok(CleanupRevisionClaims {
        agent: Agent::parse(&required_string(object, "agent")?).ok_or(TokenError::Invalid)?,
        files: required_array(object, "files")?
            .iter()
            .map(cleanup_file_from_value)
            .collect::<Result<_, _>>()?,
        state_operation: required_string(object, "state_operation")?,
        state_revision: revision_state_from_value(object.get("state_revision"))?,
        managed_config_drift: required_bool(object, "managed_config_drift")?,
    })
}

fn cleanup_file_from_value(value: &JsonValue) -> Result<CleanupRevisionFile, TokenError> {
    let object = value.as_object().ok_or(TokenError::Invalid)?;
    reject_unknown(
        object,
        &[
            "role",
            "source_path",
            "target_path",
            "operation",
            "backup_required",
            "backup_source",
            "source_revision",
            "target_revision",
            "removed_paths",
        ],
    )?;
    Ok(CleanupRevisionFile {
        role: required_string(object, "role")?,
        source_path: required_string(object, "source_path")?,
        target_path: required_string(object, "target_path")?,
        operation: required_string(object, "operation")?,
        backup_required: required_bool(object, "backup_required")?,
        backup_source: optional_string(object, "backup_source")?,
        source_revision: revision_state_from_value(object.get("source_revision"))?,
        target_revision: revision_state_from_value(object.get("target_revision"))?,
        removed_paths: optional_string_array(object, "removed_paths")?,
    })
}

fn number(value: i64) -> JsonValue {
    JsonValue::Number(JsonNumber(value.to_string()))
}

fn reject_unknown(object: &HashMap<String, JsonValue>, allowed: &[&str]) -> Result<(), TokenError> {
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(TokenError::Invalid)
    }
}

fn required_string(object: &HashMap<String, JsonValue>, key: &str) -> Result<String, TokenError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or(TokenError::Invalid)
}

fn optional_string(object: &HashMap<String, JsonValue>, key: &str) -> Result<String, TokenError> {
    match object.get(key) {
        None => Ok(String::new()),
        Some(value) => value
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or(TokenError::Invalid),
    }
}

fn required_int(object: &HashMap<String, JsonValue>, key: &str) -> Result<i64, TokenError> {
    object
        .get(key)
        .and_then(JsonValue::as_number)
        .and_then(|value| value.as_str().parse().ok())
        .ok_or(TokenError::Invalid)
}

fn optional_int(object: &HashMap<String, JsonValue>, key: &str) -> Result<i64, TokenError> {
    match object.get(key) {
        None => Ok(0),
        Some(value) => value
            .as_number()
            .and_then(|number| number.as_str().parse().ok())
            .ok_or(TokenError::Invalid),
    }
}

fn required_bool(object: &HashMap<String, JsonValue>, key: &str) -> Result<bool, TokenError> {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .ok_or(TokenError::Invalid)
}

fn optional_bool(object: &HashMap<String, JsonValue>, key: &str) -> Result<bool, TokenError> {
    match object.get(key) {
        None => Ok(false),
        Some(value) => value.as_bool().ok_or(TokenError::Invalid),
    }
}

fn required_array<'a>(
    object: &'a HashMap<String, JsonValue>,
    key: &str,
) -> Result<&'a [JsonValue], TokenError> {
    object
        .get(key)
        .and_then(JsonValue::as_array)
        .ok_or(TokenError::Invalid)
}

fn required_string_array(
    object: &HashMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<String>, TokenError> {
    required_array(object, key)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or(TokenError::Invalid)
        })
        .collect()
}

fn optional_string_array(
    object: &HashMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<String>, TokenError> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(value) => value
            .as_array()
            .ok_or(TokenError::Invalid)?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or(TokenError::Invalid)
            })
            .collect(),
    }
}

fn required_agents(
    object: &HashMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<Agent>, TokenError> {
    required_string_array(object, key)?
        .into_iter()
        .map(|value| Agent::parse(&value).ok_or(TokenError::Invalid))
        .collect()
}

fn optional_agents(
    object: &HashMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<Agent>, TokenError> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(_) => required_agents(object, key),
    }
}

impl From<super::types::ValidationError> for TokenError {
    fn from(_: super::types::ValidationError) -> Self {
        Self::Invalid
    }
}
