package agent

import (
	"context"
	"encoding/json"
	"sort"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

// ModelsExisting is the strictly typed, key-free current-file prefill. Until
// last-applied sidecar ownership is introduced, every returned current-file
// section is explicitly reported as drifted.
type ModelsExisting struct {
	ModelConfig       json.RawMessage
	UnavailableModels map[string][]string
	DriftedAgents     []string
}

// ModelsResult contains a signed catalog and safe existing model selections.
type ModelsResult struct {
	CatalogToken string
	Existing     ModelsExisting
}

// DiscoverModels signs one normalized catalog and inspects only supported
// typed model fields from selected Agent files. It never represents auth,
// headers, unrelated settings, or raw file content in its result.
func (s *Service) DiscoverModels(ctx context.Context, selected []Kind, catalog []string, claims modelconfig.CatalogClaims) (ModelsResult, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return ModelsResult{}, err
	}
	defer lock.release()
	if err := s.ensureSigner(); err != nil {
		return ModelsResult{}, err
	}
	token, err := s.signer.SignCatalog(claims)
	if err != nil {
		return ModelsResult{}, operationError(CodeModelStateInvalid, "Agent model trust state is invalid")
	}

	existing := ModelsExisting{ModelConfig: json.RawMessage(`{}`), UnavailableModels: map[string][]string{}, DriftedAgents: []string{}}
	states, err := s.detector.Detect()
	if err != nil {
		return ModelsResult{}, operationError(CodeConfigInvalid, "Agent configuration inspection failed")
	}
	byKind := make(map[Kind]State, len(states))
	for _, item := range states {
		byKind[item.Agent] = item
	}
	available := make(map[string]bool, len(catalog))
	for _, id := range catalog {
		available[id] = true
	}
	config := &modelconfig.Config{Version: modelconfig.Version}
	lastApplied, _, _, _, stateErr := s.readSidecar()
	if stateErr != nil {
		return ModelsResult{}, stateErr
	}
	for _, kind := range selected {
		state := byKind[kind]
		if !state.Exists || state.Invalid {
			continue
		}
		if applied, ok := lastApplied.Agents[kind]; ok && s.appliedRevisionsMatch(applied) {
			section, ids, ok := decodeAppliedSection(kind, applied.ModelConfig, catalog)
			if ok {
				missing := unavailable(ids, available)
				if len(missing) > 0 {
					existing.UnavailableModels[string(kind)] = missing
				} else {
					setConfigSection(config, kind, section)
				}
				continue
			}
		}
		section, ids, ok := typedCurrentSection(kind, state)
		if !ok {
			continue
		}
		missing := unavailable(ids, available)
		if len(missing) > 0 {
			existing.UnavailableModels[string(kind)] = missing
		} else {
			setConfigSection(config, kind, section)
		}
		existing.DriftedAgents = append(existing.DriftedAgents, string(kind))
	}
	if config.Claude != nil || config.OpenCode != nil || config.Codex != nil {
		canonical, canonicalErr := modelconfig.Canonical(config)
		if canonicalErr != nil {
			return ModelsResult{}, operationError(CodeConfigInvalid, "Agent model prefill is invalid")
		}
		existing.ModelConfig = canonical
	}
	return ModelsResult{CatalogToken: token, Existing: existing}, nil
}

func (s *Service) appliedRevisionsMatch(section lastAppliedAgent) bool {
	for _, file := range section.Files {
		revision, _, _, err := s.readKeyedRevision(file.Path, revisionContextAgentFile)
		if err != nil || revisionTokenValue(revision) != file.RevisionMAC {
			return false
		}
	}
	return true
}

