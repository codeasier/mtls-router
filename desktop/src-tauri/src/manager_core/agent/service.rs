use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use super::cleanup::{
    acquire_cleanup_preview_lock, build_cleanup_plan, cleanup_preview_for_plan,
    cleanup_token_for_plan, ensure_cleanup_state_exists, CleanupPreview,
};
use super::detect::Detector;
use super::journal::{
    execute_plan, journal_present, prepare_state_dir, recover_locked, JOURNAL_FILE_NAME,
};
use super::lock::acquire_transaction_lock;
use super::modelconfig::{
    canonical, decode, decode_structural, Agent, CatalogClaims, Config, TokenSigner,
};
use super::models::{
    discover_existing, discover_preset, ModelsExisting, ModelsPreset, ModelsResult,
};
use super::plan::{
    AgentPlan, CatalogBinding, ConfigMode, Preview, PreviewRequest, WritePlan, WriteRequest,
    WriteResult,
};
use super::preview::{
    build_plan, ordered_states, preview_for_plan, rebuild_files_eligible, token_for_plan,
    validate_rebuild_state, without_recovery_reason,
};
use super::render::{api_base_url, render_fragments, same_model_agents, RenderInput, RenderResult};
use super::revision::verify_keyed_revision;
use super::sidecar::{append_sidecar_plan, read_sidecar, sidecar_path};
use super::trust::{
    load_or_create_signing_key, load_signing_key, signer_from_key, SIGNING_KEY_FILE_NAME,
};
use super::types::{
    append_reason, CleanupState, CleanupUnavailableReason, OperationError, RecoveryReason, State,
    REDACTED_API_KEY,
};
use crate::protocol::{ErrorCode, MANAGEMENT_PROTOCOL_VERSION};

#[derive(Clone, Debug)]
pub struct Options {
    pub state_dir: PathBuf,
    pub detector: Detector,
    pub preset: Option<Config>,
    pub simplify: bool,
}

pub struct AgentService {
    inner: Mutex<ServiceInner>,
}

struct ServiceInner {
    state_dir: PathBuf,
    detector: Detector,
    signer: Option<TokenSigner>,
    key_generation: String,
    writes_disabled: bool,
    recovery_err: Option<OperationError>,
    preset: Option<Config>,
    simplify: bool,
}

