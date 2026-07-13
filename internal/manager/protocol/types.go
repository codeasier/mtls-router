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
	MethodManagerInfo        Method = "manager.info"
	MethodDiagnosticsCollect Method = "diagnostics.collect"
	MethodRouterStatus       Method = "router.status"
	MethodRouterStart        Method = "router.start"
	MethodRouterStop         Method = "router.stop"
	MethodRouterHealth       Method = "router.health"
	MethodRouterVersion      Method = "router.version"
	MethodRouterLogs         Method = "router.logs"
	MethodAgentDetect        Method = "agent.detect"
	MethodAgentPreview       Method = "agent.preview"
	MethodAgentWrite         Method = "agent.write"
)

// ErrorCode is stable and intended for branching. Messages are diagnostic
// only and may change or be localized by a client.
type ErrorCode string

const (
	CodeInvalidRequest       ErrorCode = "INVALID_REQUEST"
	CodeUnknownMethod        ErrorCode = "UNKNOWN_METHOD"
	CodeInvalidParams        ErrorCode = "INVALID_PARAMS"
	CodeSidecarMissing       ErrorCode = "SIDECAR_MISSING"
	CodeSidecarInvalid       ErrorCode = "SIDECAR_INVALID"
	CodeRouterNotFound       ErrorCode = "ROUTER_NOT_FOUND"
	CodeRouterAlreadyRunning ErrorCode = "ROUTER_ALREADY_RUNNING"
	CodeRouterStartFailed    ErrorCode = "ROUTER_START_FAILED"
	CodeRouterNotReady       ErrorCode = "ROUTER_NOT_READY"
	CodeRouterDegraded       ErrorCode = "ROUTER_DEGRADED"
	CodeRouterNotOwned       ErrorCode = "ROUTER_NOT_OWNED"
	CodeRouterStateStale     ErrorCode = "ROUTER_STATE_STALE"
	CodePortOccupied         ErrorCode = "PORT_OCCUPIED"
	CodeAgentNotFound        ErrorCode = "AGENT_NOT_FOUND"
	CodeConfigInvalid        ErrorCode = "CONFIG_INVALID"
	CodeConfigNotWritable    ErrorCode = "CONFIG_NOT_WRITABLE"
	CodePreviewStale         ErrorCode = "PREVIEW_STALE"
	CodeBackupFailed         ErrorCode = "BACKUP_FAILED"
	CodeWriteFailed          ErrorCode = "WRITE_FAILED"
	CodeRollbackFailed       ErrorCode = "ROLLBACK_FAILED"
	CodeOperationTimeout     ErrorCode = "OPERATION_TIMEOUT"
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
	Code    ErrorCode `json:"code"`
	Message string    `json:"message"`
}

// Deadlines returns the required internal deadline for every protocol method.
func Deadlines() map[Method]time.Duration {
	return map[Method]time.Duration{
		MethodManagerInfo:        time.Second,
		MethodDiagnosticsCollect: 5 * time.Second,
		MethodRouterStatus:       time.Second,
		MethodRouterStart:        20 * time.Second,
		MethodRouterStop:         7 * time.Second,
		MethodRouterHealth:       5 * time.Second,
		MethodRouterVersion:      time.Second,
		MethodRouterLogs:         2 * time.Second,
		MethodAgentDetect:        5 * time.Second,
		MethodAgentPreview:       5 * time.Second,
		MethodAgentWrite:         30 * time.Second,
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

type AgentSelection struct {
	Agents []string `json:"agents"`
}

type AgentWriteParams struct {
	Agents        []string `json:"agents"`
	RevisionToken string   `json:"revision_token"`
	APIKey        string   `json:"api_key"`
}

type ManagerInfoResult struct {
	Version                   string `json:"version"`
	Commit                    string `json:"commit"`
	BuildDate                 string `json:"build_date"`
	Target                    string `json:"target"`
	DeploymentID              string `json:"deployment_id"`
	ManagementProtocolVersion string `json:"management_protocol_version"`
}

type DiagnosticsResult struct {
	Summary string `json:"summary"`
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

type AgentState struct {
	Agent      string `json:"agent"`
	Name       string `json:"name"`
	Detected   bool   `json:"detected"`
	Command    string `json:"command,omitempty"`
	Path       string `json:"path"`
	AuthPath   string `json:"auth_path,omitempty"`
	Format     string `json:"format"`
	Exists     bool   `json:"exists"`
	Writable   bool   `json:"writable"`
	Configured bool   `json:"configured"`
	Invalid    bool   `json:"invalid"`
}

type AgentDetectResult struct {
	Agents []AgentState `json:"agents"`
}

type AgentChange struct {
	Agent      string `json:"agent"`
	Path       string `json:"path"`
	Operation  string `json:"operation"`
	BackupPath string `json:"backup_path,omitempty"`
	Warning    string `json:"warning,omitempty"`
}

type AgentPreviewResult struct {
	RevisionToken string        `json:"revision_token"`
	Changes       []AgentChange `json:"changes"`
}

type AgentWriteResult struct {
	Agents []AgentWriteStatus `json:"agents"`
}

type AgentWriteStatus struct {
	Agent     string    `json:"agent"`
	Success   bool      `json:"success"`
	Changed   []string  `json:"changed,omitempty"`
	Backups   []string  `json:"backups,omitempty"`
	ErrorCode ErrorCode `json:"error_code,omitempty"`
}
