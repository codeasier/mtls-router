package agent

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const journalFileName = "agent-write-journal.json"

// ErrorCode identifies an Agent operation failure without requiring callers to
// branch on localized or platform-specific error text.
type ErrorCode string

const (
	CodeInvalidParams        ErrorCode = "INVALID_PARAMS"
	CodeAgentNotFound        ErrorCode = "AGENT_NOT_FOUND"
	CodeConfigInvalid        ErrorCode = "CONFIG_INVALID"
	CodeConfigNotWritable    ErrorCode = "CONFIG_NOT_WRITABLE"
	CodePreviewStale         ErrorCode = "PREVIEW_STALE"
	CodeBackupFailed         ErrorCode = "BACKUP_FAILED"
	CodeWriteFailed          ErrorCode = "WRITE_FAILED"
	CodeRollbackFailed       ErrorCode = "ROLLBACK_FAILED"
	CodeOperationTimeout     ErrorCode = "OPERATION_TIMEOUT"
	CodeOperationBusy        ErrorCode = "AGENT_OPERATION_BUSY"
	CodeModelStateInvalid    ErrorCode = "MODEL_STATE_INVALID"
	CodeModelCatalogStale    ErrorCode = "MODEL_CATALOG_STALE"
	CodeModelConfigInvalid   ErrorCode = "MODEL_CONFIG_INVALID"
	CodeModelNotAvailable    ErrorCode = "MODEL_NOT_AVAILABLE"
	CodeManagedConfigDrift   ErrorCode = "MANAGED_CONFIG_DRIFT"
	CodeCodexAuthUnsupported ErrorCode = "CODEX_AUTH_UNSUPPORTED"
)

// OperationError is safe to return through the management protocol. It never
// includes configuration contents or an API key.
type OperationError struct {
	Code  ErrorCode
	msg   string
	cause error
}

// CatalogBinding verifies a catalog token without reading Agent files.
type CatalogBinding struct {
	Owner           string
	RouterBaseURL   string
	DeploymentID    string
	ProtocolVersion string
	Models          []string
}

func (s *Service) CatalogBinding(ctx context.Context, selected []Kind, token string, rawConfig json.RawMessage) (CatalogBinding, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return CatalogBinding{}, err
	}
	defer lock.release()
	if err := s.ensureExistingSigner(); err != nil {
		return CatalogBinding{}, err
	}
	claims, _, _, err := s.validateV2(selected, token, rawConfig)
	if err != nil {
		return CatalogBinding{}, err
	}
	return CatalogBinding{Owner: claims.Owner, RouterBaseURL: claims.RouterBaseURL, DeploymentID: claims.DeploymentID, ProtocolVersion: claims.ProtocolVersion, Models: append([]string(nil), claims.Models...)}, nil
}

// ValidatePreview proves the revision and exact approval state without creating
// backups, journals, temporary files, or target directories. Write repeats the
// same checks under its transaction lock after catalog refresh.
func (s *Service) ValidatePreview(ctx context.Context, request WriteRequest) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return err
	}
	defer lock.release()
	if err := s.ensureExistingSigner(); err != nil {
		return err
	}
	claims, config, canonical, err := s.validateV2(request.Agents, request.CatalogToken, request.ModelConfig)
	if err != nil {
		return err
	}
	apiBaseURL, err := apiURL(claims.RouterBaseURL)
	if err != nil {
		return operationError(CodeModelCatalogStale, "model catalog router address is invalid")
	}
	s.currentInput = renderInput{config: config, routerBaseURL: claims.RouterBaseURL, apiBaseURL: apiBaseURL, ownRootModel: true}
	defer func() { s.currentInput = renderInput{} }()
	plan, err := s.buildPlan(request.Agents, s.currentInput, claims, canonical)
	if err != nil {
		return err
	}
	token, err := s.tokenForPlan(plan)
	if err != nil || subtle.ConstantTimeCompare([]byte(token), []byte(request.RevisionToken)) != 1 {
		return operationError(CodePreviewStale, "Agent configuration changed after preview")
	}
	if (len(plan.drifted) != 0) != request.ApproveManagedOverwrite {
		return operationError(CodeManagedConfigDrift, "managed Agent configuration drift approval does not match preview")
	}
	if plan.requiresCodexAuthApproval != request.ApproveCodexAuthChange {
		return operationError(CodeInvalidParams, "Codex authentication approval does not match preview")
	}
	return nil
}

// ValidateRefreshedModels requires every canonical selection in the current
// write request to remain in the refreshed authenticated catalog.
func ValidateRefreshedModels(selected []Kind, rawConfig json.RawMessage, refreshed []string) error {
	agents := kindsToModelAgents(selected)
	if _, err := modelconfig.Decode(rawConfig, agents, refreshed); err != nil {
		return operationError(CodeModelNotAvailable, "a selected model is no longer available")
	}
	return nil
}

func (e *OperationError) Error() string { return e.msg }

func (e *OperationError) Unwrap() error { return e.cause }

func operationError(code ErrorCode, message string) error {
	return &OperationError{Code: code, msg: message}
}

func operationErrorWithCause(code ErrorCode, message string, cause error) error {
	return &OperationError{Code: code, msg: message, cause: cause}
}

// CodeOf extracts a stable error code from an Agent operation error.
func CodeOf(err error) ErrorCode {
	var operationErr *OperationError
	if errors.As(err, &operationErr) {
		return operationErr.Code
	}
	return ""
}

// Operation describes how a path participates in an approved change.
type Operation string

const (
	OperationCreate   Operation = "create"
	OperationReplace  Operation = "replace"
	OperationPreserve Operation = "preserve"
)

// BackupPlan describes a potential sensitive backup without reserving a path
// or writing anything during preview.
type BackupPlan struct {
	Required  bool   `json:"required"`
	Pattern   string `json:"pattern,omitempty"`
	Sensitive bool   `json:"sensitive"`
	Warning   string `json:"warning,omitempty"`
}

// FilePreview is a typed, key-free description of one affected path.
type FilePreview struct {
	Path           string      `json:"path"`
	SourcePath     string      `json:"source_path,omitempty"`
	Format         Format      `json:"format"`
	Operation      Operation   `json:"operation"`
	Operations     []Operation `json:"operations"`
	ContainsAPIKey bool        `json:"contains_api_key"`
	Preserves      []string    `json:"preserves,omitempty"`
	Backup         BackupPlan  `json:"backup"`
	Warning        string      `json:"warning,omitempty"`
}

// AgentPreview groups all affected files for one selected Agent.
type AgentPreview struct {
	Agent    Kind          `json:"agent"`
	Name     string        `json:"name"`
	Files    []FilePreview `json:"files"`
	Warnings []string      `json:"warnings,omitempty"`
}

// ManagedCollision describes an exact manager-owned path that preview proposes
// to replace, without exposing its current value.
type ManagedCollision struct {
	Agent  Kind   `json:"agent"`
	Path   string `json:"path"`
	Type   string `json:"type"`
	Action string `json:"action"`
}

