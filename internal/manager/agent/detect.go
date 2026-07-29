package agent

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const maxConfigSize = 16 << 20

// Format identifies an agent configuration format.
type Format string

const (
	FormatJSON  Format = "json"
	FormatJSONC Format = "jsonc"
	FormatTOML  Format = "toml"
)

// State is the complete read-only detection result for one agent. It contains
// metadata only; configuration values are deliberately not represented.
type State struct {
	Agent      Kind          `json:"agent"`
	Name       string        `json:"name"`
	Detected   bool          `json:"detected"`
	Command    string        `json:"command"`
	Path       string        `json:"path"`
	AuthPath   string        `json:"auth_path,omitempty"`
	Format     Format        `json:"format"`
	Exists     bool          `json:"exists"`
	Writable   bool          `json:"writable"`
	Configured bool          `json:"configured"`
	Invalid    bool          `json:"invalid"`
	Migratable bool          `json:"migratable,omitempty"`
	Recovery   RecoveryState `json:"recovery"`

	pathOverridden bool
}

// RecoveryReason is a stable, content-free explanation of recovery state.
type RecoveryReason string

const (
	RecoverySyntaxInvalid        RecoveryReason = "syntax_invalid"
	RecoveryUnsupportedStructure RecoveryReason = "unsupported_structure"
	RecoveryUnreadable           RecoveryReason = "unreadable"
	RecoveryOversized            RecoveryReason = "oversized"
	RecoveryNonRegular           RecoveryReason = "non_regular"
	RecoveryLinked               RecoveryReason = "linked"
	RecoveryNotWritable          RecoveryReason = "not_writable"
	RecoveryParentUnavailable    RecoveryReason = "parent_unavailable"
	RecoveryTransactionPending   RecoveryReason = "transaction_recovery_pending"
	RecoveryWritesDisabled       RecoveryReason = "writes_disabled"
)

// RecoveryFileState describes one complete managed target without exposing its
// contents, parser diagnostics, or credential-derived values. Reasons contains
// only file-local blockers; manager-wide blockers are reported by RecoveryState.
type RecoveryFileState struct {
	Role    string           `json:"role"`
	Path    string           `json:"path"`
	Format  Format           `json:"format"`
	Exists  bool             `json:"exists"`
	Reasons []RecoveryReason `json:"reasons,omitempty"`
}

// RecoveryState is advisory. Preview and write must repeat all checks while
// holding the Agent transaction lock.
type RecoveryState struct {
	Eligible bool                `json:"eligible"`
	Reasons  []RecoveryReason    `json:"reasons,omitempty"`
	Files    []RecoveryFileState `json:"files"`
}

// Detector permits deterministic environment lookup in tests.
// Zero values use the current process environment.
type Detector struct {
	HomeDir string
	Getenv  func(string) string
}

// Detect inspects all supported agents and returns one state for each agent.
func Detect() ([]State, error) {
	return Detector{}.Detect()
}

// Detect inspects all supported agents without modifying their files.
func (d Detector) Detect() ([]State, error) {
	home := d.HomeDir
	if home == "" {
		var err error
		home, err = os.UserHomeDir()
		if err != nil {
			return nil, errors.New("resolve user home directory")
		}
	}
	getenv := d.Getenv
	if getenv == nil {
		getenv = os.Getenv
	}
	claudePaths := ClaudePaths(home, getenv("CLAUDE_CONFIG_DIR"))
	openCodeOverride := getenv("OPENCODE_CONFIG")
	openCodePaths := OpenCodePaths(home, openCodeOverride)
	codexPaths := CodexPaths(home, getenv("CODEX_HOME"))

	claude := inspectJSONState(ClaudeCode, "Claude Code", claudePaths, FormatJSON, inspectClaude)
	openCodeFormat := FormatJSON
	if filepath.Ext(openCodePaths.ConfigPath) == ".jsonc" {
		openCodeFormat = FormatJSONC
	}
	openCode := inspectJSONState(OpenCode, "opencode", openCodePaths, openCodeFormat, inspectOpenCode)
	openCode.pathOverridden = openCodeOverride != ""
	codex := inspectCodex(codexPaths)

	return []State{claude, openCode, codex}, nil
}

type jsonInspector func(map[string]json.RawMessage) (configured, invalid bool)

