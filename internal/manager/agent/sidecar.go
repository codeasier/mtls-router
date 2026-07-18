package agent

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const maxSidecarSize = 2 << 20

type lastAppliedState struct {
	Version       int                       `json:"version"`
	KeyGeneration string                    `json:"key_generation"`
	Agents        map[Kind]lastAppliedAgent `json:"agents"`
}

type lastAppliedAgent struct {
	ModelConfig json.RawMessage   `json:"model_config"`
	Files       []lastAppliedFile `json:"files"`
	OwnedPaths  []string          `json:"owned_paths"`
}

type lastAppliedFile struct {
	Role        string `json:"role"`
	Path        string `json:"path"`
	RevisionMAC string `json:"revision_mac"`
}

func (s *Service) sidecarPath() string { return filepath.Join(s.stateDir, sidecarFileName) }

func (s *Service) readSidecar() (lastAppliedState, fileRevision, []byte, os.FileMode, error) {
	revision, content, mode, err := s.readKeyedRevision(s.sidecarPath(), revisionContextSidecar)
	if err != nil {
		return lastAppliedState{}, fileRevision{}, nil, 0, operationError(CodeModelStateInvalid, "Agent model state cannot be read")
	}
	if !revision.Exists {
		return lastAppliedState{Version: 1, KeyGeneration: s.keyGeneration, Agents: map[Kind]lastAppliedAgent{}}, revision, nil, 0o600, nil
	}
	info, err := os.Stat(s.sidecarPath())
	if err != nil || !privatePermissionsOK(s.sidecarPath(), false, info.Mode()) || len(content) == 0 || len(content) > maxSidecarSize {
		return lastAppliedState{}, fileRevision{}, nil, 0, operationError(CodeModelStateInvalid, "Agent model state is invalid")
	}
	var state lastAppliedState
	if err := strictJSON(content, &state); err != nil || validateSidecarState(state, s.keyGeneration) != nil {
		return lastAppliedState{}, fileRevision{}, nil, 0, operationError(CodeModelStateInvalid, "Agent model state is invalid")
	}
	canonical, err := modelconfig.CanonicalValue(state)
	if err != nil || !bytes.Equal(content, canonical) {
		return lastAppliedState{}, fileRevision{}, nil, 0, operationError(CodeModelStateInvalid, "Agent model state is not canonical")
	}
	return state, revision, content, mode, nil
}

func validateSidecarState(state lastAppliedState, generation string) error {
	if state.Version != 1 || state.KeyGeneration != generation || state.Agents == nil || len(state.Agents) > 3 {
		return errors.New("invalid state header")
	}
	for kind, section := range state.Agents {
		if kind != ClaudeCode && kind != OpenCode && kind != Codex || len(section.ModelConfig) == 0 || len(section.Files) == 0 {
			return errors.New("invalid Agent state")
		}
		seenRoles := map[string]bool{}
		for _, file := range section.Files {
			if (file.Role != "config" && file.Role != "auth") || seenRoles[file.Role] || !filepath.IsAbs(file.Path) || file.RevisionMAC == "" || len(file.RevisionMAC) > 128 {
				return errors.New("invalid state file")
			}
			seenRoles[file.Role] = true
		}
		if kind != Codex && len(section.Files) != 1 || kind == Codex && (len(section.Files) != 2 || !seenRoles["config"] || !seenRoles["auth"]) {
			return errors.New("inconsistent state files")
		}
		if !sort.StringsAreSorted(section.OwnedPaths) {
			return errors.New("noncanonical owned paths")
		}
		for i, path := range section.OwnedPaths {
			if path == "" || len(path) > 4096 || strings.TrimSpace(path) != path || i > 0 && path == section.OwnedPaths[i-1] {
				return errors.New("invalid owned path")
			}
		}
	}
	return nil
}

func strictJSON(content []byte, dst any) error {
	if !utf8.Valid(content) || hasDuplicateJSONKey(content) {
		return errors.New("invalid JSON")
	}
	decoder := json.NewDecoder(bytes.NewReader(content))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(dst); err != nil {
		return err
	}
	var trailing any
	if err := decoder.Decode(&trailing); err != io.EOF {
		return errors.New("trailing JSON")
	}
	return nil
}