// Preview is an immutable approval boundary. The token is bound to the
// selected Agents and every source and target revision observed by Preview.
type Preview struct {
	RevisionToken             string             `json:"revision_token"`
	Agents                    []AgentPreview     `json:"agents"`
	Warnings                  []string           `json:"warnings,omitempty"`
	ModelConfig               json.RawMessage    `json:"model_config"`
	Fragments                 []Fragment         `json:"fragments"`
	ManagedConfigDrift        bool               `json:"managed_config_drift"`
	DriftedAgents             []Kind             `json:"drifted_agents"`
	ManagedCollisions         []ManagedCollision `json:"managed_collisions"`
	RequiresCodexAuthApproval bool               `json:"requires_codex_auth_approval"`
	StateChange               *FilePreview       `json:"state_change,omitempty"`
	StateBackup               *FilePreview       `json:"state_backup,omitempty"`
}

// WriteRequest contains the transient secret and approved revision. APIKey is
// consumed only in memory and is never included in results or journal state.
type WriteRequest struct {
	Agents                  []Kind
	RevisionToken           string
	APIKey                  string
	CatalogToken            string
	ModelConfig             json.RawMessage
	ApproveManagedOverwrite bool
	ApproveCodexAuthChange  bool
}

// AgentWriteStatus reports paths only. Backups and rollback backups are
// sensitive artifacts and their contents must not be included in diagnostics.
type AgentWriteStatus struct {
	Agent           Kind              `json:"agent"`
	Success         bool              `json:"success"`
	Files           []FileWriteStatus `json:"files"`
	Changed         []string          `json:"changed,omitempty"`
	Backups         []string          `json:"backups,omitempty"`
	RollbackBackups []string          `json:"rollback_backups,omitempty"`
	RolledBack      bool              `json:"rolled_back,omitempty"`
	ErrorCode       ErrorCode         `json:"error_code,omitempty"`
}

// FileWriteStatus reports durable progress for one target path.
type FileWriteStatus struct {
	Path               string `json:"path"`
	BackupPath         string `json:"backup_path,omitempty"`
	RollbackBackupPath string `json:"rollback_backup_path,omitempty"`
	Replaced           bool   `json:"replaced"`
	Restored           bool   `json:"restored,omitempty"`
}

// WriteResult reports transaction progress without returning the API key or
// any file contents.
type WriteResult struct {
	TransactionID  string             `json:"transaction_id"`
	Agents         []AgentWriteStatus `json:"agents"`
	SensitiveFiles bool               `json:"sensitive_files"`
	Warning        string             `json:"warning"`
	StateChange    *FileWriteStatus   `json:"state_change,omitempty"`
	StateBackup    *FileWriteStatus   `json:"state_backup,omitempty"`
}

// Options configures an Agent service. StateDir stores only the key-free
// recovery journal. Zero-value Detector fields retain their normal semantics.
type Options struct {
	StateDir string
	Detector Detector
	Preset   *modelconfig.Config
	// LegacyRenderInput keeps the pre-v2 transaction scaffold testable until
	// Phase 4 replaces its request types. Production callers leave it nil.
	LegacyRenderInput *LegacyRenderInput
}

// LegacyRenderInput supplies canonical data to the merge-aware Phase 4
// scaffold. It exists only to prevent that scaffold from inventing models.
type LegacyRenderInput struct {
	Config        *modelconfig.Config
	RouterBaseURL string
	APIBaseURL    string
}

// Service serializes previews, writes, and recovery for one manager process.
type Service struct {
	mu             sync.Mutex
	stateDir       string
	detector       Detector
	signer         *modelconfig.TokenSigner
	keyGeneration  string
	writesDisabled bool
	recoveryErr    error
	hooks          serviceHooks
	legacyRender   *renderInput
	currentInput   renderInput
	currentSidecar lastAppliedState
	preset         *modelconfig.Config
}

type serviceHooks struct {
	beforeBackup   func(string) error
	backupStage    func(backupStage, string) error
	beforeReplace  func(string) error
	afterReplace   func(string)
	beforeRollback func(string) error
}

// NewService recovers any incomplete transaction before returning. On a
// recovery failure it returns both the disabled service and ROLLBACK_FAILED so
// callers may continue to offer read-only detection while rejecting writes.
func NewService(options Options) (*Service, error) {
	if strings.TrimSpace(options.StateDir) == "" {
		return nil, operationError(CodeInvalidParams, "Agent transaction state directory is required")
	}
	service := &Service{stateDir: filepath.Clean(options.StateDir), detector: options.Detector}
	if options.Preset != nil {
		canonical, err := modelconfig.Canonical(options.Preset)
		if err != nil {
			return nil, operationError(CodeModelConfigInvalid, "Agent model preset is invalid")
		}
		service.preset, err = modelconfig.DecodeStructural(canonical)
		if err != nil {
			return nil, operationError(CodeModelConfigInvalid, "Agent model preset is invalid")
		}
	}
	if options.LegacyRenderInput != nil {
		service.legacyRender = &renderInput{config: options.LegacyRenderInput.Config, routerBaseURL: options.LegacyRenderInput.RouterBaseURL, apiBaseURL: options.LegacyRenderInput.APIBaseURL}
	}
	lock, lockErr := acquireTransactionLock(context.Background(), service.stateDir)
	if lockErr != nil {
		return service, lockErr
	}
	defer lock.release()
	if _, err := os.Stat(service.journalPath()); err == nil {
		if err := restrictPrivate(service.stateDir, true); err != nil {
			service.disableWrites(err)
			return service, service.recoveryErr
		}
		if err := restrictPrivate(service.journalPath(), false); err != nil {
			service.disableWrites(err)
			return service, service.recoveryErr
		}
		if err := service.recoverLocked(context.Background()); err != nil {
			service.disableWrites(err)
			return service, service.recoveryErr
		}
	} else if !os.IsNotExist(err) {
		service.disableWrites(err)
		return service, service.recoveryErr
	}
	return service, nil
}

// WritesDisabled reports the fail-closed recovery state.
func (s *Service) WritesDisabled() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(context.Background(), s.stateDir)
	if err != nil {
		return true
	}
	defer lock.release()
	return s.writesDisabled
}

// RecoveryError returns the sanitized startup recovery failure, if any.
func (s *Service) RecoveryError() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(context.Background(), s.stateDir)
	if err != nil {
		return err
	}
	defer lock.release()
	return s.recoveryErr
}

// Detect returns a lock-consistent snapshot of supported Agent state.
func (s *Service) Detect(ctx context.Context) ([]State, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return nil, err
	}
	defer lock.release()
	if err := contextError(ctx); err != nil {
		return nil, err
	}
	states, err := s.detector.Detect()
	if err != nil {
		return nil, err
	}
	var managerReason RecoveryReason
	if s.writesDisabled {
		managerReason = RecoveryWritesDisabled
	} else if _, err := os.Stat(s.journalPath()); err == nil {
		managerReason = RecoveryTransactionPending
	} else if !os.IsNotExist(err) {
		managerReason = RecoveryTransactionPending
	}
	if managerReason != "" {
		for i := range states {
			states[i].Recovery.Eligible = false
			states[i].Recovery.Reasons = appendReason(states[i].Recovery.Reasons, managerReason)
		}
	}
	return states, nil
}

