package agent

import (
	"bytes"
	"context"
	"crypto/subtle"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

type CleanupPreviewRequest struct {
	Agent Kind
}

type CleanupWriteRequest struct {
	Agent                   Kind
	RevisionToken           string
	ApproveManagedOverwrite bool
}

type CleanupPreview struct {
	RevisionToken      string        `json:"revision_token"`
	Agent              Kind          `json:"agent"`
	Files              []FilePreview `json:"files"`
	RemovedPaths       []string      `json:"removed_paths"`
	ManagedConfigDrift bool          `json:"managed_config_drift"`
	StateChange        *FilePreview  `json:"state_change,omitempty"`
	StateBackup        *FilePreview  `json:"state_backup,omitempty"`
}

type cleanupPlan struct {
	writePlan
	removedByPath  map[string][]string
	stateOperation Operation
}

// CleanupPreview builds a key-free cleanup plan from authenticated last-applied
// state. It does not derive paths from the current environment or create state.
func (s *Service) CleanupPreview(ctx context.Context, request CleanupPreviewRequest) (CleanupPreview, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := contextError(ctx); err != nil {
		return CleanupPreview{}, err
	}
	if err := validateCleanupAgent(request.Agent); err != nil {
		return CleanupPreview{}, err
	}
	if err := s.ensureCleanupStateExists(request.Agent); err != nil {
		return CleanupPreview{}, err
	}
	lock, err := acquireExistingTransactionLock(ctx, s.stateDir)
	if err != nil {
		if CodeOf(err) != CodeOperationBusy {
			return CleanupPreview{}, operationError(CodeModelStateInvalid, "Agent cleanup coordination state is invalid")
		}
		return CleanupPreview{}, err
	}
	defer lock.release()
	if err := contextError(ctx); err != nil {
		return CleanupPreview{}, err
	}
	if err := s.ensureCleanupStateExists(request.Agent); err != nil {
		return CleanupPreview{}, err
	}
	if s.writesDisabled {
		return CleanupPreview{}, operationError(CodeRollbackFailed, "Agent writes are disabled because transaction recovery failed")
	}
	if err := s.ensureExistingSigner(); err != nil {
		return CleanupPreview{}, err
	}
	plan, err := s.buildCleanupPlan(request.Agent)
	if err != nil {
		return CleanupPreview{}, err
	}
	return s.cleanupPreviewForPlan(plan)
}

// CleanupWrite re-creates and verifies the signed cleanup plan before routing
// it through the normal backup, journal, apply, and rollback machinery.
func (s *Service) CleanupWrite(ctx context.Context, request CleanupWriteRequest) (WriteResult, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := contextError(ctx); err != nil {
		return WriteResult{}, err
	}
	if err := validateCleanupAgent(request.Agent); err != nil {
		return WriteResult{}, err
	}
	if err := s.ensureCleanupStateExists(request.Agent); err != nil {
		return WriteResult{}, err
	}
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return WriteResult{}, err
	}
	defer lock.release()
	if err := contextError(ctx); err != nil {
		return WriteResult{}, err
	}
	if err := s.ensureCleanupStateExists(request.Agent); err != nil {
		return WriteResult{}, err
	}
	if s.writesDisabled {
		return WriteResult{}, operationError(CodeRollbackFailed, "Agent writes are disabled because transaction recovery failed")
	}
	if request.RevisionToken == "" {
		return WriteResult{}, operationError(CodeInvalidParams, "cleanup revision token is required")
	}
	if err := s.ensureExistingSigner(); err != nil {
		return WriteResult{}, err
	}
	claims, err := s.signer.VerifyCleanupRevision(request.RevisionToken)
	if err != nil || claims.Agent != modelAgent(request.Agent) {
		return WriteResult{}, operationError(CodePreviewStale, "Agent cleanup preview is stale")
	}
	if err := s.verifyKeyedRevision(s.sidecarPath(), fileRevisionFromClaim(claims.StateRevision), revisionContextSidecar); err != nil {
		return WriteResult{}, operationError(CodePreviewStale, "Agent cleanup state changed after preview")
	}
	for _, file := range claims.Files {
		if err := s.verifyKeyedRevision(file.SourcePath, fileRevisionFromClaim(file.SourceRevision), revisionContextAgentFile); err != nil {
			return WriteResult{}, operationError(CodePreviewStale, "Agent configuration changed after cleanup preview")
		}
	}
	plan, err := s.buildCleanupPlan(request.Agent)
	if err != nil {
		return WriteResult{}, err
	}
	currentToken, err := s.cleanupTokenForPlan(plan)
	if err != nil || subtle.ConstantTimeCompare([]byte(currentToken), []byte(request.RevisionToken)) != 1 {
		return WriteResult{}, operationError(CodePreviewStale, "Agent cleanup preview is stale")
	}
	if plan.ManagedConfigDrift() != request.ApproveManagedOverwrite {
		return WriteResult{}, operationError(CodeManagedConfigDrift, "managed Agent configuration drift approval does not match preview")
	}
	if err := contextError(ctx); err != nil {
		return WriteResult{}, err
	}
	return s.executePlan(ctx, plan.writePlan, nil)
}

