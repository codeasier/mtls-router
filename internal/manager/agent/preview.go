package agent

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const (
	jsoncMigrationWarning = "opencode.jsonc will be migrated to opencode.json; comments and formatting will not be preserved"
	jsoncOverrideWarning  = "OPENCODE_CONFIG JSONC will be normalized to strict JSON in place; comments and formatting will not be preserved"
	backupWarning         = "Backups are sensitive recovery artifacts and may contain a previous API key"
	rebuildWarning        = "Rebuild replaces the complete Agent configuration; unrelated settings, comments, and formatting will not be preserved"
)

type writePlan struct {
	agents                    []agentPlan
	selected                  []Kind
	files                     []plannedFile
	input                     renderInput
	catalog                   modelconfig.CatalogClaims
	canonical                 json.RawMessage
	sidecar                   lastAppliedState
	sidecarRevision           fileRevision
	sidecarContent            []byte
	sidecarMode               os.FileMode
	drifted                   []Kind
	collisions                []ManagedCollision
	requiresCodexAuthApproval bool
}

type plannedFile struct {
	agent           Kind
	mode            ConfigMode
	format          Format
	sourcePath      string
	targetPath      string
	operation       Operation
	containsAPIKey  bool
	preserves       []string
	warning         string
	sourceRevision  fileRevision
	targetRevision  fileRevision
	sourceContent   []byte
	sourceMode      os.FileMode
	targetMode      os.FileMode
	backupRequired  bool
	backupSource    string
	backupPath      string
	restoreFrom     string
	render          func([]byte) ([]byte, error)
	role            string
	companionExists bool
	syntaxInvalid   bool
	scope           journalScope
}

type journalScope string

const (
	scopeAgent        journalScope = "agent"
	scopeManagerState journalScope = "manager_state"
)

func (s *Service) buildPlan(agents []agentPlan, input renderInput, catalog modelconfig.CatalogClaims, canonical json.RawMessage) (writePlan, error) {
	if input.config == nil {
		return writePlan{}, operationError(CodeInvalidParams, "canonical model configuration is required")
	}
	selected := make([]Kind, len(agents))
	for i, item := range agents {
		selected[i] = item.kind
	}
	states, err := s.detectLocked()
	if err != nil {
		return writePlan{}, operationError(CodeAgentNotFound, "could not detect supported Agents")
	}
	byKind := orderedStates(states)
	sidecar, sidecarRevision, sidecarContent, sidecarMode, err := s.readSidecar()
	if err != nil {
		return writePlan{}, err
	}
	plan := writePlan{agents: agents, selected: selected, input: input, catalog: catalog, canonical: canonical, sidecar: sidecar, sidecarRevision: sidecarRevision, sidecarContent: sidecarContent, sidecarMode: sidecarMode}
	s.currentSidecar = sidecar
	defer func() { s.currentSidecar = lastAppliedState{} }()
	for _, item := range agents {
		kind := item.kind
		state, ok := byKind[kind]
		if !ok || !state.Detected {
			return writePlan{}, operationError(CodeAgentNotFound, agentName(kind)+" is not detected")
		}
		var files []plannedFile
		if item.mode == ConfigModeRebuild {
			files, err = s.planRebuild(state)
		} else {
			if state.Invalid {
				return writePlan{}, operationError(CodeConfigInvalid, agentName(kind)+" configuration is invalid")
			}
			switch kind {
			case ClaudeCode:
				files, err = s.planClaude(state)
			case OpenCode:
				files, err = s.planOpenCode(state)
			case Codex:
				files, err = s.planCodex(state)
			}
		}
		if err != nil {
			return writePlan{}, err
		}
		for i := range files {
			files[i].mode = item.mode
			files[i].scope = scopeAgent
			if files[i].role == "" {
				files[i].role = "config"
			}
		}
		plan.files = append(plan.files, files...)
	}
	s.inspectOwnership(&plan)
	return plan, nil
}

