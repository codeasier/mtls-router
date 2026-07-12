package agent

import (
	"context"
	"crypto/hmac"
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
)

const journalFileName = "agent-write-journal.json"

// ErrorCode identifies an Agent operation failure without requiring callers to
// branch on localized or platform-specific error text.
type ErrorCode string

const (
	CodeInvalidParams     ErrorCode = "INVALID_PARAMS"
	CodeAgentNotFound     ErrorCode = "AGENT_NOT_FOUND"
	CodeConfigInvalid     ErrorCode = "CONFIG_INVALID"
	CodeConfigNotWritable ErrorCode = "CONFIG_NOT_WRITABLE"
	CodePreviewStale      ErrorCode = "PREVIEW_STALE"
	CodeBackupFailed      ErrorCode = "BACKUP_FAILED"
	CodeWriteFailed       ErrorCode = "WRITE_FAILED"
	CodeRollbackFailed    ErrorCode = "ROLLBACK_FAILED"
	CodeOperationTimeout  ErrorCode = "OPERATION_TIMEOUT"
)

// OperationError is safe to return through the management protocol. It never
// includes configuration contents or an API key.
type OperationError struct {
	Code ErrorCode
	msg  string
}

func (e *OperationError) Error() string { return e.msg }

func operationError(code ErrorCode, message string) error {
	return &OperationError{Code: code, msg: message}
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

// Preview is an immutable approval boundary. The token is bound to the
// selected Agents and every source and target revision observed by Preview.
type Preview struct {
	RevisionToken string         `json:"revision_token"`
	Agents        []AgentPreview `json:"agents"`
	Warnings      []string       `json:"warnings,omitempty"`
}

// WriteRequest contains the transient secret and approved revision. APIKey is
// consumed only in memory and is never included in results or journal state.
type WriteRequest struct {
	Agents        []Kind
	RevisionToken string
	APIKey        string
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
}

// Options configures an Agent service. StateDir stores only the key-free
// recovery journal. Zero-value Detector fields retain their normal semantics.
type Options struct {
	StateDir string
	Detector Detector
}

// Service serializes previews, writes, and recovery for one manager process.
type Service struct {
	mu             sync.Mutex
	stateDir       string
	detector       Detector
	revisionKey    [32]byte
	writesDisabled bool
	recoveryErr    error
	hooks          serviceHooks
}

type serviceHooks struct {
	beforeBackup   func(string) error
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
	service.revisionKey = sha256.Sum256([]byte("mtls-router-agent-revision-v1\x00" + service.stateDir))
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
	return s.writesDisabled
}

// RecoveryError returns the sanitized startup recovery failure, if any.
func (s *Service) RecoveryError() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.recoveryErr
}

// Preview validates selected targets and returns a structured, key-free change
// set. It performs no writes, including directory or backup creation.
func (s *Service) Preview(ctx context.Context, selected []Kind) (Preview, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := contextError(ctx); err != nil {
		return Preview{}, err
	}
	plan, err := s.buildPlan(selected)
	if err != nil {
		return Preview{}, err
	}
	return s.previewForPlan(plan), nil
}