func validateCleanupAgent(kind Kind) error {
	switch kind {
	case ClaudeCode, OpenCode, Codex:
		return nil
	default:
		return operationError(CodeInvalidParams, "unsupported Agent cleanup selection")
	}
}

func (s *Service) ensureCleanupStateExists(kind Kind) error {
	if _, err := os.Stat(s.sidecarPath()); os.IsNotExist(err) {
		return operationError(CodeAgentNotManaged, agentName(kind)+" is not managed by CodeasierRouter")
	} else if err != nil {
		return operationError(CodeModelStateInvalid, "Agent model state cannot be read")
	}
	return nil
}

func (s *Service) buildCleanupPlan(kind Kind) (cleanupPlan, error) {
	state, stateRevision, stateContent, stateMode, err := s.readSidecar()
	if err != nil {
		return cleanupPlan{}, err
	}
	section, managed := state.Agents[kind]
	if !managed {
		return cleanupPlan{}, operationError(CodeAgentNotManaged, agentName(kind)+" is not managed by CodeasierRouter")
	}
	saved, err := decodeCleanupModelConfig(kind, section.ModelConfig)
	if err != nil {
		return cleanupPlan{}, operationError(CodeModelStateInvalid, "saved Agent model state is invalid")
	}
	plan := cleanupPlan{
		writePlan: writePlan{
			agents: []agentPlan{{kind: kind, mode: ConfigModeMerge}}, selected: []Kind{kind},
			sidecar: state, sidecarRevision: stateRevision, sidecarContent: stateContent, sidecarMode: stateMode,
		},
		removedByPath: make(map[string][]string),
	}
	for _, recorded := range section.Files {
		file, removed, drifted, err := s.cleanupFilePlan(kind, recorded, section, saved)
		if err != nil {
			return cleanupPlan{}, err
		}
		if drifted {
			plan.drifted = []Kind{kind}
		}
		if len(removed) == 0 {
			continue
		}
		plan.files = append(plan.files, file)
		plan.removedByPath[file.targetPath] = removed
	}
	delete(state.Agents, kind)
	plan.sidecar = state
	if len(state.Agents) == 0 {
		plan.stateOperation = OperationDelete
		plan.files = append(plan.files, plannedFile{
			scope: scopeManagerState, role: "state", format: FormatJSON,
			sourcePath: s.sidecarPath(), targetPath: s.sidecarPath(), operation: OperationDelete,
			sourceRevision: stateRevision, targetRevision: stateRevision, sourceContent: stateContent,
			sourceMode: stateMode, targetMode: stateMode, backupRequired: true,
			backupSource: s.sidecarPath(), restoreFrom: s.sidecarPath(),
		})
		return plan, nil
	}
	stateOutput, err := modelconfig.CanonicalValue(state)
	if err != nil || len(stateOutput) > maxSidecarSize {
		return cleanupPlan{}, operationError(CodeModelStateInvalid, "Agent model state could not be updated")
	}
	plan.stateOperation = OperationReplace
	plan.files = append(plan.files, plannedFile{
		scope: scopeManagerState, role: "state", format: FormatJSON,
		sourcePath: s.sidecarPath(), targetPath: s.sidecarPath(), operation: OperationReplace,
		sourceRevision: stateRevision, targetRevision: stateRevision, sourceContent: stateContent,
		sourceMode: stateMode, targetMode: stateMode, backupRequired: true,
		backupSource: s.sidecarPath(), restoreFrom: s.sidecarPath(),
		render: func([]byte) ([]byte, error) { return append([]byte(nil), stateOutput...), nil },
	})
	return plan, nil
}