impl AgentService {
    pub fn new(options: Options) -> Result<Self, OperationError> {
        if options.state_dir.as_os_str().is_empty() {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "Agent transaction state directory is required",
            ));
        }
        let mut preset = None;
        if let Some(value) = options.preset {
            let canonical = canonical(&value).map_err(|_| {
                OperationError::new(
                    ErrorCode::ModelConfigInvalid,
                    "Agent model preset is invalid",
                )
            })?;
            preset = Some(decode_structural(&canonical).map_err(|_| {
                OperationError::new(
                    ErrorCode::ModelConfigInvalid,
                    "Agent model preset is invalid",
                )
            })?);
        }
        let service = Self {
            inner: Mutex::new(ServiceInner {
                state_dir: options.state_dir,
                detector: options.detector,
                signer: None,
                key_generation: String::new(),
                writes_disabled: false,
                recovery_err: None,
                preset,
                simplify: options.simplify,
            }),
        };
        {
            let mut inner = service.inner.lock().expect("agent service lock");
            let _lock = acquire_transaction_lock(&inner.state_dir, None)?;
            if journal_present(&inner.state_dir) {
                if prepare_state_dir(&inner.state_dir).is_err()
                    || restrict_journal(&inner.state_dir).is_err()
                {
                    inner.disable_writes();
                } else {
                    let _ = inner.ensure_signer();
                    if recover_locked(
                        inner.signer.as_ref(),
                        &inner.key_generation,
                        &inner.state_dir,
                    )
                    .is_err()
                    {
                        inner.disable_writes();
                    }
                }
            }
        }
        Ok(service)
    }

    pub fn detect(&self, deadline: Option<Instant>) -> Result<Vec<State>, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        inner.detect_locked()
    }

    pub fn render(
        &self,
        selected: &[Agent],
        catalog_token: &str,
        raw_config: &[u8],
        deadline: Option<Instant>,
    ) -> Result<RenderResult, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        inner.ensure_existing_signer()?;
        let claims = inner.verify_catalog_token(catalog_token)?;
        let normalized = normalize_selection(selected)?;
        if !same_model_agents(&normalized, &claims.agents) {
            return Err(OperationError::new(
                ErrorCode::ModelCatalogStale,
                "model catalog Agent scope changed",
            ));
        }
        let config = decode(raw_config, &claims.agents, &claims.models).map_err(|error| {
            OperationError::new(
                ErrorCode::ModelConfigInvalid,
                "canonical model configuration is invalid",
            )
            .with_details(error.path, error.rule)
        })?;
        let canonical_config = canonical(&config).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelConfigInvalid,
                "canonical model configuration is invalid",
            )
        })?;
        let api = api_base_url(&claims.router_base_url)?;
        let paths = inner.detector.target_states().map_err(|_| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Agent target paths are unavailable",
            )
        })?;
        let fragments = render_fragments(
            &normalized,
            &paths,
            &RenderInput {
                config,
                router_base_url: claims.router_base_url,
                api_base_url: api,
                key: REDACTED_API_KEY.to_owned(),
                own_root_model: false,
            },
        )?;
        Ok(RenderResult {
            model_config: canonical_config,
            fragments,
        })
    }

    pub fn discover_models(
        &self,
        selected: &[Agent],
        catalog: &[String],
        mut claims: CatalogClaims,
        deadline: Option<Instant>,
    ) -> Result<ModelsResult, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        inner.ensure_signer()?;
        claims.simplify = inner.simplify;
        let signer = inner.signer.as_ref().ok_or_else(trust_invalid)?;
        let token = signer.sign_catalog(claims).map_err(|_| trust_invalid())?;
        let preset = discover_preset(inner.preset.as_ref(), selected, catalog)?;
        let states = inner.detector.detect().map_err(|_| {
            OperationError::new(
                ErrorCode::ConfigInvalid,
                "Agent configuration inspection failed",
            )
        })?;
        let existing = discover_existing(
            signer,
            &inner.state_dir,
            &inner.key_generation,
            selected,
            catalog,
            &states,
        )?;
        Ok(ModelsResult {
            catalog_token: token,
            existing,
            preset,
        })
    }

    pub fn preview(
        &self,
        request: PreviewRequest,
        deadline: Option<Instant>,
    ) -> Result<Preview, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        inner.ensure_existing_signer()?;
        let agents = normalize_agent_plans(&request.agents, &request.modes)?;
        let (claims, config, canonical) = inner.validate_v2(
            &request.agents,
            &request.catalog_token,
            &request.model_config,
        )?;
        let api = api_base_url(&claims.router_base_url).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelCatalogStale,
                "model catalog router address is invalid",
            )
        })?;
        let input = RenderInput {
            config,
            router_base_url: claims.router_base_url.clone(),
            api_base_url: api,
            key: REDACTED_API_KEY.to_owned(),
            own_root_model: true,
        };
        let plan = inner.build_current_plan(&agents, input, claims, canonical)?;
        let sidecar = sidecar_path(&inner.state_dir)
            .to_string_lossy()
            .into_owned();
        preview_for_plan(
            inner.signer.as_ref().ok_or_else(trust_invalid)?,
            &inner.detector,
            &sidecar,
            &plan,
        )
    }

    pub fn validate_preview(
        &self,
        request: &WriteRequest,
        deadline: Option<Instant>,
    ) -> Result<(), OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        inner.ensure_existing_signer()?;
        let agents = normalize_agent_plans(&request.agents, &request.modes)?;
        validate_rebuild_approval(&agents, &request.approve_rebuild)?;
        let (claims, config, canonical) = inner.validate_v2(
            &request.agents,
            &request.catalog_token,
            &request.model_config,
        )?;
        let api = api_base_url(&claims.router_base_url).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelCatalogStale,
                "model catalog router address is invalid",
            )
        })?;
        let plan = inner.build_current_plan(
            &agents,
            RenderInput {
                config,
                router_base_url: claims.router_base_url.clone(),
                api_base_url: api,
                key: REDACTED_API_KEY.to_owned(),
                own_root_model: true,
            },
            claims,
            canonical,
        )?;
        let token = token_for_plan(inner.signer.as_ref().ok_or_else(trust_invalid)?, &plan)?;
        if !constant_eq(token.as_bytes(), request.revision_token.as_bytes()) {
            return Err(OperationError::new(
                ErrorCode::PreviewStale,
                "Agent configuration changed after preview",
            ));
        }
        if (!plan.drifted.is_empty()) != request.approve_managed_overwrite {
            return Err(OperationError::new(
                ErrorCode::ManagedConfigDrift,
                "managed Agent configuration drift approval does not match preview",
            ));
        }
        if plan.requires_codex_auth_approval != request.approve_codex_auth_change {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "Codex authentication approval does not match preview",
            ));
        }
        Ok(())
    }

    pub fn catalog_binding(
        &self,
        selected: &[Agent],
        token: &str,
        raw_config: &[u8],
        deadline: Option<Instant>,
    ) -> Result<CatalogBinding, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        inner.ensure_existing_signer()?;
        let (claims, _, _) = inner.validate_v2(selected, token, raw_config)?;
        Ok(CatalogBinding {
            owner: claims.owner,
            router_base_url: claims.router_base_url,
            deployment_id: claims.deployment_id,
            protocol_version: claims.protocol_version,
            models: claims.models,
        })
    }

    pub fn write(
        &self,
        mut request: WriteRequest,
        deadline: Option<Instant>,
    ) -> Result<WriteResult, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        if inner.writes_disabled {
            return Err(OperationError::new(
                ErrorCode::RollbackFailed,
                "Agent writes are disabled because transaction recovery failed",
            ));
        }
        if request.api_key.is_empty() || request.revision_token.is_empty() {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "API key and revision token are required",
            ));
        }
        let agents = normalize_agent_plans(&request.agents, &request.modes)?;
        validate_rebuild_approval(&agents, &request.approve_rebuild)?;
        inner.ensure_signer()?;
        let (claims, config, canonical) = inner.validate_v2(
            &request.agents,
            &request.catalog_token,
            &request.model_config,
        )?;
        let api = api_base_url(&claims.router_base_url).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelCatalogStale,
                "model catalog router address is invalid",
            )
        })?;
        let mut plan = inner.build_current_plan(
            &agents,
            RenderInput {
                config,
                router_base_url: claims.router_base_url.clone(),
                api_base_url: api,
                key: request.api_key.clone(),
                own_root_model: true,
            },
            claims,
            canonical,
        )?;
        if (!plan.drifted.is_empty()) != request.approve_managed_overwrite {
            return Err(OperationError::new(
                ErrorCode::ManagedConfigDrift,
                "managed Agent configuration drift requires approval",
            ));
        }
        if plan.requires_codex_auth_approval != request.approve_codex_auth_change {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "Codex authentication approval does not match preview",
            ));
        }
        let current = token_for_plan(inner.signer.as_ref().ok_or_else(trust_invalid)?, &plan)?;
        if !constant_eq(current.as_bytes(), request.revision_token.as_bytes()) {
            return Err(OperationError::new(
                ErrorCode::PreviewStale,
                "Agent configuration changed after preview",
            ));
        }
        inner.revalidate_rebuild_agents(&plan, &HashMap::new())?;
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        let signer = inner.signer.as_ref().ok_or_else(trust_invalid)?;
        for file in &plan.files {
            verify_keyed_revision(
                signer,
                Path::new(&file.source_path),
                &file.source_revision,
                super::revision::revision_context_for(file.scope),
            )
            .map_err(|_| {
                OperationError::new(
                    ErrorCode::PreviewStale,
                    "Agent configuration changed after preview",
                )
            })?;
            if file.source_path != file.target_path {
                verify_keyed_revision(
                    signer,
                    Path::new(&file.target_path),
                    &file.target_revision,
                    super::revision::revision_context_for(file.scope),
                )
                .map_err(|_| {
                    OperationError::new(
                        ErrorCode::PreviewStale,
                        "Agent configuration target changed after preview",
                    )
                })?;
            }
        }
        plan = append_sidecar_plan(
            signer,
            &inner.state_dir,
            &inner.key_generation,
            plan,
            &request.api_key,
        )?;
        let result = execute_plan(
            signer,
            &inner.state_dir,
            &inner.key_generation,
            &mut plan,
            &request.api_key,
            deadline,
        );
        request.api_key.clear();
        result
    }

    pub fn writes_disabled(&self) -> bool {
        let inner = self.inner.lock().expect("agent service lock");
        if acquire_transaction_lock(&inner.state_dir, None).is_err() {
            return true;
        }
        inner.writes_disabled
    }

    pub fn cleanup_preview(
        &self,
        agent: Agent,
        deadline: Option<Instant>,
    ) -> Result<CleanupPreview, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        ensure_cleanup_state_exists(&inner.state_dir, agent)?;
        let _lock = acquire_cleanup_preview_lock(&inner.state_dir, deadline)?;
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        ensure_cleanup_state_exists(&inner.state_dir, agent)?;
        if inner.writes_disabled {
            return Err(writes_disabled());
        }
        inner.ensure_existing_signer()?;
        let plan = build_cleanup_plan(
            inner.signer.as_ref().ok_or_else(trust_invalid)?,
            &inner.state_dir,
            &inner.key_generation,
            agent,
        )?;
        cleanup_preview_for_plan(
            inner.signer.as_ref().ok_or_else(trust_invalid)?,
            &inner.state_dir,
            &plan,
        )
    }

    pub fn cleanup_write(
        &self,
        agent: Agent,
        revision_token: &str,
        approve_managed_overwrite: bool,
        deadline: Option<Instant>,
    ) -> Result<WriteResult, OperationError> {
        let mut inner = self.inner.lock().expect("agent service lock");
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        ensure_cleanup_state_exists(&inner.state_dir, agent)?;
        let _lock = acquire_transaction_lock(&inner.state_dir, deadline)?;
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        ensure_cleanup_state_exists(&inner.state_dir, agent)?;
        if inner.writes_disabled {
            return Err(writes_disabled());
        }
        if revision_token.is_empty() {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "cleanup revision token is required",
            ));
        }
        inner.ensure_existing_signer()?;
        let signer = inner.signer.as_ref().ok_or_else(trust_invalid)?;
        let claims = signer
            .verify_cleanup_revision(revision_token)
            .map_err(|_| {
                OperationError::new(ErrorCode::PreviewStale, "Agent cleanup preview is stale")
            })?;
        if claims.agent != agent {
            return Err(OperationError::new(
                ErrorCode::PreviewStale,
                "Agent cleanup preview is stale",
            ));
        }
        verify_keyed_revision(
            signer,
            &sidecar_path(&inner.state_dir),
            &super::plan::FileRevision {
                exists: claims.state_revision.exists,
                size: claims.state_revision.size,
                mode: claims.state_revision.mode,
                digest: claims.state_revision.digest.clone(),
            },
            super::plan::REVISION_CONTEXT_SIDECAR,
        )
        .map_err(|_| {
            OperationError::new(
                ErrorCode::PreviewStale,
                "Agent cleanup state changed after preview",
            )
        })?;
        for file in &claims.files {
            verify_keyed_revision(
                signer,
                Path::new(&file.source_path),
                &super::plan::FileRevision {
                    exists: file.source_revision.exists,
                    size: file.source_revision.size,
                    mode: file.source_revision.mode,
                    digest: file.source_revision.digest.clone(),
                },
                super::plan::REVISION_CONTEXT_AGENT_FILE,
            )
            .map_err(|_| {
                OperationError::new(
                    ErrorCode::PreviewStale,
                    "Agent configuration changed after cleanup preview",
                )
            })?;
        }
        let mut plan = build_cleanup_plan(signer, &inner.state_dir, &inner.key_generation, agent)?;
        let current = cleanup_token_for_plan(signer, &plan)?;
        if !constant_eq(current.as_bytes(), revision_token.as_bytes()) {
            return Err(OperationError::new(
                ErrorCode::PreviewStale,
                "Agent cleanup preview is stale",
            ));
        }
        if plan.managed_config_drift() != approve_managed_overwrite {
            return Err(OperationError::new(
                ErrorCode::ManagedConfigDrift,
                "managed Agent configuration drift approval does not match preview",
            ));
        }
        if deadline_exceeded(deadline) {
            return Err(timeout());
        }
        execute_plan(
            signer,
            &inner.state_dir,
            &inner.key_generation,
            &mut plan.write,
            "",
            deadline,
        )
    }
}