// Write revalidates the approved preview before creating any backup or
// replacing any target. Any failure after replacement starts triggers reverse
// order rollback before Write returns.
func (s *Service) Write(ctx context.Context, request WriteRequest) (WriteResult, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if s.writesDisabled {
		return WriteResult{}, operationError(CodeRollbackFailed, "Agent writes are disabled because transaction recovery failed")
	}
	if request.APIKey == "" || request.RevisionToken == "" {
		return WriteResult{}, operationError(CodeInvalidParams, "API key and revision token are required")
	}
	key := []byte(request.APIKey)
	defer zeroBytes(key)

	plan, err := s.buildPlan(request.Agents)
	if err != nil {
		return WriteResult{}, err
	}
	currentToken := s.tokenForPlan(plan)
	if subtle.ConstantTimeCompare([]byte(currentToken), []byte(request.RevisionToken)) != 1 {
		return WriteResult{}, operationError(CodePreviewStale, "Agent configuration changed after preview")
	}
	if err := contextError(ctx); err != nil {
		return WriteResult{}, err
	}
	for i := range plan.files {
		file := &plan.files[i]
		if err := verifyRevision(file.sourcePath, file.sourceRevision); err != nil {
			return WriteResult{}, operationError(CodePreviewStale, "Agent configuration changed after preview")
		}
		if file.sourcePath != file.targetPath {
			if err := verifyRevision(file.targetPath, file.targetRevision); err != nil {
				return WriteResult{}, operationError(CodePreviewStale, "Agent configuration target changed after preview")
			}
		}
	}
	if err := s.prepareStateDir(); err != nil {
		return WriteResult{}, operationError(CodeWriteFailed, "could not prepare Agent transaction state")
	}

	transactionID, err := randomID()
	if err != nil {
		return WriteResult{}, operationError(CodeWriteFailed, "could not initialize Agent transaction")
	}
	result := resultForPlan(transactionID, plan)
	journal := transactionJournal{Version: 1, TransactionID: transactionID, Entries: make([]journalEntry, len(plan.files))}

	for i := range plan.files {
		file := &plan.files[i]
		if err := contextError(ctx); err != nil {
			markResultFailure(&result, err)
			return result, err
		}
		if file.backupRequired {
			if s.hooks.beforeBackup != nil {
				if err := s.hooks.beforeBackup(file.backupSource); err != nil {
					failure := operationError(CodeBackupFailed, "could not create an Agent configuration backup")
					markResultFailure(&result, failure)
					return result, failure
				}
			}
			backupPath, err := createPrivateBackup(file.backupSource, file.sourceContent, file.sourceMode, "bak")
			if err != nil {
				failure := operationError(CodeBackupFailed, "could not create an Agent configuration backup")
				markResultFailure(&result, failure)
				return result, failure
			}
			file.backupPath = backupPath
			appendAgentBackup(&result, file.agent, file.targetPath, backupPath)
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
		postRevision := revisionForContent(output, mode)
		zeroBytes(output)
		journal.Entries[i] = journalEntry{
			Agent:        file.agent,
			TargetPath:   file.targetPath,
			PreRevision:  file.targetRevision,
			PostRevision: postRevision,
			BackupPath:   file.backupPath,
			RestoreFrom:  file.restoreFrom,
			TargetMode:   uint32(file.targetMode.Perm()),
			Progress:     progressPending,
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
		if err := verifyRevision(file.sourcePath, file.sourceRevision); err != nil {
			failure := operationError(CodePreviewStale, "Agent configuration changed while writing")
			return s.failAndRollback(ctx, &journal, result, failure, replaced)
		}
		if file.sourcePath != file.targetPath {
			if err := verifyRevision(file.targetPath, file.targetRevision); err != nil {
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
				markFileReplaced(&result, file.agent, file.targetPath)
			}
			failure := operationError(CodeWriteFailed, "could not replace an Agent configuration file")
			return s.failAndRollback(ctx, &journal, result, failure, replaced)
		}
		zeroBytes(output)
		replaced = true
		markFileReplaced(&result, file.agent, file.targetPath)
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

// Recover completes startup recovery for an existing service. A prior recovery
// failure remains fail-closed unless a later call proves full restoration.
func (s *Service) Recover(ctx context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
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

func (s *Service) tokenForPlan(plan writePlan) string {
	hash := hmac.New(sha256.New, s.revisionKey[:])
	for _, kind := range plan.selected {
		fmt.Fprintf(hash, "agent:%s\x00", kind)
	}
	for _, file := range plan.files {
		fmt.Fprintf(hash, "source:%s\x00", filepath.Clean(file.sourcePath))
		file.sourceRevision.writeTo(hash)
		fmt.Fprintf(hash, "target:%s\x00", filepath.Clean(file.targetPath))
		file.targetRevision.writeTo(hash)
	}
	return hex.EncodeToString(hash.Sum(nil))
}

func (s *Service) previewForPlan(plan writePlan) Preview {
	preview := Preview{RevisionToken: s.tokenForPlan(plan), Agents: make([]AgentPreview, 0, len(plan.selected))}
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
	return preview
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
	if journal.Version != 1 || journal.TransactionID == "" || len(journal.Entries) == 0 {
		return transactionJournal{}, errors.New("invalid journal")
	}
	for _, entry := range journal.Entries {
		if entry.TargetPath == "" || !filepath.IsAbs(entry.TargetPath) || entry.Agent == "" {
			return transactionJournal{}, errors.New("invalid journal entry")
		}
		if entry.PreRevision.Exists && entry.BackupPath == "" {
			return transactionJournal{}, errors.New("missing recovery backup")
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
		if err := verifyRevision(entry.TargetPath, entry.PreRevision); err == nil {
			entry.Progress = progressRestored
			if err := s.writeJournal(*journal); err != nil {
				return err
			}
			continue
		}
		if err := verifyRevision(entry.TargetPath, entry.PostRevision); err != nil {
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
		currentRevision, currentContent, currentMode, readErr := readRevision(entry.TargetPath)
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
			if result != nil {
				appendRollbackBackup(result, entry.Agent, entry.TargetPath, rollbackBackup)
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
			digest := sha256.Sum256(backupContent)
			if int64(len(backupContent)) != entry.PreRevision.Size || hex.EncodeToString(digest[:]) != entry.PreRevision.Digest {
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
		if err := verifyRevision(entry.TargetPath, entry.PreRevision); err != nil {
			return err
		}
		entry.Progress = progressRestored
		if result != nil {
			markFileRestored(result, entry.Agent, entry.TargetPath)
		}
		if err := s.writeJournal(*journal); err != nil {
			return err
		}
	}
	if conflict != nil {
		return conflict
	}
	for _, entry := range journal.Entries {
		if err := verifyRevision(entry.TargetPath, entry.PreRevision); err != nil {
			return err
		}
	}
	return removeAndSync(s.journalPath())
}

type journalProgress string

const (
	progressPending  journalProgress = "pending"
	progressReplaced journalProgress = "replaced"
	progressRestored journalProgress = "restored"
)

type transactionJournal struct {
	Version       int            `json:"version"`
	TransactionID string         `json:"transaction_id"`
	Committed     bool           `json:"committed,omitempty"`
	Entries       []journalEntry `json:"entries"`
}

type journalEntry struct {
	Agent              Kind            `json:"agent"`
	TargetPath         string          `json:"target_path"`
	PreRevision        fileRevision    `json:"pre_revision"`
	PostRevision       fileRevision    `json:"post_revision"`
	BackupPath         string          `json:"backup_path,omitempty"`
	RestoreFrom        string          `json:"restore_from,omitempty"`
	RollbackBackupPath string          `json:"rollback_backup_path,omitempty"`
	TargetMode         uint32          `json:"target_mode"`
	Progress           journalProgress `json:"progress"`
}