// Preview validates selected targets and returns a structured, key-free change
// set. It performs no writes, including directory or backup creation.
func (s *Service) Preview(ctx context.Context, selected []Kind, values ...any) (Preview, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return Preview{}, err
	}
	defer lock.release()
	if err := contextError(ctx); err != nil {
		return Preview{}, err
	}
	var catalogToken string
	var rawConfig json.RawMessage
	if len(values) == 2 {
		if err := s.ensureExistingSigner(); err != nil {
			return Preview{}, err
		}
		catalogToken, _ = values[0].(string)
		rawConfig, _ = values[1].(json.RawMessage)
	} else if len(values) == 0 && s.legacyRender != nil {
		if err := s.ensureSigner(); err != nil {
			return Preview{}, err
		}
		claims := modelconfig.CatalogClaims{Models: []string{"model-primary", "model-sonnet"}, Agents: kindsToModelAgents(selected), Owner: "cli", RouterBaseURL: s.legacyRender.routerBaseURL, DeploymentID: "legacy-tests", ProtocolVersion: "2"}
		var err error
		catalogToken, err = s.signer.SignCatalog(claims)
		if err != nil {
			return Preview{}, err
		}
		rawConfig, err = modelconfig.Canonical(projectConfig(s.legacyRender.config, selected))
		if err != nil {
			return Preview{}, err
		}
	} else {
		return Preview{}, operationError(CodeInvalidParams, "catalog token and canonical model configuration are required")
	}
	claims, config, canonical, err := s.validateV2(selected, catalogToken, rawConfig)
	if err != nil {
		return Preview{}, err
	}
	apiBaseURL, err := apiURL(claims.RouterBaseURL)
	if err != nil {
		return Preview{}, operationError(CodeModelCatalogStale, "model catalog router address is invalid")
	}
	s.currentInput = renderInput{config: config, routerBaseURL: claims.RouterBaseURL, apiBaseURL: apiBaseURL, ownRootModel: len(values) == 2}
	defer func() { s.currentInput = renderInput{} }()
	plan, err := s.buildPlan(selected, s.currentInput, claims, canonical)
	if err != nil {
		return Preview{}, err
	}
	return s.previewForPlan(plan)
}

func projectConfig(config *modelconfig.Config, selected []Kind) *modelconfig.Config {
	result := &modelconfig.Config{Version: config.Version}
	for _, kind := range selected {
		switch kind {
		case ClaudeCode:
			result.Claude = config.Claude
		case OpenCode:
			result.OpenCode = config.OpenCode
		case Codex:
			result.Codex = config.Codex
		}
	}
	return result
}

func (s *Service) validateV2(selected []Kind, catalogToken string, rawConfig json.RawMessage) (modelconfig.CatalogClaims, *modelconfig.Config, json.RawMessage, error) {
	claims, err := s.signer.VerifyCatalog(catalogToken)
	if err != nil {
		return modelconfig.CatalogClaims{}, nil, nil, operationError(CodeModelCatalogStale, "model catalog token is invalid")
	}
	normalized, err := normalizeSelection(selected)
	if err != nil {
		return modelconfig.CatalogClaims{}, nil, nil, err
	}
	if !sameModelAgents(normalized, claims.Agents) {
		return modelconfig.CatalogClaims{}, nil, nil, operationError(CodeModelCatalogStale, "model catalog Agent scope changed")
	}
	config, err := modelconfig.Decode(rawConfig, claims.Agents, claims.Models)
	if err != nil {
		return modelconfig.CatalogClaims{}, nil, nil, operationErrorWithCause(CodeModelConfigInvalid, "canonical model configuration is invalid", err)
	}
	canonical, err := modelconfig.Canonical(config)
	if err != nil {
		return modelconfig.CatalogClaims{}, nil, nil, operationError(CodeModelConfigInvalid, "canonical model configuration is invalid")
	}
	return claims, config, canonical, nil
}