func (s *Service) planRebuild(state State) ([]plannedFile, error) {
	if err := validateRebuildState(state); err != nil {
		return nil, err
	}
	files := make([]plannedFile, 0, len(state.Recovery.Files))
	for _, recovery := range state.Recovery.Files {
		revision, content, mode, err := s.readKeyedRevision(recovery.Path, revisionContextAgentFile)
		if err != nil || revision.Exists != recovery.Exists {
			return nil, operationError(CodeConfigInvalid, agentName(state.Agent)+" recovery state is unavailable")
		}
		operation := OperationCreate
		if revision.Exists {
			operation = OperationReplace
		}
		file := plannedFile{
			agent: state.Agent, mode: ConfigModeRebuild, role: recovery.Role, format: recovery.Format,
			sourcePath: recovery.Path, targetPath: recovery.Path, operation: operation,
			containsAPIKey: recovery.Role == "auth" || state.Agent != Codex, warning: rebuildWarning,
			sourceRevision: revision, targetRevision: revision, sourceContent: content,
			sourceMode: mode, targetMode: mode, backupRequired: revision.Exists,
			backupSource: recovery.Path, restoreFrom: recovery.Path,
		}
		file.syntaxInvalid = containsRecoveryReason(recovery.Reasons, RecoverySyntaxInvalid)
		if !revision.Exists {
			file.targetMode = 0o600
		}
		switch state.Agent {
		case ClaudeCode:
			file.render = func(key []byte) ([]byte, error) {
				return renderClaudeFragment(s.currentInput.config.Claude, s.currentInput.routerBaseURL, string(key))
			}
		case OpenCode:
			file.format = FormatJSON
			file.render = func(key []byte) ([]byte, error) {
				return renderOpenCodeFragment(s.currentInput.config.OpenCode, s.currentInput.apiBaseURL, string(key))
			}
		case Codex:
			if recovery.Role == "config" {
				file.render = func([]byte) ([]byte, error) {
					return renderCodexFragment(s.currentInput.config.Codex, s.currentInput.apiBaseURL)
				}
			} else {
				file.render = func(key []byte) ([]byte, error) { return renderCodexAuthFragment(string(key), nil) }
			}
		}
		files = append(files, file)
	}
	if state.Agent == Codex && len(files) == 2 {
		files[0].companionExists = files[1].sourceRevision.Exists
		files[1].companionExists = files[0].sourceRevision.Exists
	}
	return files, nil
}

func validateRebuildState(state State) error {
	if !state.Detected || !state.Invalid || !state.Recovery.Eligible {
		return operationError(CodeConfigInvalid, agentName(state.Agent)+" configuration is not eligible for rebuild")
	}
	expected := []RecoveryFileState{{Role: "config", Path: state.Path, Format: state.Format}}
	if state.Agent == Codex {
		expected = []RecoveryFileState{{Role: "config", Path: state.Path, Format: FormatTOML}, {Role: "auth", Path: state.AuthPath, Format: FormatJSON}}
	}
	if len(state.Recovery.Files) != len(expected) {
		return operationError(CodeConfigInvalid, agentName(state.Agent)+" recovery metadata is invalid")
	}
	for _, reason := range state.Recovery.Reasons {
		if reason != RecoverySyntaxInvalid {
			return operationError(CodeConfigInvalid, agentName(state.Agent)+" recovery metadata is unsafe")
		}
	}
	syntaxInvalid := false
	for i, file := range state.Recovery.Files {
		want := expected[i]
		if file.Role != want.Role || filepath.Clean(file.Path) != filepath.Clean(want.Path) || file.Format != want.Format {
			return operationError(CodeConfigInvalid, agentName(state.Agent)+" recovery metadata is invalid")
		}
		for _, reason := range file.Reasons {
			if reason == RecoverySyntaxInvalid {
				syntaxInvalid = true
			} else {
				return operationError(CodeConfigInvalid, agentName(state.Agent)+" recovery metadata is unsafe")
			}
		}
	}
	if !syntaxInvalid {
		return operationError(CodeConfigInvalid, agentName(state.Agent)+" configuration is not syntax-invalid")
	}
	return nil
}