impl ServiceInner {
    fn detect_locked(&mut self) -> Result<Vec<State>, OperationError> {
        let mut states = self
            .detector
            .detect()
            .map_err(|_| OperationError::new(ErrorCode::ConfigInvalid, "Agent detection failed"))?;
        let mut manager_reason = None;
        if self.writes_disabled {
            manager_reason = Some(RecoveryReason::WritesDisabled);
        } else if journal_present(&self.state_dir) {
            manager_reason = Some(RecoveryReason::TransactionRecoveryPending);
        }
        if let Some(reason) = manager_reason {
            for state in &mut states {
                state.recovery.eligible = false;
                append_reason(&mut state.recovery.reasons, reason);
            }
        }
        self.annotate_cleanup(&mut states);
        Ok(states)
    }

    fn annotate_cleanup(&mut self, states: &mut [State]) {
        let sidecar = sidecar_path(&self.state_dir);
        match std::fs::metadata(&sidecar) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for state in states.iter_mut() {
                    state.cleanup = CleanupState {
                        managed: false,
                        available: false,
                        reason: Some(CleanupUnavailableReason::NotManaged),
                    };
                }
                return;
            }
            Err(_) => {
                set_cleanup_invalid(states);
                return;
            }
            Ok(_) => {}
        }
        if self.ensure_existing_signer().is_err() {
            set_cleanup_invalid(states);
            return;
        }
        let Ok(signer) = self.signer.as_ref().ok_or_else(trust_invalid) else {
            set_cleanup_invalid(states);
            return;
        };
        let Ok((state, _, _, _)) = read_sidecar(signer, &self.state_dir, &self.key_generation)
        else {
            set_cleanup_invalid(states);
            return;
        };
        let blocked = self.writes_disabled || journal_present(&self.state_dir);
        for item in states.iter_mut() {
            let managed = state.agents.contains_key(&item.agent);
            item.cleanup.managed = managed;
            if !managed {
                item.cleanup.available = false;
                item.cleanup.reason = Some(CleanupUnavailableReason::NotManaged);
            } else if blocked {
                item.cleanup.available = false;
                item.cleanup.reason = Some(CleanupUnavailableReason::WritesDisabled);
            } else {
                item.cleanup.available = true;
                item.cleanup.reason = None;
            }
        }
    }

    fn ensure_signer(&mut self) -> Result<(), OperationError> {
        let key = load_or_create_signing_key(
            &self.state_dir,
            journal_present(&self.state_dir),
            sidecar_path(&self.state_dir).exists(),
        )?;
        self.set_signer(key)
    }

    fn ensure_existing_signer(&mut self) -> Result<(), OperationError> {
        let key = load_signing_key(&self.state_dir.join(SIGNING_KEY_FILE_NAME))
            .map_err(|_| trust_invalid())?;
        self.set_signer(key)
    }

    fn set_signer(&mut self, key: super::trust::SigningKeyFile) -> Result<(), OperationError> {
        self.signer = Some(signer_from_key(&key)?);
        self.key_generation = key.generation;
        Ok(())
    }

    fn verify_catalog_token(&self, token: &str) -> Result<CatalogClaims, OperationError> {
        let signer = self.signer.as_ref().ok_or_else(trust_invalid)?;
        let claims = signer.verify_catalog(token).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelCatalogStale,
                "model catalog token is invalid",
            )
        })?;
        if claims.protocol_version != MANAGEMENT_PROTOCOL_VERSION
            || claims.simplify != self.simplify
        {
            return Err(OperationError::new(
                ErrorCode::ModelCatalogStale,
                "Agent model catalog is stale",
            ));
        }
        Ok(claims)
    }

    fn validate_v2(
        &self,
        selected: &[Agent],
        catalog_token: &str,
        raw_config: &[u8],
    ) -> Result<(CatalogClaims, Config, Vec<u8>), OperationError> {
        let claims = self.verify_catalog_token(catalog_token)?;
        let normalized = normalize_selection(selected)?;
        if !same_model_agents(&normalized, &claims.agents) {
            return Err(OperationError::new(
                ErrorCode::ModelCatalogStale,
                "model catalog Agent scope changed",
            ));
        }
        let config = decode(raw_config, &claims.agents, &claims.models).map_err(|error| {
            OperationError::new(
                ErrorCode::ModelConfigInvalid,
                "canonical model configuration is invalid",
            )
            .with_details(error.path, error.rule)
        })?;
        let canonical = canonical(&config).map_err(|_| {
            OperationError::new(
                ErrorCode::ModelConfigInvalid,
                "canonical model configuration is invalid",
            )
        })?;
        Ok((claims, config, canonical))
    }

    fn build_current_plan(
        &mut self,
        agents: &[AgentPlan],
        input: RenderInput,
        catalog: CatalogClaims,
        canonical: Vec<u8>,
    ) -> Result<WritePlan, OperationError> {
        let states = self.detect_locked().map_err(|_| {
            OperationError::new(
                ErrorCode::AgentNotFound,
                "could not detect supported Agents",
            )
        })?;
        let signer = self.signer.as_ref().ok_or_else(trust_invalid)?;
        let (sidecar, sidecar_revision, sidecar_content, sidecar_mode) =
            read_sidecar(signer, &self.state_dir, &self.key_generation)?;
        build_plan(
            signer,
            &states,
            sidecar,
            sidecar_revision,
            sidecar_content,
            sidecar_mode,
            agents,
            input,
            catalog,
            canonical,
        )
    }

    fn revalidate_rebuild_agents(
        &mut self,
        plan: &WritePlan,
        replaced: &HashMap<String, bool>,
    ) -> Result<(), OperationError> {
        let mut states = self.detect_locked().map_err(|_| {
            OperationError::new(
                ErrorCode::PreviewStale,
                "Agent recovery state changed after preview",
            )
        })?;
        if journal_present(&self.state_dir) && !self.writes_disabled {
            for state in &mut states {
                state.recovery.reasons = without_recovery_reason(
                    &state.recovery.reasons,
                    RecoveryReason::TransactionRecoveryPending,
                );
                state.recovery.eligible = rebuild_files_eligible(&state.recovery.files)
                    && state.recovery.reasons.len() == 1
                    && state.recovery.reasons[0] == RecoveryReason::SyntaxInvalid;
            }
        }
        let by_kind = ordered_states(&states);
        for item in &plan.agents {
            if item.mode != ConfigMode::Rebuild {
                continue;
            }
            let state = by_kind.get(&item.kind).ok_or_else(|| {
                OperationError::new(
                    ErrorCode::PreviewStale,
                    "Agent recovery state changed after preview",
                )
            })?;
            if replaced.is_empty() {
                validate_rebuild_state(state).map_err(|_| {
                    OperationError::new(
                        ErrorCode::PreviewStale,
                        "Agent recovery state changed after preview",
                    )
                })?;
            }
        }
        Ok(())
    }

    fn disable_writes(&mut self) {
        self.writes_disabled = true;
        self.recovery_err = Some(OperationError::new(
            ErrorCode::RollbackFailed,
            "Agent transaction recovery could not prove restoration",
        ));
    }
}

