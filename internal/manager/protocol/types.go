// Package protocol defines the private stdin/stdout contract used by
// mtls-router-manager clients.
package protocol

import (
	"encoding/json"
	"time"
)

// Method is a manager protocol operation.
type Method string

const (
	MethodManagerInfo                  Method = "manager.info"
	MethodDiagnosticsCollect           Method = "diagnostics.collect"
	MethodRouterStatus                 Method = "router.status"
	MethodRouterStart                  Method = "router.start"
	MethodRouterMigrateLegacy          Method = "router.migrate_legacy"
	MethodRouterStop                   Method = "router.stop"
	MethodRouterHealth                 Method = "router.health"
	MethodRouterVersion                Method = "router.version"
	MethodRouterLogs                   Method = "router.logs"
	MethodRouterInspectOccupant        Method = "router.inspect_occupant"
	MethodRouterForceTerminateOccupant Method = "router.force_terminate_occupant"
	MethodAgentDetect                  Method = "agent.detect"
	MethodAgentModels                  Method = "agent.models"
	MethodAgentRender                  Method = "agent.render"
	MethodAgentPreview                 Method = "agent.preview"
	MethodAgentWrite                   Method = "agent.write"
	MethodAgentCleanupPreview          Method = "agent.cleanup.preview"
	MethodAgentCleanupWrite            Method = "agent.cleanup.write"
	MethodAPIKeyUsage                  Method = "apikey.usage"
)

// ErrorCode is stable and intended for branching. Messages are diagnostic
// only and may change or be localized by a client.
type ErrorCode string

const (
	CodeInvalidRequest              ErrorCode = "INVALID_REQUEST"
	CodeUnknownMethod               ErrorCode = "UNKNOWN_METHOD"
	CodeInvalidParams               ErrorCode = "INVALID_PARAMS"
	CodeSidecarMissing              ErrorCode = "SIDECAR_MISSING"
	CodeSidecarInvalid              ErrorCode = "SIDECAR_INVALID"
	CodeRouterNotFound              ErrorCode = "ROUTER_NOT_FOUND"
	CodeRouterAlreadyRunning        ErrorCode = "ROUTER_ALREADY_RUNNING"
	CodeRouterStartFailed           ErrorCode = "ROUTER_START_FAILED"
	CodeRouterNotReady              ErrorCode = "ROUTER_NOT_READY"
	CodeRouterDegraded              ErrorCode = "ROUTER_DEGRADED"
	CodeRouterNotOwned              ErrorCode = "ROUTER_NOT_OWNED"
	CodeRouterStateStale            ErrorCode = "ROUTER_STATE_STALE"
	CodeRouterLegacyManaged         ErrorCode = "ROUTER_LEGACY_MANAGED"
	CodePortOccupied                ErrorCode = "PORT_OCCUPIED"
	CodeAgentNotFound               ErrorCode = "AGENT_NOT_FOUND"
	CodeConfigInvalid               ErrorCode = "CONFIG_INVALID"
	CodeConfigNotWritable           ErrorCode = "CONFIG_NOT_WRITABLE"
	CodePreviewStale                ErrorCode = "PREVIEW_STALE"
	CodeBackupFailed                ErrorCode = "BACKUP_FAILED"
	CodeWriteFailed                 ErrorCode = "WRITE_FAILED"
	CodeRollbackFailed              ErrorCode = "ROLLBACK_FAILED"
	CodeOperationTimeout            ErrorCode = "OPERATION_TIMEOUT"
	CodeOccupantNotFound            ErrorCode = "OCCUPANT_NOT_FOUND"
	CodeOccupantNotOwned            ErrorCode = "OCCUPANT_NOT_OWNED"
	CodeOccupantIdentityUnavailable ErrorCode = "OCCUPANT_IDENTITY_UNAVAILABLE"
	CodeOccupantChanged             ErrorCode = "OCCUPANT_CHANGED"
	CodeOccupantProtected           ErrorCode = "OCCUPANT_PROTECTED"
	CodeOccupantPermissionDenied    ErrorCode = "OCCUPANT_PERMISSION_DENIED"
	CodeOccupantTerminationFailed   ErrorCode = "OCCUPANT_TERMINATION_FAILED"
	CodePortReleaseTimeout          ErrorCode = "PORT_RELEASE_TIMEOUT"
	CodeConfirmationExpired         ErrorCode = "CONFIRMATION_EXPIRED"
	CodeModelAuthFailed             ErrorCode = "MODEL_AUTH_FAILED"
	CodeModelDiscoveryFailed        ErrorCode = "MODEL_DISCOVERY_FAILED"
	CodeModelResponseInvalid        ErrorCode = "MODEL_RESPONSE_INVALID"
	CodeModelCatalogEmpty           ErrorCode = "MODEL_CATALOG_EMPTY"
	CodeModelCatalogStale           ErrorCode = "MODEL_CATALOG_STALE"
	CodeModelConfigInvalid          ErrorCode = "MODEL_CONFIG_INVALID"
	CodeModelNotAvailable           ErrorCode = "MODEL_NOT_AVAILABLE"
	CodeManagedConfigDrift          ErrorCode = "MANAGED_CONFIG_DRIFT"
	CodeModelStateInvalid           ErrorCode = "MODEL_STATE_INVALID"
	CodeAgentOperationBusy          ErrorCode = "AGENT_OPERATION_BUSY"
	CodeCodexAuthUnsupported        ErrorCode = "CODEX_AUTH_UNSUPPORTED"
	CodeAgentNotManaged             ErrorCode = "AGENT_NOT_MANAGED"
	CodeUsageAuthFailed             ErrorCode = "USAGE_AUTH_FAILED"
	CodeUsageUnavailable            ErrorCode = "USAGE_UNAVAILABLE"
	CodeUsageRequestFailed          ErrorCode = "USAGE_REQUEST_FAILED"
	CodeUsageResponseInvalid        ErrorCode = "USAGE_RESPONSE_INVALID"
)