// Write revalidates the approved preview before creating any backup or
// replacing any target. Any failure after replacement starts triggers reverse
// order rollback before Write returns.
func (s *Service) Write(ctx context.Context, request WriteRequest) (WriteResult, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return WriteResult{}, err
	}
	defer lock.release()

	if s.writesDisabled {
		return WriteResult{}, operationError(CodeRollbackFailed, "Agent writes are disabled because transaction recovery failed")
	}
	if request.APIKey == "" || request.RevisionToken == "" {
		return WriteResult{}, operationError(CodeInvalidParams, "API key and revision token are required")
	}
	key := []byte(request.APIKey)
	defer zeroBytes(key)
	if err := s.ensureSigner(); err != nil {
		return WriteResult{}, err
	}
	legacyRequest := request.CatalogToken == "" && len(request.ModelConfig) == 0 && s.legacyRender != nil
	if legacyRequest {
		claims := modelconfig.CatalogClaims{Models: []string{"model-primary", "model-sonnet"}, Agents: kindsToModelAgents(request.Agents), Owner: "cli", RouterBaseURL: s.legacyRender.routerBaseURL, DeploymentID: "legacy-tests", ProtocolVersion: "2"}
		request.CatalogToken, err = s.signer.SignCatalog(claims)
		if err != nil {
			return WriteResult{}, err
		}
		request.ModelConfig, err = modelconfig.Canonical(projectConfig(s.legacyRender.config, request.Agents))
		if err != nil {
			return WriteResult{}, err
		}
	}

	claims, config, canonical, err := s.validateV2(request.Agents, request.CatalogToken, request.ModelConfig)
	if err != nil {
		return WriteResult{}, err
	}
	apiBaseURL, err := apiURL(claims.RouterBaseURL)
	if err != nil {
		return WriteResult{}, operationError(CodeModelCatalogStale, "model catalog router address is invalid")
	}
	s.currentInput = renderInput{config: config, routerBaseURL: claims.RouterBaseURL, apiBaseURL: apiBaseURL, ownRootModel: !legacyRequest}
	defer func() { s.currentInput = renderInput{} }()
	plan, err := s.buildPlan(request.Agents, s.currentInput, claims, canonical)
	if err != nil {
		return WriteResult{}, err
	}
	if legacyRequest {
		request.ApproveManagedOverwrite = len(plan.drifted) != 0
		request.ApproveCodexAuthChange = plan.requiresCodexAuthApproval
	}
	if (len(plan.drifted) != 0) != request.ApproveManagedOverwrite {
		return WriteResult{}, operationError(CodeManagedConfigDrift, "managed Agent configuration drift requires approval")
	}
	if plan.requiresCodexAuthApproval != request.ApproveCodexAuthChange {
		return WriteResult{}, operationError(CodeInvalidParams, "Codex authentication approval does not match preview")
	}
	currentToken, err := s.tokenForPlan(plan)
	if err != nil {
		return WriteResult{}, operationError(CodeModelStateInvalid, "Agent revision token could not be created")
	}
	if subtle.ConstantTimeCompare([]byte(currentToken), []byte(request.RevisionToken)) != 1 {
		return WriteResult{}, operationError(CodePreviewStale, "Agent configuration changed after preview")
	}
	if err := contextError(ctx); err != nil {
		return WriteResult{}, err
	}
	for i := range plan.files {
		file := &plan.files[i]
		if err := s.verifyPlannedRevision(file.sourcePath, file.sourceRevision, file.scope); err != nil {
			return WriteResult{}, operationError(CodePreviewStale, "Agent configuration changed after preview")
		}
		if file.sourcePath != file.targetPath {
			if err := s.verifyPlannedRevision(file.targetPath, file.targetRevision, file.scope); err != nil {
				return WriteResult{}, operationError(CodePreviewStale, "Agent configuration target changed after preview")
			}
		}
	}
	plan, err = s.appendSidecarPlan(plan, key)
	if err != nil {
		return WriteResult{}, err
	}
	if err := s.prepareStateDir(); err != nil {
		return WriteResult{}, operationError(CodeWriteFailed, "could not prepare Agent transaction state")
	}

	transactionID, err := randomID()
	if err != nil {
		return WriteResult{}, operationError(CodeWriteFailed, "could not initialize Agent transaction")
	}
	result := resultForPlan(transactionID, plan)
	journal := transactionJournal{Version: 2, KeyGeneration: s.keyGeneration, TransactionID: transactionID, Entries: make([]journalEntry, len(plan.files))}
	var createdBackups []string
	backupFailure := func() (WriteResult, error) {
		failure := operationError(CodeBackupFailed, "could not create an Agent configuration backup")
		remaining, err := cleanupCreatedBackups(createdBackups)
		if err != nil {
			retainResultBackups(&result, remaining)
			s.disableWrites(err)
			markResultFailure(&result, s.recoveryErr)
			return result, s.recoveryErr
		}
		clearResultBackups(&result)
		markResultFailure(&result, failure)
		return result, failure
	}

	for i := range plan.files {
		file := &plan.files[i]
		if err := contextError(ctx); err != nil {
			markResultFailure(&result, err)
			return result, err
		}
		if file.backupRequired {
			if s.hooks.beforeBackup != nil {
				if err := s.hooks.beforeBackup(file.backupSource); err != nil {
					return backupFailure()
				}
			}
			backupPath, err := createPrivateBackupWithHook(file.backupSource, file.sourceContent, file.sourceMode, "bak", s.hooks.backupStage)
			if err != nil {
				if backupPath != "" {
					createdBackups = append(createdBackups, backupPath)
				}
				return backupFailure()
			}
			file.backupPath = backupPath
			createdBackups = append(createdBackups, backupPath)
			if file.scope == scopeManagerState {
				result.StateBackup = &FileWriteStatus{Path: file.targetPath, BackupPath: backupPath}
			} else {
				appendAgentBackup(&result, file.agent, file.targetPath, backupPath)
			}
		}
		output, err := file.render(key)
		if err != nil {
			failure := operationError(CodeWriteFailed, "could not render an Agent configuration file")
			markResultFailure(&result, failure)
			return result, failure
		}
		mode := file.targetMode
		if !file.targetRevision.Exists {
			mode = 0o600
		}
		postRevision, err := s.keyedRevisionForContent(output, mode, revisionContextJournal, file.targetPath)
		if err != nil {
			zeroBytes(output)
			return WriteResult{}, operationError(CodeModelStateInvalid, "Agent revision state could not be created")
		}
		preRevision, _, _, err := s.readKeyedRevision(file.targetPath, revisionContextJournal)
		if err != nil {
			zeroBytes(output)
			return WriteResult{}, operationError(CodeWriteFailed, "could not snapshot Agent transaction state")
		}
		zeroBytes(output)
		var backupRevision fileRevision
		if file.backupPath != "" {
			backupRevision, err = s.keyedRevisionForContent(file.sourceContent, file.sourceMode, revisionContextBackup, file.backupPath)
			if err != nil {
				return WriteResult{}, operationError(CodeModelStateInvalid, "Agent backup revision could not be created")
			}
		}
		journal.Entries[i] = journalEntry{
			Scope:          file.scope,
			Agent:          file.agent,
			TargetPath:     file.targetPath,
			PreRevision:    preRevision,
			PostRevision:   postRevision,
			BackupPath:     file.backupPath,
			BackupRevision: backupRevision,
			RestoreFrom:    file.restoreFrom,
			TargetMode:     uint32(file.targetMode.Perm()),
			Progress:       progressPending,
		}
	}

	if err := s.writeJournal(journal); err != nil {
		failure := operationError(CodeWriteFailed, "could not persist Agent transaction journal")
		if cleanupErr := removeAndSync(s.journalPath()); cleanupErr != nil {
			s.disableWrites(cleanupErr)
			markResultFailure(&result, s.recoveryErr)
			return result, s.recoveryErr
		}
		markResultFailure(&result, failure)
		return result, failure
	}

	replaced := false
	for i := range plan.files {
		file := &plan.files[i]
		if err := contextError(ctx); err != nil {
			return s.failAndRollback(ctx, &journal, result, err, replaced)
		}
		if err := s.verifyPlannedRevision(file.sourcePath, file.sourceRevision, file.scope); err != nil {
			failure := operationError(CodePreviewStale, "Agent configuration changed while writing")
			return s.failAndRollback(ctx, &journal, result, failure, replaced)
		}
		if file.sourcePath != file.targetPath {
			if err := s.verifyPlannedRevision(file.targetPath, file.targetRevision, file.scope); err != nil {
				failure := operationError(CodePreviewStale, "Agent configuration target changed while writing")
				return s.failAndRollback(ctx, &journal, result, failure, replaced)
			}
		}
		if s.hooks.beforeReplace != nil {
			if err := s.hooks.beforeReplace(file.targetPath); err != nil {
				failure := operationError(CodeWriteFailed, "could not replace an Agent configuration file")
				return s.failAndRollback(ctx, &journal, result, failure, replaced)
			}
		}
		output, err := file.render(key)
		if err != nil {
			failure := operationError(CodeWriteFailed, "could not render an Agent configuration file")
			return s.failAndRollback(ctx, &journal, result, failure, replaced)
		}
		mode := file.targetMode
		if !file.targetRevision.Exists {
			mode = 0o600
		}
		if err := writeAtomic(file.targetPath, output, mode, !file.targetRevision.Exists); err != nil {
			zeroBytes(output)
			if replacementOccurred(err) {
				replaced = true
				markPlannedFileReplaced(&result, file)
			}
			failure := operationError(CodeWriteFailed, "could not replace an Agent configuration file")
			return s.failAndRollback(ctx, &journal, result, failure, replaced)
		}
		zeroBytes(output)
		replaced = true
		markPlannedFileReplaced(&result, file)
		journal.Entries[i].Progress = progressReplaced
		if err := s.writeJournal(journal); err != nil {
			failure := operationError(CodeWriteFailed, "could not record Agent transaction progress")
			return s.failAndRollback(ctx, &journal, result, failure, replaced)
		}
		if s.hooks.afterReplace != nil {
			s.hooks.afterReplace(file.targetPath)
		}
	}

	if err := contextError(ctx); err != nil {
		return s.failAndRollback(ctx, &journal, result, err, replaced)
	}
	journal.Committed = true
	if err := s.writeJournal(journal); err != nil {
		failure := operationError(CodeWriteFailed, "could not commit Agent transaction journal")
		return s.failAndRollback(ctx, &journal, result, failure, replaced)
	}
	// A synced committed journal is itself a durable commit. Cleanup failure is
	// harmless: startup recognizes the marker and removes it without rollback.
	_ = removeAndSync(s.journalPath())
	for i := range result.Agents {
		result.Agents[i].Success = true
	}
	return result, nil
}