func decodeAppliedSection(kind Kind, section json.RawMessage, catalog []string) (any, []string, bool) {
	document := map[string]any{"version": modelconfig.Version}
	var value any
	if json.Unmarshal(section, &value) != nil {
		return nil, nil, false
	}
	document[string(kind)] = value
	raw, err := json.Marshal(document)
	if err != nil {
		return nil, nil, false
	}
	decoded, err := modelconfig.Decode(raw, []modelconfig.Agent{modelAgent(kind)}, catalog)
	if err != nil {
		// Decode once against the IDs recorded in the section so unavailable
		// values can still be reported without accepting malformed state.
		ids := sectionModelIDs(kind, section)
		decoded, err = modelconfig.Decode(raw, []modelconfig.Agent{modelAgent(kind)}, ids)
		if err != nil {
			return nil, nil, false
		}
	}
	switch kind {
	case ClaudeCode:
		return decoded.Claude, sectionModelIDs(kind, section), true
	case OpenCode:
		return decoded.OpenCode, sectionModelIDs(kind, section), true
	default:
		return decoded.Codex, sectionModelIDs(kind, section), true
	}
}

func sectionModelIDs(kind Kind, raw json.RawMessage) []string {
	var ids []string
	switch kind {
	case ClaudeCode:
		var section modelconfig.ClaudeConfig
		if json.Unmarshal(raw, &section) == nil {
			ids = append(ids, section.Primary.Model)
			for _, role := range []modelconfig.ClaudeRole{section.Haiku, section.Sonnet, section.Opus} {
				if role.Selection != nil {
					ids = append(ids, role.Selection.Model)
				}
			}
		}
	case OpenCode:
		var section modelconfig.OpenCodeConfig
		if json.Unmarshal(raw, &section) == nil {
			for id := range section.Models {
				ids = append(ids, id)
			}
		}
	case Codex:
		var section modelconfig.CodexConfig
		if json.Unmarshal(raw, &section) == nil {
			ids = append(ids, section.Model)
		}
	}
	sort.Strings(ids)
	return ids
}

func setConfigSection(config *modelconfig.Config, kind Kind, section any) {
	switch kind {
	case ClaudeCode:
		config.Claude = section.(*modelconfig.ClaudeConfig)
	case OpenCode:
		config.OpenCode = section.(*modelconfig.OpenCodeConfig)
	case Codex:
		config.Codex = section.(*modelconfig.CodexConfig)
	}
}

func typedCurrentSection(kind Kind, state State) (any, []string, bool) {
	switch kind {
	case ClaudeCode:
		return currentClaude(state.Path)
	case OpenCode:
		return currentOpenCode(state.Path, state.Format)
	case Codex:
		return currentCodex(state.Path)
	default:
		return nil, nil, false
	}
}

func currentClaude(path string) (any, []string, bool) {
	content, err := readConfig(path)
	if err != nil {
		return nil, nil, false
	}
	root, ok := decodeObject(content)
	if !ok {
		return nil, nil, false
	}
	env, ok := decodeObject(root["env"])
	if !ok {
		return nil, nil, false
	}
	primary, ok := rawString(env["ANTHROPIC_MODEL"])
	if !ok {
		return nil, nil, false
	}
	config := &modelconfig.ClaudeConfig{Primary: modelconfig.Model{Model: primary}}
	if name, present := rawString(env["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"]); present {
		config.Primary.Name = &name
	}
	ids := []string{primary}
	for key, target := range map[string]*modelconfig.ClaudeRole{
		"HAIKU": &config.Haiku, "SONNET": &config.Sonnet, "OPUS": &config.Opus,
	} {
		id, present := rawString(env["ANTHROPIC_DEFAULT_"+key+"_MODEL"])
		if !present {
			return nil, nil, false
		}
		ids = append(ids, id)
		if id == primary {
			*target = modelconfig.ClaudeRole{InheritPrimary: true}
		} else {
			selection := &modelconfig.Model{Model: id}
			if name, namePresent := rawString(env["ANTHROPIC_DEFAULT_"+key+"_MODEL_NAME"]); namePresent {
				selection.Name = &name
			}
			*target = modelconfig.ClaudeRole{Selection: selection}
		}
	}
	return config, ids, true
}

