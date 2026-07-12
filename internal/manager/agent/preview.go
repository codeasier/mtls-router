package agent

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
)

const (
	jsoncMigrationWarning = "opencode.jsonc will be migrated to opencode.json; comments and formatting will not be preserved"
	backupWarning         = "Backups are sensitive recovery artifacts and may contain a previous API key"
)

type writePlan struct {
	selected []Kind
	files    []plannedFile
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
}

func (s *Service) buildPlan(selected []Kind) (writePlan, error) {
	normalized, err := normalizeSelection(selected)
	if err != nil {
		return writePlan{}, err
	}
	states, err := s.detector.Detect()
	if err != nil {
		return writePlan{}, operationError(CodeAgentNotFound, "could not detect supported Agents")
	}
	byKind := orderedStates(states)
	plan := writePlan{selected: normalized}
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
			files, err = planClaude(state)
		case OpenCode:
			files, err = planOpenCode(state)
		case Codex:
			files, err = planCodex(state)
		}
		if err != nil {
			return writePlan{}, err
		}
		plan.files = append(plan.files, files...)
	}
	return plan, nil
}

func planClaude(state State) ([]plannedFile, error) {
	if err := ensureWritable(state.Path); err != nil {
		return nil, err
	}
	revision, content, mode, err := readRevision(state.Path)
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
	file.render = func(key []byte) ([]byte, error) { return renderClaude(root, key) }
	return []plannedFile{file}, nil
}

func planOpenCode(state State) ([]plannedFile, error) {
	sourcePath := state.Path
	targetPath := state.Path
	format := state.Format
	if filepath.Ext(sourcePath) == ".jsonc" {
		targetPath = filepath.Join(filepath.Dir(sourcePath), "opencode.json")
		format = FormatJSON
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
	if err := ensureWritable(sourcePath); err != nil {
		return nil, err
	}
	if err := ensureWritable(targetPath); err != nil {
		return nil, err
	}
	sourceRevision, sourceContent, sourceMode, err := readRevision(sourcePath)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "opencode configuration cannot be read")
	}
	parsed := sourceContent
	if filepath.Ext(sourcePath) == ".jsonc" && sourceRevision.Exists {
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
	targetRevision, _, targetMode, err := readRevision(targetPath)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "opencode target cannot be read")
	}
	operation := OperationCreate
	if targetRevision.Exists {
		operation = OperationReplace
	}
	warning := ""
	if filepath.Ext(sourcePath) == ".jsonc" && sourcePath != targetPath {
		warning = jsoncMigrationWarning
	}
	file := plannedFile{
		agent: OpenCode, format: format, sourcePath: sourcePath, targetPath: targetPath,
		operation: operation, containsAPIKey: true,
		preserves: []string{"all root fields other than provider.mtls-router", "all other providers"}, warning: warning,
		sourceRevision: sourceRevision, targetRevision: targetRevision, sourceContent: sourceContent,
		sourceMode: sourceMode, targetMode: targetMode, backupRequired: sourceRevision.Exists,
		backupSource: sourcePath, restoreFrom: sourcePath,
	}
	file.render = func(key []byte) ([]byte, error) { return renderOpenCode(root, key) }
	return []plannedFile{file}, nil
}

func planCodex(state State) ([]plannedFile, error) {
	if err := ensureWritable(state.Path); err != nil {
		return nil, err
	}
	if err := ensureWritable(state.AuthPath); err != nil {
		return nil, err
	}
	configRevision, configContent, configMode, err := readRevision(state.Path)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "Codex configuration cannot be read")
	}
	if configRevision.Exists {
		if _, valid := parseTOML(configContent); !valid {
			return nil, operationError(CodeConfigInvalid, "Codex configuration is invalid TOML")
		}
	}
	authRevision, authContent, authMode, err := readRevision(state.AuthPath)
	if err != nil {
		return nil, operationError(CodeConfigInvalid, "Codex auth configuration cannot be read")
	}
	if authRevision.Exists {
		if _, valid := decodeObject(authContent); !valid {
			return nil, operationError(CodeConfigInvalid, "Codex auth configuration is invalid JSON")
		}
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
	config.render = func([]byte) ([]byte, error) { return renderCodex(configContent), nil }
	auth := plannedFile{
		agent: Codex, format: FormatJSON, sourcePath: state.AuthPath, targetPath: state.AuthPath,
		operation: authOperation, containsAPIKey: true,
		warning:        "Codex auth.json is replaced with only OPENAI_API_KEY; existing auth fields are not preserved",
		sourceRevision: authRevision, targetRevision: authRevision, sourceContent: authContent,
		sourceMode: authMode, targetMode: authMode, backupRequired: authRevision.Exists,
		backupSource: state.AuthPath, restoreFrom: state.AuthPath,
	}
	auth.render = renderCodexAuth
	return []plannedFile{config, auth}, nil
}

