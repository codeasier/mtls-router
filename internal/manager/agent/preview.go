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
)

type writePlan struct {
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
	agent          Kind
	format         Format
	sourcePath     string
	targetPath     string
	operation      Operation
	containsAPIKey bool
	preserves      []string
	warning        string
	sourceRevision fileRevision
	targetRevision fileRevision
	sourceContent  []byte
	sourceMode     os.FileMode
	targetMode     os.FileMode
	backupRequired bool
	backupSource   string
	backupPath     string
	restoreFrom    string
	render         func([]byte) ([]byte, error)
	role           string
	scope          journalScope
}

type journalScope string

const (
	scopeAgent        journalScope = "agent"
	scopeManagerState journalScope = "manager_state"
)

func (s *Service) buildPlan(selected []Kind, input renderInput, catalog modelconfig.CatalogClaims, canonical json.RawMessage) (writePlan, error) {
	if input.config == nil {
		return writePlan{}, operationError(CodeInvalidParams, "canonical model configuration is required")
	}
	normalized, err := normalizeSelection(selected)
	if err != nil {
		return writePlan{}, err
	}
	states, err := s.detector.Detect()
	if err != nil {
		return writePlan{}, operationError(CodeAgentNotFound, "could not detect supported Agents")
	}
	byKind := orderedStates(states)
	sidecar, sidecarRevision, sidecarContent, sidecarMode, err := s.readSidecar()
	if err != nil {
		return writePlan{}, err
	}
	plan := writePlan{selected: normalized, input: input, catalog: catalog, canonical: canonical, sidecar: sidecar, sidecarRevision: sidecarRevision, sidecarContent: sidecarContent, sidecarMode: sidecarMode}
	s.currentSidecar = sidecar
	defer func() { s.currentSidecar = lastAppliedState{} }()
	for _, kind := range normalized {
		state, ok := byKind[kind]
		if !ok || !state.Detected {
			return writePlan{}, operationError(CodeAgentNotFound, agentName(kind)+" is not detected")
		}
		if state.Invalid {
			return writePlan{}, operationError(CodeConfigInvalid, agentName(kind)+" configuration is invalid")
		}
		var files []plannedFile
		switch kind {
		case ClaudeCode:
			files, err = s.planClaude(state)
		case OpenCode:
			files, err = s.planOpenCode(state)
		case Codex:
			files, err = s.planCodex(state)
		}
		if err != nil {
			return writePlan{}, err
		}
		for i := range files {
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
	file.render = func(key []byte) ([]byte, error) {
		return mergeClaude(root, s.currentInput.config.Claude, s.currentInput.routerBaseURL, string(key), s.obsoleteClaudeExtras())
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
	config.render = func([]byte) ([]byte, error) {
		return mergeCodex(configContent, s.currentInput.config.Codex, s.currentInput.apiBaseURL, s.obsoleteCodexOptional(), migrateHistorical)
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
	auth.render = func(key []byte) ([]byte, error) { return renderCodexAuthFragment(string(key), authRoot) }
	return []plannedFile{config, auth}, nil
}

func (s *Service) obsoleteClaudeExtras() []string {
	previous, ok := s.currentSidecar.Agents[ClaudeCode]
	if !ok {
		return nil
	}
	current := map[string]bool{}
	for key := range s.currentInput.config.Claude.Extra {
		current["env."+key] = true
	}
	fixed := map[string]bool{}
	for _, key := range claudeFixedEnvKeys {
		fixed["env."+key] = true
	}
	var result []string
	for _, path := range previous.OwnedPaths {
		if strings.HasPrefix(path, "env.") && !fixed[path] && !current[path] {
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
	for _, kind := range plan.selected {
		previous, exists := plan.sidecar.Agents[kind]
		if exists {
			for _, recorded := range previous.Files {
				current, _, _, err := s.readKeyedRevision(recorded.Path, revisionContextAgentFile)
				if err != nil || revisionTokenValue(current) != recorded.RevisionMAC {
					plan.drifted = append(plan.drifted, kind)
					break
				}
			}
			continue
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
				for _, key := range claudeFixedEnvKeys {
					if _, ok := env[key]; ok {
						collision = true
						break
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
				plan.collisions = append(plan.collisions, ManagedCollision{Agent: kind, Path: managedNamespace(kind, file.role), Type: "fixed_managed_path", Action: "replace"})
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