func decodeCleanupModelConfig(kind Kind, raw json.RawMessage) (*modelconfig.Config, error) {
	document, err := modelconfig.CanonicalValue(map[string]any{
		"version":                modelconfig.Version,
		string(modelAgent(kind)): json.RawMessage(raw),
	})
	if err != nil {
		return nil, err
	}
	return modelconfig.DecodeStructural(document)
}

func (s *Service) cleanupFilePlan(kind Kind, recorded lastAppliedFile, section lastAppliedAgent, saved *modelconfig.Config) (plannedFile, []string, bool, error) {
	safety := inspectRecoveryTarget(recorded.Role, recorded.Path, cleanupFileFormat(kind, recorded))
	if len(safety.Reasons) != 0 {
		if containsRecoveryReason(safety.Reasons, RecoveryNotWritable) || containsRecoveryReason(safety.Reasons, RecoveryParentUnavailable) {
			return plannedFile{}, nil, false, operationError(CodeConfigNotWritable, agentName(kind)+" managed configuration is not writable")
		}
		return plannedFile{}, nil, false, operationError(CodeConfigInvalid, agentName(kind)+" managed configuration path is unsafe")
	}
	revision, content, mode, err := s.readKeyedRevision(recorded.Path, revisionContextAgentFile)
	if err != nil || !revision.Exists {
		return plannedFile{}, nil, false, operationError(CodeConfigInvalid, agentName(kind)+" managed configuration cannot be read")
	}
	if err := ensureWritable(recorded.Path); err != nil {
		return plannedFile{}, nil, false, err
	}
	var transform cleanupTransform
	format := FormatJSON
	switch kind {
	case ClaudeCode:
		root, valid := decodeObject(content)
		if !valid {
			return plannedFile{}, nil, false, operationError(CodeConfigInvalid, "Claude Code configuration is not a JSON object")
		}
		transform, err = cleanupClaude(root, section.OwnedPaths)
	case OpenCode:
		parsed := content
		if filepath.Ext(recorded.Path) == ".jsonc" {
			format = FormatJSONC
			parsed, err = stripJSONC(content)
			if err != nil {
				return plannedFile{}, nil, false, operationError(CodeConfigInvalid, "opencode configuration is invalid JSONC")
			}
		}
		root, valid := decodeObject(parsed)
		if !valid {
			return plannedFile{}, nil, false, operationError(CodeConfigInvalid, "opencode configuration is not a JSON object")
		}
		transform, err = cleanupOpenCode(root, containsString(section.OwnedPaths, "model"))
	case Codex:
		if recorded.Role == "config" {
			format = FormatTOML
			transform, err = cleanupCodexConfig(content, saved.Codex)
		} else {
			root, valid := decodeObject(content)
			if !valid {
				return plannedFile{}, nil, false, operationError(CodeConfigInvalid, "Codex auth configuration is not a JSON object")
			}
			transform, err = cleanupCodexAuth(root)
		}
	}
	if err != nil {
		return plannedFile{}, nil, false, err
	}
	operation := OperationReplace
	if transform.Delete {
		operation = OperationDelete
	}
	output := append([]byte(nil), transform.Content...)
	file := plannedFile{
		agent: kind, mode: ConfigModeMerge, role: recorded.Role, format: format, scope: scopeAgent,
		sourcePath: recorded.Path, targetPath: recorded.Path, operation: operation,
		containsAPIKey: kind != Codex || recorded.Role == "auth",
		sourceRevision: revision, targetRevision: revision, sourceContent: content,
		sourceMode: mode, targetMode: mode, backupRequired: true,
		backupSource: recorded.Path, restoreFrom: recorded.Path,
	}
	if operation == OperationReplace {
		file.render = func([]byte) ([]byte, error) { return append([]byte(nil), output...), nil }
	}
	return file, transform.RemovedPaths, revisionTokenValue(revision) != recorded.RevisionMAC, nil
}

func cleanupFileFormat(kind Kind, recorded lastAppliedFile) Format {
	if kind == Codex && recorded.Role == "config" {
		return FormatTOML
	}
	if kind == OpenCode && filepath.Ext(recorded.Path) == ".jsonc" {
		return FormatJSONC
	}
	return FormatJSON
}