// Request is one newline-delimited manager request. Params is method-specific.
type Request struct {
	ID     string          `json:"id"`
	Method Method          `json:"method"`
	Params json.RawMessage `json:"params,omitempty"`
}

// Response is one newline-delimited manager response. Exactly one of Result
// and Error is present. ID is nil when no valid request ID can be recovered.
type Response struct {
	ID     *string         `json:"id"`
	Result json.RawMessage `json:"result,omitempty"`
	Error  *Error          `json:"error,omitempty"`
}

// Error is a sanitized protocol error.
type Error struct {
	Code    ErrorCode     `json:"code"`
	Message string        `json:"message"`
	Details *ErrorDetails `json:"details,omitempty"`
}

// ErrorDetails identifies one validation failure without echoing the rejected
// value. It is omitted for non-validation errors.
type ErrorDetails struct {
	Path string `json:"path"`
	Rule string `json:"rule"`
}

// Deadlines returns the required internal deadline for every protocol method.
func Deadlines() map[Method]time.Duration {
	return map[Method]time.Duration{
		MethodManagerInfo:                  time.Second,
		MethodDiagnosticsCollect:           5 * time.Second,
		MethodRouterStatus:                 time.Second,
		MethodRouterStart:                  20 * time.Second,
		MethodRouterMigrateLegacy:          27 * time.Second,
		MethodRouterStop:                   7 * time.Second,
		MethodRouterHealth:                 12 * time.Second,
		MethodRouterVersion:                time.Second,
		MethodRouterLogs:                   2 * time.Second,
		MethodRouterInspectOccupant:        2 * time.Second,
		MethodRouterForceTerminateOccupant: 3 * time.Second,
		MethodAgentDetect:                  5 * time.Second,
		MethodAgentModels:                  30 * time.Second,
		MethodAgentRender:                  5 * time.Second,
		MethodAgentPreview:                 5 * time.Second,
		MethodAgentWrite:                   30 * time.Second,
		MethodAgentCleanupPreview:          5 * time.Second,
		MethodAgentCleanupWrite:            30 * time.Second,
		MethodAPIKeyUsage:                  25 * time.Second,
	}
}

// RouterOwner selects router lifecycle ownership behavior.
type RouterOwner string

const (
	RouterOwnerDesktop RouterOwner = "desktop"
	RouterOwnerCLI     RouterOwner = "cli"
)

type RouterStartParams struct {
	Owner RouterOwner `json:"owner"`
}

type RouterLogsParams struct {
	Limit int `json:"limit,omitempty"`
}

type RouterForceTerminateOccupantParams struct {
	ConfirmationToken string `json:"confirmation_token"`
}

type AgentModelsParams struct {
	Owner  RouterOwner `json:"owner"`
	Agents []string    `json:"agents"`
	APIKey string      `json:"api_key"`
}

type APIKeyUsageParams struct {
	Owner  RouterOwner `json:"owner"`
	Period string      `json:"period,omitempty"`
	APIKey string      `json:"api_key"`
}

type APIKeyUsageSummary struct {
	Requests         int64   `json:"requests"`
	PromptTokens     int64   `json:"prompt_tokens"`
	CompletionTokens int64   `json:"completion_tokens"`
	Cost             float64 `json:"cost"`
}

type APIKeyUsageQuota struct {
	Used     float64  `json:"used"`
	Limit    *float64 `json:"limit"`
	Unit     string   `json:"unit"`
	ResetsAt string   `json:"resets_at,omitempty"`
}