func renderClaude(root map[string]json.RawMessage, key []byte) ([]byte, error) {
	result := cloneRawObject(root)
	env := map[string]string{
		"ANTHROPIC_BASE_URL":                  "http://127.0.0.1:19099",
		"ANTHROPIC_AUTH_TOKEN":                string(key),
		"ANTHROPIC_DEFAULT_HAIKU_MODEL":       "gpt-5.5",
		"ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":  "gpt-5.5",
		"ANTHROPIC_DEFAULT_OPUS_MODEL":        "gpt-5.5",
		"ANTHROPIC_DEFAULT_OPUS_MODEL_NAME":   "gpt-5.5",
		"ANTHROPIC_DEFAULT_SONNET_MODEL":      "gpt-5.4[1M]",
		"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "gpt-5.4",
		"ANTHROPIC_MODEL":                     "gpt-5.5",
		"ENABLE_TOOL_SEARCH":                  "true",
		"DISABLE_AUTOUPDATER":                 "1",
	}
	envJSON, err := json.Marshal(env)
	if err != nil {
		return nil, err
	}
	result["env"] = envJSON
	return marshalObject(result)
}

func renderOpenCode(root map[string]json.RawMessage, key []byte) ([]byte, error) {
	result := cloneRawObject(root)
	providers := make(map[string]json.RawMessage)
	if raw, ok := result["provider"]; ok && !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		var valid bool
		providers, valid = decodeObject(raw)
		if !valid {
			return nil, errors.New("provider is not an object")
		}
	}
	providerJSON, err := json.Marshal(openCodeProvider(string(key)))
	if err != nil {
		return nil, err
	}
	providers["mtls-router"] = providerJSON
	providersJSON, err := json.Marshal(providers)
	if err != nil {
		return nil, err
	}
	result["provider"] = providersJSON
	return marshalObject(result)
}

func openCodeProvider(key string) map[string]any {
	return map[string]any{
		"npm":  "@ai-sdk/openai-compatible",
		"name": "mtls-router",
		"options": map[string]any{
			"baseURL": "http://127.0.0.1:19099/v1",
			"apiKey":  key,
		},
		"models": map[string]any{
			"gpt-5.5": map[string]any{
				"name": "GPT-5.5", "reasoning": true, "attachment": true, "tool_call": true,
				"limit":      map[string]int{"context": 272000, "input": 244800, "output": 27200},
				"modalities": map[string][]string{"input": {"text", "image"}, "output": {"text"}},
				"options":    map[string]string{"reasoningEffort": "medium"},
			},
			"gpt-5.4": map[string]any{
				"name": "GPT-5.4", "reasoning": true, "attachment": true, "tool_call": true,
				"limit":      map[string]int{"context": 1000000, "input": 900000, "output": 100000},
				"modalities": map[string][]string{"input": {"text", "image"}, "output": {"text"}},
				"options":    map[string]string{"reasoningEffort": "medium"},
			},
		},
	}
}

func cloneRawObject(source map[string]json.RawMessage) map[string]json.RawMessage {
	result := make(map[string]json.RawMessage, len(source)+1)
	for key, value := range source {
		result[key] = append(json.RawMessage(nil), value...)
	}
	return result
}

func marshalObject(value map[string]json.RawMessage) ([]byte, error) {
	content, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(content, '\n'), nil
}

func renderCodex(content []byte) []byte {
	lines := strings.SplitAfter(string(content), "\n")
	kept := make([]string, 0, len(lines))
	inRoot := true
	skipCustom := false
	for _, line := range lines {
		trimmed := strings.TrimSpace(strings.TrimSuffix(line, "\n"))
		withoutComment, _ := stripTOMLComment(trimmed)
		withoutComment = strings.TrimSpace(withoutComment)
		if strings.HasPrefix(withoutComment, "[") {
			inRoot = false
			if skipCustom {
				skipCustom = false
			}
			if withoutComment == "[model_providers.custom]" {
				skipCustom = true
				continue
			}
		}
		if skipCustom {
			continue
		}
		if inRoot {
			if key, _, ok := splitTOMLAssignment(withoutComment); ok {
				switch strings.TrimSpace(key) {
				case "model_provider", "model", "disable_response_storage":
					continue
				}
			}
		}
		kept = append(kept, line)
	}
	body := strings.Join(kept, "")
	header := "model_provider = \"custom\"\n" +
		"model = \"gpt-5.5\"\n" +
		"disable_response_storage = true\n\n" +
		"[model_providers.custom]\n" +
		"name = \"9router\"\n" +
		"wire_api = \"responses\"\n" +
		"requires_openai_auth = true\n" +
		"base_url = \"http://127.0.0.1:19099/v1\"\n"
	if body != "" {
		return []byte(header + "\n" + body)
	}
	return []byte(header)
}

func renderCodexAuth(key []byte) ([]byte, error) {
	content, err := json.MarshalIndent(map[string]string{"OPENAI_API_KEY": string(key)}, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(content, '\n'), nil
}