func cleanupCreatedBackups(paths []string) ([]string, error) {
	for i := len(paths) - 1; i >= 0; i-- {
		if err := removeAndSync(paths[i]); err != nil {
			return append([]string(nil), paths[:i+1]...), err
		}
	}
	return nil, nil
}

func clearResultBackups(result *WriteResult) {
	result.StateBackup = nil
	for i := range result.Agents {
		result.Agents[i].Backups = nil
		for j := range result.Agents[i].Files {
			result.Agents[i].Files[j].BackupPath = ""
		}
	}
}

func retainResultBackups(result *WriteResult, remaining []string) {
	keep := make(map[string]bool, len(remaining))
	for _, path := range remaining {
		keep[path] = true
	}
	if result.StateBackup != nil && !keep[result.StateBackup.BackupPath] {
		result.StateBackup = nil
	}
	for i := range result.Agents {
		backups := result.Agents[i].Backups[:0]
		for _, path := range result.Agents[i].Backups {
			if keep[path] {
				backups = append(backups, path)
			}
		}
		result.Agents[i].Backups = backups
		for j := range result.Agents[i].Files {
			if !keep[result.Agents[i].Files[j].BackupPath] {
				result.Agents[i].Files[j].BackupPath = ""
			}
		}
	}
}

func (s *Service) verifyPlannedRevision(path string, expected fileRevision, scope journalScope) error {
	contextName := revisionContextAgentFile
	if scope == scopeManagerState {
		contextName = revisionContextSidecar
	}
	return s.verifyKeyedRevision(path, expected, contextName)
}

func containsKind(kinds []Kind, want Kind) bool {
	for _, kind := range kinds {
		if kind == want {
			return true
		}
	}
	return false
}

// Recover completes startup recovery for an existing service. A prior recovery
// failure remains fail-closed unless a later call proves full restoration.
func (s *Service) Recover(ctx context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return err
	}
	defer lock.release()
	if err := s.recoverLocked(ctx); err != nil {
		s.disableWrites(err)
		return s.recoveryErr
	}
	s.writesDisabled = false
	s.recoveryErr = nil
	return nil
}

func (s *Service) disableWrites(error) {
	s.writesDisabled = true
	s.recoveryErr = operationError(CodeRollbackFailed, "Agent transaction recovery could not prove restoration")
}

func contextError(ctx context.Context) error {
	if ctx == nil {
		return nil
	}
	select {
	case <-ctx.Done():
		return operationError(CodeOperationTimeout, "Agent operation deadline exceeded")
	default:
		return nil
	}
}

func zeroBytes(value []byte) {
	for i := range value {
		value[i] = 0
	}
}