func currentOpenCode(path string, format Format) (any, []string, bool) {
	content, err := readConfig(path)
	if err != nil {
		return nil, nil, false
	}
	if format == FormatJSONC {
		content, err = stripJSONC(content)
		if err != nil {
			return nil, nil, false
		}
	}
	root, ok := decodeObject(content)
	if !ok {
		return nil, nil, false
	}
	defaultRef, ok := rawString(root["model"])
	if !ok || !strings.HasPrefix(defaultRef, "mtls-router/") {
		return nil, nil, false
	}
	providers, ok := decodeObject(root["provider"])
	if !ok {
		return nil, nil, false
	}
	provider, ok := decodeObject(providers["mtls-router"])
	if !ok {
		return nil, nil, false
	}
	models, ok := decodeObject(provider["models"])
	if !ok || len(models) == 0 {
		return nil, nil, false
	}
	defaultModel := strings.TrimPrefix(defaultRef, "mtls-router/")
	projectedModels := make(map[string]json.RawMessage, len(models))
	ids := make([]string, 0, len(models))
	for id, raw := range models {
		object, valid := decodeObject(raw)
		if !valid {
			return nil, nil, false
		}
		projected := map[string]json.RawMessage{}
		for _, key := range []string{"name", "reasoning", "attachment", "tool_call", "temperature", "limit", "modalities", "interleaved"} {
			if value, present := object[key]; present {
				projected[key] = value
			}
		}
		encoded, encodeErr := json.Marshal(projected)
		if encodeErr != nil {
			return nil, nil, false
		}
		projectedModels[id] = encoded
		ids = append(ids, id)
	}
	if _, present := projectedModels[defaultModel]; !present {
		return nil, nil, false
	}
	document, encodeErr := json.Marshal(map[string]any{
		"version":  modelconfig.Version,
		"opencode": map[string]any{"default_model": defaultModel, "models": projectedModels},
	})
	if encodeErr != nil {
		return nil, nil, false
	}
	decoded, decodeErr := modelconfig.Decode(document, []modelconfig.Agent{modelconfig.OpenCode}, ids)
	if decodeErr != nil {
		return nil, nil, false
	}
	return decoded.OpenCode, ids, true
}

func currentCodex(path string) (any, []string, bool) {
	content, err := readConfig(path)
	if err != nil {
		return nil, nil, false
	}
	values, ok := decodeTOML(content)
	if !ok || values["model_provider"] != "mtls-router" {
		return nil, nil, false
	}
	id, ok := values["model"].(string)
	if !ok || id == "" {
		return nil, nil, false
	}
	section := map[string]any{"model": id}
	for tomlKey, configKey := range map[string]string{
		"model_reasoning_effort": "reasoning_effort", "model_reasoning_summary": "reasoning_summary", "model_verbosity": "verbosity",
	} {
		if raw, present := values[tomlKey]; present {
			value, valid := raw.(string)
			if !valid {
				return nil, nil, false
			}
			section[configKey] = value
		}
	}
	for tomlKey, configKey := range map[string]string{
		"model_context_window": "context_window", "model_auto_compact_token_limit": "auto_compact_token_limit",
	} {
		if raw, present := values[tomlKey]; present {
			value, valid := raw.(int64)
			if !valid {
				return nil, nil, false
			}
			section[configKey] = value
		}
	}
	if raw, present := values["model_auto_compact_token_limit_scope"]; present {
		value, valid := raw.(string)
		if !valid {
			return nil, nil, false
		}
		section["extra"] = map[string]any{"model_auto_compact_token_limit_scope": value}
	}
	document, encodeErr := json.Marshal(map[string]any{"version": modelconfig.Version, "codex": section})
	if encodeErr != nil {
		return nil, nil, false
	}
	decoded, decodeErr := modelconfig.Decode(document, []modelconfig.Agent{modelconfig.Codex}, []string{id})
	if decodeErr != nil {
		return nil, nil, false
	}
	return decoded.Codex, []string{id}, true
}

func rawString(raw json.RawMessage) (string, bool) {
	var value string
	if json.Unmarshal(raw, &value) != nil || value == "" {
		return "", false
	}
	return value, true
}

func unavailable(ids []string, available map[string]bool) []string {
	missing := map[string]bool{}
	for _, id := range ids {
		if !available[id] {
			missing[id] = true
		}
	}
	result := make([]string, 0, len(missing))
	for id := range missing {
		result = append(result, id)
	}
	sort.Strings(result)
	return result
}