func (s *Service) planClaude(state State) ([]plannedFile, error) {
	if err := ensureWritable(state.Path); err != nil {
		return nil, err
	}
	revision, content, mode, err := s.readKeyedRevision(state.Path, revisionContextAgentFile)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "Claude Code configuration cannot be read")
	}
	root := make(map[string]json.RawMessage)
	if revision.Exists {
		var valid bool
		root, valid = decodeObject(content)
		if !valid {
			return nil, operationError(CodeConfigInvalid, "Claude Code configuration is not a JSON object")
		}
	}
	operation := OperationCreate
	if revision.Exists {
		operation = OperationReplace
	}
	file := plannedFile{
		agent: ClaudeCode, format: FormatJSON, sourcePath: state.Path, targetPath: state.Path,
		operation: operation, containsAPIKey: true, preserves: []string{"all non-env top-level fields"},
		sourceRevision: revision, targetRevision: revision, sourceContent: content,
		sourceMode: mode, targetMode: mode, backupRequired: revision.Exists,
		backupSource: state.Path, restoreFrom: state.Path,
	}
	obsoleteOwnedEnv := s.obsoleteClaudeEnv()
	file.render = func(key []byte) ([]byte, error) {
		return mergeClaude(root, s.currentInput.config.Claude, s.currentInput.routerBaseURL, string(key), obsoleteOwnedEnv)
	}
	return []plannedFile{file}, nil
}

func (s *Service) planOpenCode(state State) ([]plannedFile, error) {
	sourcePath := state.Path
	targetPath := state.Path
	format := state.Format
	isJSONC := filepath.Ext(sourcePath) == ".jsonc"
	if isJSONC {
		format = FormatJSON
		if !state.pathOverridden {
			targetPath = filepath.Join(filepath.Dir(sourcePath), "opencode.json")
			if _, sourceErr := os.Lstat(sourcePath); os.IsNotExist(sourceErr) {
				if info, targetErr := os.Stat(targetPath); targetErr == nil && info.Mode().IsRegular() {
					sourcePath = targetPath
				} else if targetErr != nil && !os.IsNotExist(targetErr) {
					return nil, operationError(CodeConfigInvalid, "opencode target cannot be inspected")
				}
			} else if sourceErr != nil {
				return nil, operationError(CodeConfigInvalid, "opencode JSONC source cannot be inspected")
			} else if _, targetErr := os.Lstat(targetPath); targetErr == nil || !os.IsNotExist(targetErr) {
				return nil, operationError(CodeConfigInvalid, "opencode JSONC migration target already exists")
			}
		}
	}
	if err := ensureWritable(sourcePath); err != nil {
		return nil, err
	}
	if err := ensureWritable(targetPath); err != nil {
		return nil, err
	}
	sourceRevision, sourceContent, sourceMode, err := s.readKeyedRevision(sourcePath, revisionContextAgentFile)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "opencode configuration cannot be read")
	}
	parsed := sourceContent
	if isJSONC && sourceRevision.Exists {
		parsed, err = stripJSONC(sourceContent)
		if err != nil {
			return nil, operationError(CodeConfigInvalid, "opencode JSONC configuration is invalid")
		}
	}
	root := make(map[string]json.RawMessage)
	if sourceRevision.Exists {
		var valid bool
		root, valid = decodeObject(parsed)
		if !valid {
			return nil, operationError(CodeConfigInvalid, "opencode configuration is not a JSON object")
		}
		if provider, exists := root["provider"]; exists && !bytes.Equal(bytes.TrimSpace(provider), []byte("null")) {
			if _, valid := decodeObject(provider); !valid {
				return nil, operationError(CodeConfigInvalid, "opencode provider field is not an object")
			}
		}
	}
	targetRevision, _, targetMode, err := s.readKeyedRevision(targetPath, revisionContextAgentFile)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "opencode target cannot be read")
	}
	operation := OperationCreate
	if targetRevision.Exists {
		operation = OperationReplace
	}
	warning := ""
	if isJSONC && sourceRevision.Exists {
		if state.pathOverridden {
			warning = jsoncOverrideWarning
		} else if sourcePath != targetPath {
			warning = jsoncMigrationWarning
		}
	}
	file := plannedFile{
		agent: OpenCode, format: format, sourcePath: sourcePath, targetPath: targetPath,
		operation: operation, containsAPIKey: true,
		preserves: []string{"all root fields other than provider.mtls-router", "all other providers"}, warning: warning,
		sourceRevision: sourceRevision, targetRevision: targetRevision, sourceContent: sourceContent,
		sourceMode: sourceMode, targetMode: targetMode, backupRequired: sourceRevision.Exists,
		backupSource: sourcePath, restoreFrom: sourcePath,
	}
	file.render = func(key []byte) ([]byte, error) {
		return mergeOpenCode(root, s.currentInput.config.OpenCode, s.currentInput.apiBaseURL, string(key), s.currentInput.ownRootModel)
	}
	return []plannedFile{file}, nil
}