func randomID() (string, error) {
	var value [16]byte
	if _, err := io.ReadFull(rand.Reader, value[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(value[:]), nil
}

func normalizeSelection(selected []Kind) ([]Kind, error) {
	if len(selected) == 0 {
		return nil, operationError(CodeInvalidParams, "at least one Agent must be selected")
	}
	set := make(map[Kind]bool, len(selected))
	for _, kind := range selected {
		switch kind {
		case ClaudeCode, OpenCode, Codex:
			set[kind] = true
		default:
			return nil, operationError(CodeInvalidParams, "unsupported Agent selection")
		}
	}
	result := make([]Kind, 0, len(set))
	for _, kind := range []Kind{ClaudeCode, OpenCode, Codex} {
		if set[kind] {
			result = append(result, kind)
		}
	}
	return result, nil
}

func (s *Service) tokenForPlan(plan writePlan) (string, error) {
	agents := make([]modelconfig.Agent, 0, len(plan.selected))
	bindings := make([]modelconfig.RevisionBinding, 0, len(plan.files)*2)
	for _, kind := range plan.selected {
		agents = append(agents, modelAgent(kind))
	}
	for _, file := range plan.files {
		bindings = append(bindings,
			modelconfig.RevisionBinding{Context: "source", Identity: filepath.Clean(file.sourcePath), Revision: revisionTokenValue(file.sourceRevision)},
			modelconfig.RevisionBinding{Context: "target", Identity: filepath.Clean(file.targetPath), Revision: revisionTokenValue(file.targetRevision)},
		)
	}
	return s.signer.SignRevision(modelconfig.RevisionClaims{
		Agents: agents, CanonicalConfig: plan.canonical, CatalogIdentity: catalogIdentity(plan.catalog),
		SidecarRevision: revisionTokenValue(plan.sidecarRevision), RouterBaseURL: plan.catalog.RouterBaseURL,
		DeploymentID: plan.catalog.DeploymentID, ProtocolVersion: plan.catalog.ProtocolVersion, Bindings: bindings,
		ManagedDrift: len(plan.drifted) != 0, DriftedAgents: kindsToModelAgents(plan.drifted), RequiresCodexAuthApproval: plan.requiresCodexAuthApproval,
	})
}

func catalogIdentity(claims modelconfig.CatalogClaims) string {
	encoded, _ := modelconfig.CanonicalValue(claims)
	digest := sha256.Sum256(encoded)
	return hex.EncodeToString(digest[:])
}

func kindsToModelAgents(kinds []Kind) []modelconfig.Agent {
	result := make([]modelconfig.Agent, len(kinds))
	for i, kind := range kinds {
		result[i] = modelAgent(kind)
	}
	return result
}

func revisionTokenValue(revision fileRevision) string {
	if !revision.Exists {
		return "absent"
	}
	return revision.Digest
}

func (s *Service) previewForPlan(plan writePlan) (Preview, error) {
	token, err := s.tokenForPlan(plan)
	if err != nil {
		return Preview{}, err
	}
	states, err := s.detector.targetStates()
	if err != nil {
		return Preview{}, operationError(CodeConfigInvalid, "Agent target paths are unavailable")
	}
	fragmentInput := plan.input
	fragmentInput.key = RedactedAPIKey
	fragments, err := renderFragments(plan.selected, states, fragmentInput)
	if err != nil {
		return Preview{}, err
	}
	stateOperation := OperationCreate
	if plan.sidecarRevision.Exists {
		stateOperation = OperationReplace
	}
	preview := Preview{
		RevisionToken:             token,
		Agents:                    make([]AgentPreview, 0, len(plan.selected)),
		ModelConfig:               append(json.RawMessage(nil), plan.canonical...),
		Fragments:                 fragments,
		ManagedConfigDrift:        len(plan.drifted) != 0,
		DriftedAgents:             append([]Kind(nil), plan.drifted...),
		ManagedCollisions:         append([]ManagedCollision(nil), plan.collisions...),
		RequiresCodexAuthApproval: plan.requiresCodexAuthApproval,
		StateChange: &FilePreview{
			Path:       s.sidecarPath(),
			Format:     FormatJSON,
			Operation:  stateOperation,
			Operations: []Operation{stateOperation},
			Backup: BackupPlan{
				Required:  plan.sidecarRevision.Exists,
				Sensitive: plan.sidecarRevision.Exists,
			},
		},
	}
	if preview.StateChange.Backup.Required {
		preview.StateChange.Backup.Pattern = s.sidecarPath() + ".bak-<timestamp>-<random>"
		preview.StateChange.Backup.Warning = backupWarning
	}
	for _, kind := range plan.selected {
		agentPreview := AgentPreview{Agent: kind, Name: agentName(kind)}
		for _, file := range plan.files {
			if file.agent != kind {
				continue
			}
			backup := BackupPlan{Required: file.backupRequired, Sensitive: file.backupRequired}
			if backup.Required {
				backup.Pattern = file.backupSource + ".bak-<timestamp>-<random>"
				backup.Warning = backupWarning
			}
			operations := []Operation{file.operation}
			if len(file.preserves) != 0 {
				operations = append(operations, OperationPreserve)
			}
			agentPreview.Files = append(agentPreview.Files, FilePreview{
				Path:           file.targetPath,
				SourcePath:     differentPath(file.sourcePath, file.targetPath),
				Format:         file.format,
				Operation:      file.operation,
				Operations:     operations,
				ContainsAPIKey: file.containsAPIKey,
				Preserves:      append([]string(nil), file.preserves...),
				Backup:         backup,
				Warning:        file.warning,
			})
			if file.warning != "" {
				agentPreview.Warnings = append(agentPreview.Warnings, file.warning)
				preview.Warnings = append(preview.Warnings, file.warning)
			}
			if backup.Required {
				agentPreview.Warnings = append(agentPreview.Warnings, backupWarning)
				preview.Warnings = append(preview.Warnings, backupWarning)
			}
		}
		agentPreview.Warnings = uniqueWarnings(agentPreview.Warnings)
		preview.Agents = append(preview.Agents, agentPreview)
	}
	preview.Warnings = uniqueWarnings(preview.Warnings)
	return preview, nil
}

func modelAgent(kind Kind) modelconfig.Agent {
	switch kind {
	case ClaudeCode:
		return modelconfig.Claude
	case OpenCode:
		return modelconfig.OpenCode
	default:
		return modelconfig.Codex
	}
}

func differentPath(source, target string) string {
	if source == target {
		return ""
	}
	return source
}

func agentName(kind Kind) string {
	switch kind {
	case ClaudeCode:
		return "Claude Code"
	case OpenCode:
		return "opencode"
	case Codex:
		return "Codex"
	default:
		return string(kind)
	}
}

func resultForPlan(transactionID string, plan writePlan) WriteResult {
	result := WriteResult{TransactionID: transactionID, SensitiveFiles: true, Warning: backupWarning}
	for _, kind := range plan.selected {
		status := AgentWriteStatus{Agent: kind}
		for _, file := range plan.files {
			if file.agent == kind {
				status.Files = append(status.Files, FileWriteStatus{Path: file.targetPath})
			}
		}
		result.Agents = append(result.Agents, status)
	}
	for _, file := range plan.files {
		if file.scope == scopeManagerState {
			result.StateChange = &FileWriteStatus{Path: file.targetPath}
		}
	}
	return result
}

func appendAgentBackup(result *WriteResult, kind Kind, targetPath, backupPath string) {
	for i := range result.Agents {
		if result.Agents[i].Agent == kind {
			result.Agents[i].Backups = append(result.Agents[i].Backups, backupPath)
			for j := range result.Agents[i].Files {
				if result.Agents[i].Files[j].Path == targetPath {
					result.Agents[i].Files[j].BackupPath = backupPath
					return
				}
			}
			return
		}
	}
}

func appendRollbackBackup(result *WriteResult, kind Kind, targetPath, backupPath string) {
	for i := range result.Agents {
		if result.Agents[i].Agent == kind {
			result.Agents[i].RollbackBackups = append(result.Agents[i].RollbackBackups, backupPath)
			for j := range result.Agents[i].Files {
				if result.Agents[i].Files[j].Path == targetPath {
					result.Agents[i].Files[j].RollbackBackupPath = backupPath
					return
				}
			}
			return
		}
	}
}

func markFileReplaced(result *WriteResult, kind Kind, path string) {
	for i := range result.Agents {
		if result.Agents[i].Agent != kind {
			continue
		}
		result.Agents[i].Changed = append(result.Agents[i].Changed, path)
		for j := range result.Agents[i].Files {
			if result.Agents[i].Files[j].Path == path {
				result.Agents[i].Files[j].Replaced = true
				return
			}
		}
	}
}

func markPlannedFileReplaced(result *WriteResult, file *plannedFile) {
	if file.scope == scopeManagerState {
		if result.StateChange != nil {
			result.StateChange.Replaced = true
		}
		return
	}
	markFileReplaced(result, file.agent, file.targetPath)
}

func markFileRestored(result *WriteResult, kind Kind, path string) {
	for i := range result.Agents {
		if result.Agents[i].Agent != kind {
			continue
		}
		for j := range result.Agents[i].Files {
			if result.Agents[i].Files[j].Path == path {
				result.Agents[i].Files[j].Restored = true
				return
			}
		}
	}
}

func markResultFailure(result *WriteResult, err error) {
	code := CodeOf(err)
	for i := range result.Agents {
		result.Agents[i].Success = false
		result.Agents[i].ErrorCode = code
	}
}

type fileRevision struct {
	Exists bool   `json:"exists"`
	Size   int64  `json:"size,omitempty"`
	Mode   uint32 `json:"mode,omitempty"`
	Digest string `json:"digest,omitempty"`
}

const (
	revisionContextAgentFile = "agent-file"
	revisionContextJournal   = "journal-entry"
	revisionContextBackup    = "backup-verification"
	revisionContextSidecar   = "manager-state-sidecar"
)

func (s *Service) keyedRevisionForContent(content []byte, mode os.FileMode, revisionContext, identity string) (fileRevision, error) {
	digest, err := s.signer.RevisionMAC(revisionContext, filepath.Clean(identity), content)
	if err != nil {
		return fileRevision{}, err
	}
	return fileRevision{Exists: true, Size: int64(len(content)), Mode: uint32(mode.Perm()), Digest: digest}, nil
}

func (s *Service) readKeyedRevision(path, revisionContext string) (fileRevision, []byte, os.FileMode, error) {
	info, err := os.Stat(path)
	if os.IsNotExist(err) {
		return fileRevision{}, nil, 0o600, nil
	}
	if err != nil || !info.Mode().IsRegular() {
		return fileRevision{}, nil, 0, errors.New("invalid revision target")
	}
	content, err := readConfig(path)
	if err != nil {
		return fileRevision{}, nil, 0, err
	}
	infoAfter, err := os.Stat(path)
	if err != nil || !os.SameFile(info, infoAfter) || info.Size() != infoAfter.Size() || info.ModTime() != infoAfter.ModTime() {
		return fileRevision{}, nil, 0, errors.New("file changed while reading")
	}
	revision, err := s.keyedRevisionForContent(content, infoAfter.Mode().Perm(), revisionContext, path)
	return revision, content, infoAfter.Mode().Perm(), err
}

func (s *Service) verifyKeyedRevision(path string, expected fileRevision, revisionContext string) error {
	current, _, _, err := s.readKeyedRevision(path, revisionContext)
	if err != nil {
		return err
	}
	if current != expected {
		return errors.New("revision mismatch")
	}
	return nil
}

func (revision fileRevision) writeTo(dst io.Writer) {
	fmt.Fprintf(dst, "%t:%d:%d:%s\x00", revision.Exists, revision.Size, revision.Mode, revision.Digest)
}

func readRevision(path string) (fileRevision, []byte, os.FileMode, error) {
	info, err := os.Stat(path)
	if os.IsNotExist(err) {
		return fileRevision{}, nil, 0o600, nil
	}
	if err != nil {
		return fileRevision{}, nil, 0, err
	}
	if !info.Mode().IsRegular() {
		return fileRevision{}, nil, 0, errors.New("not a regular file")
	}
	content, err := readConfig(path)
	if err != nil {
		return fileRevision{}, nil, 0, err
	}
	infoAfter, err := os.Stat(path)
	if err != nil || !os.SameFile(info, infoAfter) || info.Size() != infoAfter.Size() || info.ModTime() != infoAfter.ModTime() {
		return fileRevision{}, nil, 0, errors.New("file changed while reading")
	}
	revision := revisionForContent(content, infoAfter.Mode().Perm())
	return revision, content, infoAfter.Mode().Perm(), nil
}

func revisionForContent(content []byte, mode os.FileMode) fileRevision {
	digest := sha256.Sum256(content)
	return fileRevision{
		Exists: true,
		Size:   int64(len(content)),
		Mode:   uint32(mode.Perm()),
		Digest: hex.EncodeToString(digest[:]),
	}
}

func verifyRevision(path string, expected fileRevision) error {
	current, _, _, err := readRevision(path)
	if err != nil {
		return err
	}
	if current != expected {
		return errors.New("revision mismatch")
	}
	return nil
}

func orderedStates(states []State) map[Kind]State {
	result := make(map[Kind]State, len(states))
	for _, state := range states {
		result[state.Agent] = state
	}
	return result
}

func ensureWritable(path string) error {
	if !pathWritable(path) {
		return operationError(CodeConfigNotWritable, "Agent configuration path is not writable")
	}
	return nil
}

func uniqueWarnings(values []string) []string {
	set := make(map[string]bool, len(values))
	result := values[:0]
	for _, value := range values {
		if value != "" && !set[value] {
			set[value] = true
			result = append(result, value)
		}
	}
	sort.Strings(result)
	return result
}

func (s *Service) prepareStateDir() error {
	if err := os.MkdirAll(s.stateDir, 0o700); err != nil {
		return err
	}
	return restrictPrivate(s.stateDir, true)
}

func (s *Service) journalPath() string { return filepath.Join(s.stateDir, journalFileName) }

func (s *Service) writeJournal(journal transactionJournal) error {
	content, err := json.MarshalIndent(journal, "", "  ")
	if err != nil {
		return err
	}
	content = append(content, '\n')
	defer zeroBytes(content)
	return writeAtomic(s.journalPath(), content, 0o600, true)
}

func decodeJournal(path string) (transactionJournal, error) {
	content, err := readConfig(path)
	if err != nil {
		return transactionJournal{}, err
	}
	var journal transactionJournal
	decoder := json.NewDecoder(strings.NewReader(string(content)))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&journal); err != nil {
		return transactionJournal{}, err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return transactionJournal{}, errors.New("trailing journal data")
	}
	if (journal.Version != 1 && journal.Version != 2) || journal.TransactionID == "" || len(journal.Entries) == 0 {
		return transactionJournal{}, errors.New("invalid journal")
	}
	if journal.Version == 2 && journal.KeyGeneration == "" {
		return transactionJournal{}, errors.New("missing journal key generation")
	}
	for _, entry := range journal.Entries {
		if entry.TargetPath == "" || !filepath.IsAbs(entry.TargetPath) || (entry.Scope != scopeManagerState && entry.Agent == "") || (entry.Scope == scopeManagerState && entry.Agent != "") {
			return transactionJournal{}, errors.New("invalid journal entry")
		}
		if entry.PreRevision.Exists && entry.BackupPath == "" {
			return transactionJournal{}, errors.New("missing recovery backup")
		}
		if journal.Version == 2 && entry.PreRevision.Exists && !entry.BackupRevision.Exists {
			return transactionJournal{}, errors.New("missing keyed backup revision")
		}
		if !entry.PostRevision.Exists || entry.PostRevision.Digest == "" {
			return transactionJournal{}, errors.New("missing post-write revision")
		}
		if entry.BackupPath != "" && filepath.Dir(entry.BackupPath) != filepath.Dir(entry.RestoreFrom) {
			return transactionJournal{}, errors.New("backup is not beside its source")
		}
		if entry.BackupPath != "" && filepath.Dir(entry.BackupPath) != filepath.Dir(entry.TargetPath) {
			return transactionJournal{}, errors.New("backup is not beside its target")
		}
	}
	return journal, nil
}

func removeAndSync(path string) error {
	if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
		return err
	}
	return syncDirectory(filepath.Dir(path))
}