func (p cleanupPlan) ManagedConfigDrift() bool { return len(p.drifted) != 0 }

func (s *Service) cleanupTokenForPlan(plan cleanupPlan) (string, error) {
	files := make([]modelconfig.CleanupRevisionFile, 0, len(plan.files)-1)
	for _, file := range plan.files {
		if file.scope == scopeManagerState {
			continue
		}
		files = append(files, modelconfig.CleanupRevisionFile{
			Role: file.role, SourcePath: filepath.Clean(file.sourcePath), TargetPath: filepath.Clean(file.targetPath),
			Operation: string(file.operation), BackupRequired: file.backupRequired, BackupSource: file.backupSource,
			SourceRevision: revisionClaim(file.sourceRevision), TargetRevision: revisionClaim(file.targetRevision),
			RemovedPaths: append([]string(nil), plan.removedByPath[file.targetPath]...),
		})
	}
	return s.signer.SignCleanupRevision(modelconfig.CleanupRevisionClaims{
		Agent: modelAgent(plan.selected[0]), Files: files, StateOperation: string(plan.stateOperation),
		StateRevision: revisionClaim(plan.sidecarRevision), ManagedConfigDrift: plan.ManagedConfigDrift(),
	})
}

func fileRevisionFromClaim(value modelconfig.RevisionState) fileRevision {
	return fileRevision{Exists: value.Exists, Size: value.Size, Mode: value.Mode, Digest: value.Digest}
}

func (s *Service) cleanupPreviewForPlan(plan cleanupPlan) (CleanupPreview, error) {
	token, err := s.cleanupTokenForPlan(plan)
	if err != nil {
		return CleanupPreview{}, operationError(CodeModelStateInvalid, "Agent cleanup revision token could not be created")
	}
	preview := CleanupPreview{
		RevisionToken: token, Agent: plan.selected[0], ManagedConfigDrift: plan.ManagedConfigDrift(),
		StateChange: &FilePreview{
			Role: "state", Path: s.sidecarPath(), Format: FormatJSON, Operation: plan.stateOperation,
			Operations: []Operation{plan.stateOperation}, Backup: cleanupBackupPlan(s.sidecarPath()),
		},
		StateBackup: &FilePreview{
			Role: "state", Path: s.sidecarPath(), Format: FormatJSON, Operation: Operation("backup"),
			Operations: []Operation{Operation("backup")}, Backup: cleanupBackupPlan(s.sidecarPath()),
		},
	}
	for _, file := range plan.files {
		if file.scope == scopeManagerState {
			continue
		}
		removed := plan.removedByPath[file.targetPath]
		preview.RemovedPaths = append(preview.RemovedPaths, removed...)
		preview.Files = append(preview.Files, FilePreview{
			Role: file.role, Path: file.targetPath, Format: file.format, Operation: file.operation,
			Operations: []Operation{file.operation}, ContainsAPIKey: file.containsAPIKey,
			Backup: cleanupBackupPlan(file.backupSource),
		})
	}
	sort.Strings(preview.RemovedPaths)
	preview.RemovedPaths = uniqueSortedStrings(preview.RemovedPaths)
	return preview, nil
}

func cleanupBackupPlan(path string) BackupPlan {
	return BackupPlan{
		Required: true, Sensitive: true, Pattern: path + ".bak-<timestamp>-<random>",
	}
}

func uniqueSortedStrings(values []string) []string {
	if len(values) == 0 {
		return values
	}
	result := values[:1]
	for _, value := range values[1:] {
		if value != result[len(result)-1] {
			result = append(result, value)
		}
	}
	return result
}

type cleanupTransform struct {
	Content      []byte
	Delete       bool
	RemovedPaths []string
}

func newCleanupTransform(content []byte, empty bool, removed []string) cleanupTransform {
	sort.Strings(removed)
	unique := removed[:0]
	for _, path := range removed {
		if len(unique) == 0 || unique[len(unique)-1] != path {
			unique = append(unique, path)
		}
	}
	if empty {
		content = nil
	}
	return cleanupTransform{Content: content, Delete: empty, RemovedPaths: unique}
}

func marshalCleanupJSON(value any) ([]byte, error) {
	var output bytes.Buffer
	encoder := json.NewEncoder(&output)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return nil, err
	}
	return output.Bytes(), nil
}