pub fn normalize_selection(selected: &[Agent]) -> Result<Vec<Agent>, OperationError> {
    if selected.is_empty() {
        return Err(OperationError::new(
            ErrorCode::InvalidParams,
            "at least one Agent must be selected",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for agent in selected {
        seen.insert(*agent);
    }
    Ok(Agent::all()
        .into_iter()
        .filter(|agent| seen.contains(agent))
        .collect())
}

pub fn normalize_agent_plans(
    selected: &[Agent],
    modes: &HashMap<Agent, ConfigMode>,
) -> Result<Vec<AgentPlan>, OperationError> {
    let normalized = normalize_selection(selected)?;
    if !modes.is_empty() && modes.len() != normalized.len() {
        return Err(OperationError::new(
            ErrorCode::InvalidParams,
            "Agent modes must exactly match selected Agents",
        ));
    }
    for (kind, mode) in modes {
        if !normalized.contains(kind)
            || (*mode != ConfigMode::Merge && *mode != ConfigMode::Rebuild)
        {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "Agent modes must exactly match selected Agents",
            ));
        }
    }
    Ok(normalized
        .into_iter()
        .map(|kind| AgentPlan {
            kind,
            mode: if modes.is_empty() {
                ConfigMode::Merge
            } else {
                modes[&kind]
            },
        })
        .collect())
}

fn validate_rebuild_approval(
    plans: &[AgentPlan],
    approval: &[Agent],
) -> Result<(), OperationError> {
    let want: std::collections::HashSet<_> = plans
        .iter()
        .filter(|plan| plan.mode == ConfigMode::Rebuild)
        .map(|plan| plan.kind)
        .collect();
    let mut got = std::collections::HashSet::new();
    for kind in approval {
        if !got.insert(*kind) || !want.contains(kind) {
            return Err(OperationError::new(
                ErrorCode::InvalidParams,
                "rebuild approval must exactly match rebuild Agents",
            ));
        }
    }
    if got.len() != want.len() {
        return Err(OperationError::new(
            ErrorCode::InvalidParams,
            "rebuild approval must exactly match rebuild Agents",
        ));
    }
    Ok(())
}

fn restrict_journal(state_dir: &Path) -> Result<(), OperationError> {
    super::files::restrict_private(&state_dir.join(JOURNAL_FILE_NAME), false).map_err(|_| {
        OperationError::new(
            ErrorCode::RollbackFailed,
            "Agent transaction recovery could not prove restoration",
        )
    })
}

fn set_cleanup_invalid(states: &mut [State]) {
    for state in states {
        state.cleanup = CleanupState {
            managed: false,
            available: false,
            reason: Some(CleanupUnavailableReason::ModelStateInvalid),
        };
    }
}

fn deadline_exceeded(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|value| Instant::now() >= value)
}