func inspectJSONState(kind Kind, name string, paths Paths, format Format, inspect jsonInspector) State {
	state := baseState(kind, name, paths, format)
	file := &state.Recovery.Files[0]
	if len(file.Reasons) != 0 {
		state.Invalid = hasLegacyInvalidFileReason(file.Reasons)
		finalizeRecovery(&state)
		return state
	}
	if !state.Exists {
		finalizeRecovery(&state)
		return state
	}
	content, err := readRecoveryConfig(paths.ConfigPath)
	if err != nil {
		state.Invalid = true
		file.Reasons = appendReason(file.Reasons, recoveryReadReason(err))
		finalizeRecovery(&state)
		return state
	}
	if format == FormatJSONC {
		content, err = stripJSONC(content)
		if err != nil {
			state.Invalid = true
			file.Reasons = appendReason(file.Reasons, RecoverySyntaxInvalid)
			finalizeRecovery(&state)
			return state
		}
	}
	root, reason := decodeRecoveryObject(content)
	if reason != "" {
		state.Invalid = true
		file.Reasons = appendReason(file.Reasons, reason)
		finalizeRecovery(&state)
		return state
	}
	state.Configured, state.Invalid = inspect(root)
	if state.Invalid {
		file.Reasons = appendReason(file.Reasons, RecoveryUnsupportedStructure)
	} else if kind == ClaudeCode {
		if env, exists := root["env"]; exists {
			if _, valid := decodeObject(env); !valid {
				file.Reasons = appendReason(file.Reasons, RecoveryUnsupportedStructure)
			}
		}
	}
	finalizeRecovery(&state)
	return state
}

func baseState(kind Kind, name string, paths Paths, format Format) State {
	configFile := inspectRecoveryTarget("config", paths.ConfigPath, format)
	exists := configFile.Exists
	writable := pathWritable(paths.ConfigPath)
	files := []RecoveryFileState{configFile}
	if paths.AuthPath != "" {
		writable = writable && pathWritable(paths.AuthPath)
		files = append(files, inspectRecoveryTarget("auth", paths.AuthPath, FormatJSON))
	}
	return State{
		Agent: kind, Name: name, Detected: true, Command: "",
		Path: paths.ConfigPath, AuthPath: paths.AuthPath, Format: format,
		Exists: exists, Writable: writable,
		Recovery: RecoveryState{Files: files},
	}
}

func inspectClaude(root map[string]json.RawMessage) (bool, bool) {
	envRaw, ok := root["env"]
	if !ok {
		return false, false
	}
	env, valid := decodeObject(envRaw)
	if !valid {
		return false, false
	}
	base, ok := rawString(env["ANTHROPIC_BASE_URL"])
	if !ok || !validRouterBaseURL(base) || !hasNonemptyJSONString(env["ANTHROPIC_AUTH_TOKEN"]) {
		return false, false
	}
	for _, key := range []string{"ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL"} {
		if !hasNonemptyJSONString(env[key]) {
			return false, false
		}
	}
	return true, false
}

func inspectOpenCode(root map[string]json.RawMessage) (bool, bool) {
	providerRaw, ok := root["provider"]
	if !ok || bytes.Equal(bytes.TrimSpace(providerRaw), []byte("null")) {
		return false, false
	}
	providers, valid := decodeObject(providerRaw)
	if !valid {
		return false, true
	}
	routerRaw, ok := providers["mtls-router"]
	if !ok {
		return false, false
	}
	router, valid := decodeObject(routerRaw)
	if !valid {
		return false, false
	}
	options, valid := decodeObject(router["options"])
	if !valid {
		return false, false
	}
	base, ok := rawString(options["baseURL"])
	if !ok || !validAPIBaseURL(base) || !hasNonemptyJSONString(options["apiKey"]) {
		return false, false
	}
	name, nameOK := rawString(router["name"])
	if !jsonStringEquals(router["npm"], "@ai-sdk/openai-compatible") || !nameOK || !isManagedProviderDisplayName(name) {
		return false, false
	}
	models, valid := decodeObject(router["models"])
	if !valid || len(models) == 0 {
		return false, false
	}
	rootModel, ok := rawString(root["model"])
	if !ok || !strings.HasPrefix(rootModel, "mtls-router/") {
		return false, false
	}
	_, exists := models[strings.TrimPrefix(rootModel, "mtls-router/")]
	return exists, false
}