func (s *Service) failAndRollback(ctx context.Context, journal *transactionJournal, result WriteResult, original error, replaced bool) (WriteResult, error) {
	if !replaced {
		if err := removeAndSync(s.journalPath()); err != nil {
			s.disableWrites(err)
			markResultFailure(&result, s.recoveryErr)
			return result, s.recoveryErr
		}
		markResultFailure(&result, original)
		return result, original
	}
	// Rollback deliberately ignores cancellation: the write deadline includes
	// restoration, and returning before restoration would violate atomicity.
	if err := s.rollback(context.WithoutCancel(ctx), journal, &result); err != nil {
		s.disableWrites(err)
		markResultFailure(&result, s.recoveryErr)
		return result, s.recoveryErr
	}
	markResultFailure(&result, original)
	for i := range result.Agents {
		for _, file := range result.Agents[i].Files {
			if file.Restored {
				result.Agents[i].RolledBack = true
				break
			}
		}
	}
	return result, original
}

func (s *Service) recoverLocked(ctx context.Context) error {
	path := s.journalPath()
	if _, err := os.Stat(path); os.IsNotExist(err) {
		return nil
	} else if err != nil {
		return err
	}
	journal, err := decodeJournal(path)
	if err != nil {
		return err
	}
	if journal.Version == 2 {
		if err := s.ensureSigner(); err != nil || journal.KeyGeneration != s.keyGeneration {
			return errors.New("transaction signing key unavailable")
		}
	}
	if journal.Committed {
		return removeAndSync(path)
	}
	for _, entry := range journal.Entries {
		if entry.BackupPath != "" {
			if err := restrictPrivate(entry.BackupPath, false); err != nil {
				return err
			}
		}
		if entry.RollbackBackupPath != "" {
			if err := restrictPrivate(entry.RollbackBackupPath, false); err != nil {
				return err
			}
		}
	}
	return s.rollback(context.WithoutCancel(ctx), &journal, nil)
}