fn timeout() -> OperationError {
    OperationError::new(
        ErrorCode::OperationTimeout,
        "Agent operation deadline exceeded",
    )
}

fn trust_invalid() -> OperationError {
    OperationError::new(
        ErrorCode::ModelStateInvalid,
        "Agent model trust state is invalid",
    )
}

fn writes_disabled() -> OperationError {
    OperationError::new(
        ErrorCode::RollbackFailed,
        "Agent writes are disabled because transaction recovery failed",
    )
}

fn constant_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

pub fn isolated_service() -> AgentService {
    let home = std::env::temp_dir().join(format!(
        "mtls-manager-agent-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let _ = std::fs::create_dir_all(&home);
    AgentService::new(Options {
        state_dir: home.join("state"),
        detector: Detector::isolated(home),
        preset: None,
        simplify: false,
    })
    .expect("isolated agent service")
}

pub fn validate_refreshed_models(
    selected: &[Agent],
    raw_config: &[u8],
    refreshed: &[String],
) -> Result<(), OperationError> {
    decode(raw_config, selected, refreshed).map_err(|_| {
        OperationError::new(
            ErrorCode::ModelNotAvailable,
            "a selected model is no longer available",
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MANAGEMENT_PROTOCOL_VERSION;

    fn claude_config() -> Vec<u8> {
        br#"{"claude":{"haiku":{"inherit_primary":true},"opus":{"inherit_primary":true},"primary":{"model":"model-a"},"sonnet":{"inherit_primary":true}},"version":1}"#.to_vec()
    }

    fn catalog_claims(agents: &[Agent]) -> CatalogClaims {
        CatalogClaims {
            version: 1,
            models: vec!["model-a".into()],
            agents: agents.to_vec(),
            owner: "desktop".into(),
            router_base_url: "http://127.0.0.1:19099".into(),
            deployment_id: "test-deploy".into(),
            protocol_version: MANAGEMENT_PROTOCOL_VERSION.into(),
            simplify: false,
            canonicalization: String::new(),
            key_generation: String::new(),
        }
    }

    #[test]
    fn detect_returns_three_isolated_agents() {
        let service = isolated_service();
        let states = service.detect(None).unwrap();
        assert_eq!(states.len(), 3);
        assert!(states
            .iter()
            .all(|state| state.detected && state.command.is_empty()));
        assert!(states.iter().all(|state| !state.cleanup.managed));
        assert!(states
            .iter()
            .all(|state| state.cleanup.reason == Some(CleanupUnavailableReason::NotManaged)));
    }

    #[test]
    fn preview_write_cleanup_claude_round_trip() {
        let home =
            std::env::temp_dir().join(format!("mtls-agent-tx-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&home).unwrap();
        let service = AgentService::new(Options {
            state_dir: home.join("state"),
            detector: Detector::isolated(&home),
            preset: None,
            simplify: false,
        })
        .unwrap();
        let discovered = service
            .discover_models(
                &[Agent::Claude],
                &["model-a".into()],
                catalog_claims(&[Agent::Claude]),
                None,
            )
            .unwrap();
        let raw = claude_config();
        let preview = service
            .preview(
                PreviewRequest {
                    agents: vec![Agent::Claude],
                    modes: HashMap::new(),
                    catalog_token: discovered.catalog_token.clone(),
                    model_config: raw.clone(),
                },
                None,
            )
            .unwrap();
        assert!(!preview.revision_token.is_empty());
        assert_eq!(preview.agents.len(), 1);
        let written = service
            .write(
                WriteRequest {
                    agents: vec![Agent::Claude],
                    modes: HashMap::new(),
                    approve_rebuild: Vec::new(),
                    revision_token: preview.revision_token,
                    api_key: "secret-key".into(),
                    catalog_token: discovered.catalog_token.clone(),
                    model_config: raw,
                    approve_managed_overwrite: preview.managed_config_drift,
                    approve_codex_auth_change: preview.requires_codex_auth_approval,
                },
                None,
            )
            .unwrap();
        assert!(written.agents.iter().all(|agent| agent.success));
        let states = service.detect(None).unwrap();
        let claude = states
            .iter()
            .find(|state| state.agent == Agent::Claude)
            .unwrap();
        assert!(claude.cleanup.managed);
        assert!(claude.cleanup.available);
        let cleanup = service.cleanup_preview(Agent::Claude, None).unwrap();
        let cleaned = service
            .cleanup_write(
                Agent::Claude,
                &cleanup.revision_token,
                cleanup.managed_config_drift,
                None,
            )
            .unwrap();
        assert!(cleaned.agents.iter().all(|agent| agent.success));
        let after = service.detect(None).unwrap();
        assert!(
            !after
                .iter()
                .find(|state| state.agent == Agent::Claude)
                .unwrap()
                .cleanup
                .managed
        );
        let _ = std::fs::remove_dir_all(home);
    }
}