func inspectCodex(paths Paths) State {
	state := baseState(Codex, "Codex", paths, FormatTOML)
	configFile := &state.Recovery.Files[0]
	authFile := &state.Recovery.Files[1]

	authConfigured := false
	var authRoot map[string]json.RawMessage
	if authFile.Exists && !hasBlockingFileReason(authFile.Reasons) {
		authContent, err := readRecoveryConfig(paths.AuthPath)
		if err != nil {
			state.Invalid = true
			authFile.Reasons = appendReason(authFile.Reasons, recoveryReadReason(err))
		} else {
			var reason RecoveryReason
			authRoot, reason = decodeRecoveryObject(authContent)
			if reason != "" {
				state.Invalid = true
				authFile.Reasons = appendReason(authFile.Reasons, reason)
			} else {
				authConfigured = jsonStringEquals(authRoot["auth_mode"], "apikey") && hasNonemptyJSONString(authRoot["OPENAI_API_KEY"])
			}
		}
	}
	if !state.Exists {
		state.Invalid = state.Invalid || hasLegacyInvalidFileReason(configFile.Reasons) || hasLegacyInvalidFileReason(authFile.Reasons)
		finalizeRecovery(&state)
		return state
	}
	if hasBlockingFileReason(configFile.Reasons) {
		state.Invalid = hasLegacyInvalidFileReason(configFile.Reasons)
		finalizeRecovery(&state)
		return state
	}

	content, err := readRecoveryConfig(paths.ConfigPath)
	if err != nil {
		state.Invalid = true
		configFile.Reasons = appendReason(configFile.Reasons, recoveryReadReason(err))
		finalizeRecovery(&state)
		return state
	}
	values, valid := decodeTOML(content)
	if !valid {
		state.Invalid = true
		configFile.Reasons = appendReason(configFile.Reasons, RecoverySyntaxInvalid)
		finalizeRecovery(&state)
		return state
	}

	if authRoot == nil {
		authRoot = map[string]json.RawMessage{}
	}
	state.Migratable = exactHistoricalCodex(values, authRoot)
	provider, providerOK := tomlTable(values, "model_providers", "mtls-router")
	model, modelOK := tomlString(values["model"])
	store, storeOK := tomlString(values["cli_auth_credentials_store"])
	providerName, _ := provider["name"].(string)
	state.Configured = values["model_provider"] == "mtls-router" && modelOK && model != "" && storeOK && store == "file" &&
		providerOK && isManagedProviderDisplayName(providerName) && provider["wire_api"] == "responses" && provider["requires_openai_auth"] == true &&
		validAPIValue(provider["base_url"]) && authConfigured && validCodexModelSettings(values)
	state.Invalid = state.Invalid || hasLegacyInvalidFileReason(configFile.Reasons) || hasLegacyInvalidFileReason(authFile.Reasons)
	finalizeRecovery(&state)
	return state
}

func decodeRecoveryObject(content []byte) (map[string]json.RawMessage, RecoveryReason) {
	content = bytes.TrimPrefix(content, []byte{0xef, 0xbb, 0xbf})
	var value any
	decoder := json.NewDecoder(bytes.NewReader(content))
	if err := decoder.Decode(&value); err != nil {
		return nil, RecoverySyntaxInvalid
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return nil, RecoverySyntaxInvalid
	}
	if _, ok := value.(map[string]any); !ok {
		return nil, RecoveryUnsupportedStructure
	}
	root, valid := decodeObject(content)
	if !valid {
		return nil, RecoveryUnsupportedStructure
	}
	return root, ""
}

func validRouterBaseURL(value string) bool { _, err := apiURL(value); return err == nil }
func validAPIBaseURL(value string) bool    { return validAPIValue(value) }
func validAPIValue(value any) bool {
	text, ok := value.(string)
	if !ok || !strings.HasSuffix(text, "/v1") {
		return false
	}
	api, err := apiURL(strings.TrimSuffix(text, "/v1"))
	return err == nil && api == strings.TrimSuffix(text, "/")
}