func (s *Service) planCodex(state State) ([]plannedFile, error) {
	if err := ensureWritable(state.Path); err != nil {
		return nil, err
	}
	if err := ensureWritable(state.AuthPath); err != nil {
		return nil, err
	}
	configRevision, configContent, configMode, err := s.readKeyedRevision(state.Path, revisionContextAgentFile)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "Codex configuration cannot be read")
	}
	var codexRoot map[string]any
	if configRevision.Exists {
		var valid bool
		codexRoot, valid = decodeTOML(configContent)
		if !valid {
			return nil, operationError(CodeConfigInvalid, "Codex configuration is invalid TOML")
		}
	}
	authRevision, authContent, authMode, err := s.readKeyedRevision(state.AuthPath, revisionContextAgentFile)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "Codex auth configuration cannot be read")
	}
	if authRevision.Exists {
		if _, valid := decodeObject(authContent); !valid {
			return nil, operationError(CodeConfigInvalid, "Codex auth configuration is invalid JSON")
		}
	}
	authRoot, _ := decodeObject(authContent)
	assessment, assessErr := assessCodexMerge(codexRoot, authRoot)
	if assessErr != nil {
		return nil, assessErr
	}
	configOperation := OperationCreate
	if configRevision.Exists {
		configOperation = OperationReplace
	}
	authOperation := OperationCreate
	if authRevision.Exists {
		authOperation = OperationReplace
	}
	config := plannedFile{
		agent: Codex, format: FormatTOML, sourcePath: state.Path, targetPath: state.Path,
		operation: configOperation, preserves: []string{"unrelated root keys and sections"},
		sourceRevision: configRevision, targetRevision: configRevision, sourceContent: configContent,
		sourceMode: configMode, targetMode: configMode, backupRequired: configRevision.Exists,
		backupSource: state.Path, restoreFrom: state.Path,
	}
	config.role = "config"
	migrateHistorical := assessment.HistoricalMigration
	obsoleteOptional := s.obsoleteCodexOptional()
	config.render = func([]byte) ([]byte, error) {
		return mergeCodex(configContent, s.currentInput.config.Codex, s.currentInput.apiBaseURL, obsoleteOptional, migrateHistorical)
	}
	auth := plannedFile{
		agent: Codex, format: FormatJSON, sourcePath: state.AuthPath, targetPath: state.AuthPath,
		operation: authOperation, containsAPIKey: true,
		warning:        "Codex authentication changes to file-backed API-key login and requires separate approval",
		sourceRevision: authRevision, targetRevision: authRevision, sourceContent: authContent,
		sourceMode: authMode, targetMode: authMode, backupRequired: authRevision.Exists,
		backupSource: state.AuthPath, restoreFrom: state.AuthPath,
	}
	auth.role = "auth"
	config.companionExists = authRevision.Exists
	auth.companionExists = configRevision.Exists
	auth.render = func(key []byte) ([]byte, error) { return renderCodexAuthFragment(string(key), authRoot) }
	return []plannedFile{config, auth}, nil
}

func (s *Service) obsoleteClaudeEnv() []string {
	previous, ok := s.currentSidecar.Agents[ClaudeCode]
	if !ok {
		return nil
	}
	current := map[string]bool{}
	for _, key := range claudeOwnedEnvKeys(s.currentInput.config.Claude) {
		current["env."+key] = true
	}
	var result []string
	for _, path := range previous.OwnedPaths {
		if strings.HasPrefix(path, "env.") && !current[path] {
			result = append(result, strings.TrimPrefix(path, "env."))
		}
	}
	return result
}