func hasDuplicateJSONKey(content []byte) bool {
	decoder := json.NewDecoder(bytes.NewReader(content))
	var walk func(bool) bool
	walk = func(object bool) bool {
		if object {
			seen := map[string]bool{}
			for decoder.More() {
				token, err := decoder.Token()
				key, ok := token.(string)
				if err != nil || !ok || seen[key] {
					return true
				}
				seen[key] = true
				if walk(false) {
					return true
				}
			}
			_, err := decoder.Token()
			return err != nil
		}
		token, err := decoder.Token()
		if err != nil {
			return true
		}
		if delimiter, ok := token.(json.Delim); ok {
			switch delimiter {
			case '{':
				return walk(true)
			case '[':
				for decoder.More() {
					if walk(false) {
						return true
					}
				}
				_, err := decoder.Token()
				return err != nil
			}
		}
		return false
	}
	return walk(false)
}

func sidecarSection(config *modelconfig.Config, kind Kind) (json.RawMessage, []string, error) {
	var section any
	var owned []string
	switch kind {
	case ClaudeCode:
		section = config.Claude
		owned = make([]string, 0, len(claudeFixedEnvKeys)+len(config.Claude.Extra))
		for _, key := range claudeFixedEnvKeys {
			owned = append(owned, "env."+key)
		}
		for key := range config.Claude.Extra {
			owned = append(owned, "env."+key)
		}
	case OpenCode:
		section = config.OpenCode
		owned = []string{"model", "provider.mtls-router"}
	case Codex:
		section = config.Codex
		owned = append([]string{"model_providers.mtls-router", "auth.auth_mode", "auth.OPENAI_API_KEY"}, codexManagedRootKeys...)
	}
	sort.Strings(owned)
	raw, err := modelconfig.CanonicalValue(section)
	return raw, owned, err
}

func (s *Service) appendSidecarPlan(plan writePlan, key []byte) (writePlan, error) {
	state := plan.sidecar
	if state.Agents == nil {
		state.Agents = map[Kind]lastAppliedAgent{}
	}
	for _, kind := range plan.selected {
		section, owned, err := sidecarSection(plan.input.config, kind)
		if err != nil {
			return writePlan{}, operationError(CodeModelStateInvalid, "Agent model state could not be created")
		}
		entry := lastAppliedAgent{ModelConfig: section, OwnedPaths: owned}
		for i := range plan.files {
			file := &plan.files[i]
			if file.agent != kind || file.scope != scopeAgent {
				continue
			}
			output, err := file.render(key)
			if err != nil {
				return writePlan{}, operationError(CodeWriteFailed, "could not render an Agent configuration file")
			}
			mode := file.targetMode
			if !file.targetRevision.Exists {
				mode = 0o600
			}
			revision, err := s.keyedRevisionForContent(output, mode, revisionContextAgentFile, file.targetPath)
			zeroBytes(output)
			if err != nil {
				return writePlan{}, operationError(CodeModelStateInvalid, "Agent model state could not be created")
			}
			entry.Files = append(entry.Files, lastAppliedFile{Role: file.role, Path: file.targetPath, RevisionMAC: revisionTokenValue(revision)})
		}
		state.Agents[kind] = entry
	}
	state.Version = 1
	state.KeyGeneration = s.keyGeneration
	content, err := modelconfig.CanonicalValue(state)
	if err != nil || len(content) > maxSidecarSize {
		return writePlan{}, operationError(CodeModelStateInvalid, "Agent model state could not be created")
	}
	mode := plan.sidecarMode
	if !plan.sidecarRevision.Exists {
		mode = 0o600
	}
	contentCopy := append([]byte(nil), content...)
	operation := OperationCreate
	if plan.sidecarRevision.Exists {
		operation = OperationReplace
	}
	plan.files = append(plan.files, plannedFile{
		scope: scopeManagerState, role: "state", format: FormatJSON,
		sourcePath: s.sidecarPath(), targetPath: s.sidecarPath(), operation: operation,
		sourceRevision: plan.sidecarRevision, targetRevision: plan.sidecarRevision,
		sourceContent: plan.sidecarContent, sourceMode: plan.sidecarMode, targetMode: mode,
		backupRequired: plan.sidecarRevision.Exists, backupSource: s.sidecarPath(), restoreFrom: s.sidecarPath(),
		render: func([]byte) ([]byte, error) { return append([]byte(nil), contentCopy...), nil },
	})
	return plan, nil
}