func (s *Service) rollback(_ context.Context, journal *transactionJournal, result *WriteResult) error {
	var conflict error
	for i := len(journal.Entries) - 1; i >= 0; i-- {
		entry := &journal.Entries[i]
		if err := s.verifyJournalRevision(journal.Version, entry.TargetPath, entry.PreRevision); err == nil {
			entry.Progress = progressRestored
			if err := s.writeJournal(*journal); err != nil {
				return err
			}
			continue
		}
		if err := s.verifyJournalRevision(journal.Version, entry.TargetPath, entry.PostRevision); err != nil {
			if conflict == nil {
				conflict = errors.New("target matches neither transaction revision")
			}
			continue
		}
		if s.hooks.beforeRollback != nil {
			if err := s.hooks.beforeRollback(entry.TargetPath); err != nil {
				return err
			}
		}
		currentRevision, currentContent, currentMode, readErr := s.readJournalRevision(journal.Version, entry.TargetPath)
		if readErr != nil {
			return readErr
		}
		if currentRevision.Exists && entry.RollbackBackupPath == "" {
			rollbackBackup, err := createPrivateBackup(entry.TargetPath, currentContent, currentMode, "rollback-"+journal.TransactionID)
			zeroBytes(currentContent)
			if err != nil {
				return err
			}
			entry.RollbackBackupPath = rollbackBackup
			if result != nil && entry.Scope != scopeManagerState {
				appendRollbackBackup(result, entry.Agent, entry.TargetPath, rollbackBackup)
			} else if result != nil && result.StateChange != nil {
				result.StateChange.RollbackBackupPath = rollbackBackup
			}
			if err := s.writeJournal(*journal); err != nil {
				return err
			}
		}
		if entry.PreRevision.Exists {
			backupContent, err := readConfig(entry.BackupPath)
			if err != nil {
				return err
			}
			if err := s.verifyBackupContent(journal.Version, entry.BackupPath, backupContent, entry.PreRevision, entry.BackupRevision); err != nil {
				zeroBytes(backupContent)
				return errors.New("recovery backup revision mismatch")
			}
			if err := writeAtomic(entry.TargetPath, backupContent, os.FileMode(entry.TargetMode), false); err != nil {
				zeroBytes(backupContent)
				return err
			}
			zeroBytes(backupContent)
		} else if err := removeAndSync(entry.TargetPath); err != nil {
			return err
		}
		if err := s.verifyJournalRevision(journal.Version, entry.TargetPath, entry.PreRevision); err != nil {
			return err
		}
		entry.Progress = progressRestored
		if result != nil && entry.Scope != scopeManagerState {
			markFileRestored(result, entry.Agent, entry.TargetPath)
		} else if result != nil && result.StateChange != nil {
			result.StateChange.Restored = true
		}
		if err := s.writeJournal(*journal); err != nil {
			return err
		}
	}
	if conflict != nil {
		return conflict
	}
	for _, entry := range journal.Entries {
		if err := s.verifyJournalRevision(journal.Version, entry.TargetPath, entry.PreRevision); err != nil {
			return err
		}
	}
	return removeAndSync(s.journalPath())
}

func (s *Service) readJournalRevision(version int, path string) (fileRevision, []byte, os.FileMode, error) {
	if version == 1 {
		return readRevision(path)
	}
	return s.readKeyedRevision(path, revisionContextJournal)
}

func (s *Service) verifyJournalRevision(version int, path string, expected fileRevision) error {
	if version == 1 {
		return verifyRevision(path, expected)
	}
	return s.verifyKeyedRevision(path, expected, revisionContextJournal)
}

func (s *Service) verifyBackupContent(version int, path string, content []byte, expected, backupExpected fileRevision) error {
	if int64(len(content)) != expected.Size {
		return errors.New("backup size mismatch")
	}
	if version == 1 {
		digest := sha256.Sum256(content)
		if hex.EncodeToString(digest[:]) != expected.Digest {
			return errors.New("backup digest mismatch")
		}
		return nil
	}
	revision, err := s.keyedRevisionForContent(content, os.FileMode(backupExpected.Mode), revisionContextBackup, path)
	if err != nil || revision != backupExpected {
		return errors.New("backup MAC mismatch")
	}
	return nil
}

type journalProgress string

const (
	progressPending  journalProgress = "pending"
	progressReplaced journalProgress = "replaced"
	progressRestored journalProgress = "restored"
)

type transactionJournal struct {
	Version       int            `json:"version"`
	KeyGeneration string         `json:"key_generation,omitempty"`
	TransactionID string         `json:"transaction_id"`
	Committed     bool           `json:"committed,omitempty"`
	Entries       []journalEntry `json:"entries"`
}

type journalEntry struct {
	Scope              journalScope    `json:"scope,omitempty"`
	Agent              Kind            `json:"agent,omitempty"`
	TargetPath         string          `json:"target_path"`
	PreRevision        fileRevision    `json:"pre_revision"`
	PostRevision       fileRevision    `json:"post_revision"`
	BackupPath         string          `json:"backup_path,omitempty"`
	BackupRevision     fileRevision    `json:"backup_revision,omitempty"`
	RestoreFrom        string          `json:"restore_from,omitempty"`
	RollbackBackupPath string          `json:"rollback_backup_path,omitempty"`
	TargetMode         uint32          `json:"target_mode"`
	Progress           journalProgress `json:"progress"`
}