func validCodexModelSettings(values map[string]any) bool {
	section := map[string]any{"model": values["model"]}
	for key, canonical := range map[string]string{"model_reasoning_effort": "reasoning_effort", "model_reasoning_summary": "reasoning_summary", "model_verbosity": "verbosity", "model_context_window": "context_window", "model_auto_compact_token_limit": "auto_compact_token_limit"} {
		if value, ok := values[key]; ok {
			section[canonical] = value
		}
	}
	if value, ok := values["model_auto_compact_token_limit_scope"]; ok {
		section["extra"] = map[string]any{"model_auto_compact_token_limit_scope": value}
	}
	doc, err := json.Marshal(map[string]any{"version": modelconfig.Version, "codex": section})
	if err != nil {
		return false
	}
	model, _ := values["model"].(string)
	_, err = modelconfig.Decode(doc, []modelconfig.Agent{modelconfig.Codex}, []string{model})
	return err == nil
}

func decodeObject(content []byte) (map[string]json.RawMessage, bool) {
	var object map[string]json.RawMessage
	content = bytes.TrimPrefix(content, []byte{0xef, 0xbb, 0xbf})
	decoder := json.NewDecoder(bytes.NewReader(content))
	if err := decoder.Decode(&object); err != nil || object == nil {
		return nil, false
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return nil, false
	}
	return object, true
}

func jsonStringEquals(raw json.RawMessage, want string) bool {
	var value string
	return json.Unmarshal(raw, &value) == nil && value == want
}

func hasNonemptyJSONString(raw json.RawMessage) bool {
	var value string
	return json.Unmarshal(raw, &value) == nil && value != ""
}

func readConfig(path string) ([]byte, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	content, err := io.ReadAll(io.LimitReader(file, maxConfigSize+1))
	if err != nil {
		return nil, err
	}
	if len(content) > maxConfigSize {
		return nil, errConfigOversized
	}
	return content, nil
}

var errConfigOversized = errors.New("configuration file is too large")

func readRecoveryConfig(path string) ([]byte, error) {
	return readConfig(path)
}

func recoveryReadReason(err error) RecoveryReason {
	if errors.Is(err, errConfigOversized) {
		return RecoveryOversized
	}
	return RecoveryUnreadable
}

func isRegularFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}

func pathWritable(path string) bool {
	info, err := os.Stat(path)
	if err == nil {
		if !info.Mode().IsRegular() {
			return false
		}
		file, openErr := os.OpenFile(path, os.O_WRONLY, 0)
		if openErr != nil {
			return false
		}
		return file.Close() == nil
	}
	if !os.IsNotExist(err) {
		return false
	}

	dir := filepath.Dir(path)
	for {
		info, err = os.Stat(dir)
		if err == nil {
			return info.IsDir() && info.Mode().Perm()&0222 != 0
		}
		if !os.IsNotExist(err) {
			return false
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return false
		}
		dir = parent
	}
}

// stripJSONC mirrors the setup script's string-aware removal of comments and
// trailing commas.
func stripJSONC(src []byte) ([]byte, error) {
	var out bytes.Buffer
	inString, escape, lineComment, blockComment := false, false, false, false
	for i := 0; i < len(src); {
		ch := src[i]
		var next byte
		if i+1 < len(src) {
			next = src[i+1]
		}
		switch {
		case lineComment:
			if ch == '\n' {
				lineComment = false
				out.WriteByte(ch)
			}
			i++
		case blockComment:
			if ch == '*' && next == '/' {
				blockComment = false
				i += 2
			} else {
				i++
			}
		case inString:
			out.WriteByte(ch)
			if escape {
				escape = false
			} else if ch == '\\' {
				escape = true
			} else if ch == '"' {
				inString = false
			}
			i++
		case ch == '"':
			inString = true
			out.WriteByte(ch)
			i++
		case ch == '/' && next == '/':
			lineComment = true
			i += 2
		case ch == '/' && next == '*':
			blockComment = true
			i += 2
		case ch == ',':
			j := i + 1
			for j < len(src) && strings.ContainsRune(" \t\r\n", rune(src[j])) {
				j++
			}
			if j < len(src) && (src[j] == ']' || src[j] == '}') {
				i++
				continue
			}
			out.WriteByte(ch)
			i++
		default:
			out.WriteByte(ch)
			i++
		}
	}
	if inString || blockComment {
		return nil, errors.New("unterminated JSONC token")
	}
	return out.Bytes(), nil
}