type APIKeyUsageModel struct {
	Model            string  `json:"model"`
	Requests         int64   `json:"requests"`
	PromptTokens     int64   `json:"prompt_tokens"`
	CompletionTokens int64   `json:"completion_tokens"`
	Cost             float64 `json:"cost"`
}

type APIKeyUsageResult struct {
	Period  string             `json:"period"`
	AsOf    string             `json:"as_of,omitempty"`
	Summary APIKeyUsageSummary `json:"summary"`
	Quota   *APIKeyUsageQuota  `json:"quota,omitempty"`
	ByModel []APIKeyUsageModel `json:"by_model"`
}

type AgentConfigParams struct {
	Agents       []string          `json:"agents"`
	Modes        map[string]string `json:"modes,omitempty"`
	CatalogToken string            `json:"catalog_token"`
	ModelConfig  json.RawMessage   `json:"model_config"`
}

type AgentWriteParams struct {
	Agents                  []string          `json:"agents"`
	Modes                   map[string]string `json:"modes,omitempty"`
	ApproveRebuild          []string          `json:"approve_rebuild,omitempty"`
	CatalogToken            string            `json:"catalog_token"`
	ModelConfig             json.RawMessage   `json:"model_config"`
	RevisionToken           string            `json:"revision_token"`
	ApproveManagedOverwrite *bool             `json:"approve_managed_overwrite"`
	ApproveCodexAuthChange  *bool             `json:"approve_codex_auth_change"`
	APIKey                  string            `json:"api_key"`
}

type AgentCleanupParams struct {
	Agent string `json:"agent"`
}

type AgentCleanupWriteParams struct {
	Agent                   string `json:"agent"`
	RevisionToken           string `json:"revision_token"`
	ApproveManagedOverwrite *bool  `json:"approve_managed_overwrite"`
}

type ManagerInfoResult struct {
	Version                   string `json:"version"`
	Commit                    string `json:"commit"`
	BuildDate                 string `json:"build_date"`
	Target                    string `json:"target"`
	DeploymentID              string `json:"deployment_id"`
	ManagementProtocolVersion string `json:"management_protocol_version"`
}

type ManagerFailure struct {
	Stage string `json:"stage"`
	Code  string `json:"code"`
}

type DiagnosticsResult struct {
	Summary        string          `json:"summary"`
	RouterState    string          `json:"router_state,omitempty"`
	ManagerFailure *ManagerFailure `json:"manager_failure,omitempty"`
}

type RouterStatusResult struct {
	State      string   `json:"state"`
	Owner      string   `json:"owner,omitempty"`
	ListenAddr string   `json:"listen_addr,omitempty"`
	ProcessID  int      `json:"pid,omitempty"`
	LastError  string   `json:"last_error,omitempty"`
	RecentLogs []string `json:"recent_logs,omitempty"`
}

type RouterHealthResult struct {
	Status    string    `json:"status"`
	CheckedAt time.Time `json:"checked_at"`
}

type RouterVersionResult struct {
	Version                   string `json:"version"`
	DeploymentID              string `json:"deployment_id"`
	ManagementProtocolVersion string `json:"management_protocol_version"`
}

type RouterLogsResult struct {
	Lines []string `json:"lines"`
}

type RouterOccupantInspectionResult struct {
	PID               int                       `json:"pid"`
	VerificationMode  string                    `json:"verification_mode"`
	ProcessName       string                    `json:"process_name,omitempty"`
	Executable        string                    `json:"executable,omitempty"`
	ListenAddr        string                    `json:"listen_addr"`
	Recovery          RouterOccupantRecovery    `json:"recovery"`
	Supervisor        *RouterOccupantSupervisor `json:"supervisor,omitempty"`
	ConfirmationToken string                    `json:"confirmation_token,omitempty"`
	ExpiresAt         *time.Time                `json:"expires_at,omitempty"`
}

type RouterOccupantRecovery struct {
	Action string `json:"action"`
	Reason string `json:"reason,omitempty"`
}

type RouterOccupantSupervisor struct {
	Kind        string   `json:"kind"`
	Scope       string   `json:"scope"`
	Identifiers []string `json:"identifiers"`
}

type RouterOccupantTerminationResult struct {
	Termination string `json:"termination"`
	PortState   string `json:"port_state"`
}

type AgentState struct {
	Agent      string             `json:"agent"`
	Name       string             `json:"name"`
	Detected   bool               `json:"detected"`
	Command    string             `json:"command"`
	Path       string             `json:"path"`
	AuthPath   string             `json:"auth_path,omitempty"`
	Format     string             `json:"format"`
	Exists     bool               `json:"exists"`
	Writable   bool               `json:"writable"`
	Configured bool               `json:"configured"`
	Invalid    bool               `json:"invalid"`
	Migratable bool               `json:"migratable,omitempty"`
	Recovery   AgentRecoveryState `json:"recovery"`
	Cleanup    AgentCleanupState  `json:"cleanup"`
}