func (s *Service) obsoleteCodexOptional() []string {
	previous, ok := s.currentSidecar.Agents[Codex]
	if !ok {
		return nil
	}
	current := codexManagedConfig(s.currentInput.config.Codex, s.currentInput.apiBaseURL)
	var result []string
	for _, path := range previous.OwnedPaths {
		if strings.HasPrefix(path, "model_") {
			if _, exists := current[path]; !exists {
				result = append(result, path)
			}
		}
	}
	return result
}

func (s *Service) inspectOwnership(plan *writePlan) {
	for _, item := range plan.agents {
		kind := item.kind
		if item.mode == ConfigModeRebuild {
			continue
		}
		previous, exists := plan.sidecar.Agents[kind]
		if exists {
			for _, recorded := range previous.Files {
				current, _, _, err := s.readKeyedRevision(recorded.Path, revisionContextAgentFile)
				if err != nil || revisionTokenValue(current) != recorded.RevisionMAC {
					plan.drifted = append(plan.drifted, kind)
					break
				}
			}
			if kind != ClaudeCode {
				continue
			}
		}
		for _, file := range plan.files {
			if file.agent != kind || !file.sourceRevision.Exists {
				continue
			}
			collision := false
			switch kind {
			case ClaudeCode:
				root, _ := decodeObject(file.sourceContent)
				env, _ := decodeObject(root["env"])
				previouslyOwned := map[string]bool{}
				for _, path := range previous.OwnedPaths {
					previouslyOwned[path] = true
				}
				for _, key := range claudeOwnedEnvKeys(plan.input.config.Claude) {
					if _, ok := env[key]; ok && !previouslyOwned["env."+key] {
						plan.collisions = append(plan.collisions, ManagedCollision{Agent: kind, Path: "/env/" + key, Type: "fixed_managed_path", Action: "replace"})
						collision = true
					}
				}
			case OpenCode:
				content := file.sourceContent
				if file.format == FormatJSONC {
					content, _ = stripJSONC(content)
				}
				root, _ := decodeObject(content)
				_, model := root["model"]
				providers, _ := decodeObject(root["provider"])
				_, provider := providers["mtls-router"]
				collision = model || provider
			case Codex:
				if file.role == "config" {
					root, _ := decodeTOML(file.sourceContent)
					assessment, _ := assessCodexMerge(root, nil)
					collision = assessment.ManagedConfigCollision
				}
			}
			if collision {
				if kind != ClaudeCode {
					plan.collisions = append(plan.collisions, ManagedCollision{Agent: kind, Path: managedNamespace(kind, file.role), Type: "fixed_managed_path", Action: "replace"})
				}
				plan.drifted = append(plan.drifted, kind)
				break
			}
		}
	}
	configFile := planFile(plan.files, Codex, "config")
	authFile := planFile(plan.files, Codex, "auth")
	if authFile.targetPath != "" {
		config, _ := decodeTOML(configFile.sourceContent)
		auth, _ := decodeObject(authFile.sourceContent)
		assessment, _ := assessCodexMerge(config, auth)
		plan.requiresCodexAuthApproval = assessment.RequiresAuthApproval
	}
	plan.drifted = uniqueKinds(plan.drifted)
}

func managedNamespace(kind Kind, role string) string {
	switch kind {
	case ClaudeCode:
		return "/env"
	case OpenCode:
		return "/provider/mtls-router"
	default:
		if role == "auth" {
			return "/auth"
		}
		return "model_providers.mtls-router"
	}
}

func planFile(files []plannedFile, kind Kind, role string) plannedFile {
	for _, file := range files {
		if file.agent == kind && file.role == role {
			return file
		}
	}
	return plannedFile{}
}

func uniqueKinds(values []Kind) []Kind {
	seen := map[Kind]bool{}
	result := []Kind{}
	for _, kind := range []Kind{ClaudeCode, OpenCode, Codex} {
		for _, value := range values {
			if value == kind && !seen[kind] {
				seen[kind] = true
				result = append(result, kind)
			}
		}
	}
	return result
}

func cloneRawObject(source map[string]json.RawMessage) map[string]json.RawMessage {
	result := make(map[string]json.RawMessage, len(source)+1)
	for key, value := range source {
		result[key] = append(json.RawMessage(nil), value...)
	}
	return result
}

func marshalObject(value map[string]json.RawMessage) ([]byte, error) {
	return marshalIndentedJSON(value)
}
