package agent

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
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
	Agent      Kind   `json:"agent"`
	Name       string `json:"name"`
	Detected   bool   `json:"detected"`
	Command    string `json:"command,omitempty"`
	Path       string `json:"path"`
	AuthPath   string `json:"auth_path,omitempty"`
	Format     Format `json:"format"`
	Exists     bool   `json:"exists"`
	Writable   bool   `json:"writable"`
	Configured bool   `json:"configured"`
	Invalid    bool   `json:"invalid"`
	Migratable bool   `json:"migratable,omitempty"`

	pathOverridden bool
}

// Detector permits deterministic environment and executable lookup in tests.
// Zero values use the current process environment and executable search path.
type Detector struct {
	HomeDir  string
	Getenv   func(string) string
	LookPath func(string) (string, error)
}

// Detect inspects all supported agents. It returns one state for each agent,
// including agents that are not installed, so callers can render stable cards.
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
	lookPath := d.LookPath
	if lookPath == nil {
		lookPath = exec.LookPath
	}

	claudeCommand, _ := lookup(lookPath, "claude")
	openCodeCommand, _ := lookup(lookPath, "opencode")
	codexCommand, _ := lookup(lookPath, "codex")

	claudePaths := ClaudePaths(home, getenv("CLAUDE_CONFIG_DIR"))
	openCodeOverride := getenv("OPENCODE_CONFIG")
	openCodePaths := OpenCodePaths(home, openCodeOverride)
	codexPaths := CodexPaths(home, getenv("CODEX_HOME"))

	claude := inspectJSONState(ClaudeCode, "Claude Code", true, claudeCommand, claudePaths, FormatJSON, inspectClaude)
	openCodeFormat := FormatJSON
	if filepath.Ext(openCodePaths.ConfigPath) == ".jsonc" {
		openCodeFormat = FormatJSONC
	}
	openCode := inspectJSONState(OpenCode, "opencode", true, openCodeCommand, openCodePaths, openCodeFormat, inspectOpenCode)
	openCode.pathOverridden = openCodeOverride != ""
	codex := inspectCodex(true, codexCommand, codexPaths)

	return []State{claude, openCode, codex}, nil
}

func lookup(lookPath func(string) (string, error), name string) (string, bool) {
	path, err := lookPath(name)
	return path, err == nil && path != ""
}

type jsonInspector func(map[string]json.RawMessage) (configured, invalid bool)

func inspectJSONState(kind Kind, name string, detected bool, command string, paths Paths, format Format, inspect jsonInspector) State {
	state := baseState(kind, name, detected, command, paths, format)
	if !state.Exists {
		return state
	}
	content, err := readConfig(paths.ConfigPath)
	if err != nil {
		state.Invalid = true
		return state
	}
	if format == FormatJSONC {
		content, err = stripJSONC(content)
		if err != nil {
			state.Invalid = true
			return state
		}
	}
	root, valid := decodeObject(content)
	if !valid {
		state.Invalid = true
		return state
	}
	state.Configured, state.Invalid = inspect(root)
	return state
}

func baseState(kind Kind, name string, detected bool, command string, paths Paths, format Format) State {
	info, err := os.Stat(paths.ConfigPath)
	exists := err == nil
	invalid := exists && !info.Mode().IsRegular()
	writable := pathWritable(paths.ConfigPath)
	if paths.AuthPath != "" {
		writable = writable && pathWritable(paths.AuthPath)
	}
	return State{
		Agent: kind, Name: name, Detected: detected, Command: command,
		Path: paths.ConfigPath, AuthPath: paths.AuthPath, Format: format,
		Exists: exists, Writable: writable, Invalid: invalid,
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
	if !jsonStringEquals(router["npm"], "@ai-sdk/openai-compatible") || !jsonStringEquals(router["name"], "mtls-router") {
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

func inspectCodex(detected bool, command string, paths Paths) State {
	state := baseState(Codex, "Codex", detected, command, paths, FormatTOML)
	if state.Invalid {
		return state
	}

	authConfigured := false
	if info, err := os.Stat(paths.AuthPath); err == nil {
		if !info.Mode().IsRegular() {
			state.Invalid = true
			return state
		}
		authContent, err := readConfig(paths.AuthPath)
		if err != nil {
			state.Invalid = true
			return state
		}
		auth, valid := decodeObject(authContent)
		if !valid {
			state.Invalid = true
			return state
		}
		authConfigured = jsonStringEquals(auth["auth_mode"], "apikey") && hasNonemptyJSONString(auth["OPENAI_API_KEY"])
	}
	if !state.Exists {
		return state
	}

	content, err := readConfig(paths.ConfigPath)
	if err != nil {
		state.Invalid = true
		return state
	}
	values, valid := decodeTOML(content)
	if !valid {
		state.Invalid = true
		return state
	}

	authRoot := map[string]json.RawMessage{}
	if authContent, err := readConfig(paths.AuthPath); err == nil {
		authRoot, _ = decodeObject(authContent)
	}
	state.Migratable = exactHistoricalCodex(values, authRoot)
	provider, providerOK := tomlTable(values, "model_providers", "mtls-router")
	model, modelOK := tomlString(values["model"])
	store, storeOK := tomlString(values["cli_auth_credentials_store"])
	state.Configured = values["model_provider"] == "mtls-router" && modelOK && model != "" && storeOK && store == "file" &&
		providerOK && provider["name"] == "mtls-router" && provider["wire_api"] == "responses" && provider["requires_openai_auth"] == true &&
		validAPIValue(provider["base_url"]) && authConfigured && validCodexModelSettings(values)
	return state
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
		return nil, errors.New("configuration file is too large")
	}
	return content, nil
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