type AgentCleanupState struct {
	Managed   bool   `json:"managed"`
	Available bool   `json:"available"`
	Reason    string `json:"reason,omitempty"`
}

type AgentRecoveryFileState struct {
	Role    string   `json:"role"`
	Path    string   `json:"path"`
	Format  string   `json:"format"`
	Exists  bool     `json:"exists"`
	Reasons []string `json:"reasons,omitempty"`
}

type AgentRecoveryState struct {
	Eligible bool                     `json:"eligible"`
	Reasons  []string                 `json:"reasons,omitempty"`
	Files    []AgentRecoveryFileState `json:"files"`
}

type AgentDetectResult struct {
	Agents []AgentState `json:"agents"`
}

type AgentModelsExisting struct {
	ModelConfig       json.RawMessage     `json:"model_config"`
	UnavailableModels map[string][]string `json:"unavailable_models"`
	DriftedAgents     []string            `json:"drifted_agents"`
}

type AgentPresetUnavailable struct {
	Code   ErrorCode `json:"code"`
	Models []string  `json:"models"`
}

type AgentModelsPreset struct {
	ModelConfig       json.RawMessage                   `json:"model_config"`
	UnavailableAgents map[string]AgentPresetUnavailable `json:"unavailable_agents"`
}

type AgentModelsResult struct {
	Models        []string            `json:"models"`
	CatalogToken  string              `json:"catalog_token"`
	RouterBaseURL string              `json:"router_base_url"`
	APIBaseURL    string              `json:"api_base_url"`
	Existing      AgentModelsExisting `json:"existing"`
	Preset        AgentModelsPreset   `json:"preset"`
}

type AgentFragment struct {
	Agent   string `json:"agent"`
	Role    string `json:"role"`
	Path    string `json:"path"`
	Format  string `json:"format"`
	Content string `json:"content"`
}

type AgentRenderResult struct {
	ModelConfig json.RawMessage `json:"model_config"`
	Fragments   []AgentFragment `json:"fragments"`
}

type AgentFileEffect struct {
	Agent           string   `json:"agent,omitempty"`
	Mode            string   `json:"mode,omitempty"`
	Path            string   `json:"path"`
	Role            string   `json:"role"`
	Format          string   `json:"format"`
	Operation       string   `json:"operation"`
	BackupPath      string   `json:"backup_path,omitempty"`
	BackupRequired  bool     `json:"backup_required,omitempty"`
	BackupPattern   string   `json:"backup_pattern,omitempty"`
	BackupSensitive bool     `json:"backup_sensitive,omitempty"`
	Preserves       []string `json:"preserves,omitempty"`
	Warning         string   `json:"warning,omitempty"`
}

type ManagedCollision struct {
	Agent  string `json:"agent"`
	Path   string `json:"path"`
	Type   string `json:"type"`
	Action string `json:"action"`
}

type AgentPreviewResult struct {
	RevisionToken             string             `json:"revision_token"`
	ModelConfig               json.RawMessage    `json:"model_config"`
	Fragments                 []AgentFragment    `json:"fragments"`
	Files                     []AgentFileEffect  `json:"files"`
	ManagedConfigDrift        bool               `json:"managed_config_drift"`
	DriftedAgents             []string           `json:"drifted_agents"`
	ManagedCollisions         []ManagedCollision `json:"managed_collisions"`
	RequiresCodexAuthApproval bool               `json:"requires_codex_auth_approval"`
	StateChange               *AgentFileEffect   `json:"state_change,omitempty"`
	StateBackup               *AgentFileEffect   `json:"state_backup,omitempty"`
}

type AgentCleanupPreviewResult struct {
	RevisionToken      string            `json:"revision_token"`
	Agent              string            `json:"agent"`
	Files              []AgentFileEffect `json:"files"`
	RemovedPaths       []string          `json:"removed_paths"`
	ManagedConfigDrift bool              `json:"managed_config_drift"`
	StateChange        *AgentFileEffect  `json:"state_change,omitempty"`
	StateBackup        *AgentFileEffect  `json:"state_backup,omitempty"`
}

type AgentWriteStatus struct {
	Agent     string    `json:"agent"`
	Success   bool      `json:"success"`
	Changed   []string  `json:"changed,omitempty"`
	Backups   []string  `json:"backups,omitempty"`
	ErrorCode ErrorCode `json:"error_code,omitempty"`
}

type AgentWriteResult struct {
	TransactionID string             `json:"transaction_id"`
	Agents        []AgentWriteStatus `json:"agents"`
	StateChange   *AgentFileEffect   `json:"state_change,omitempty"`
	StateBackup   *AgentFileEffect   `json:"state_backup,omitempty"`
}
