package agent

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"unicode/utf8"
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

	claudeCommand, claudeDetected := lookup(lookPath, "claude")
	openCodeCommand, openCodeDetected := lookup(lookPath, "opencode")
	codexCommand, codexCLI := lookup(lookPath, "codex")

	claudePaths := ClaudePaths(home, getenv("CLAUDE_CONFIG_DIR"))
	openCodeOverride := getenv("OPENCODE_CONFIG")
	openCodePaths := OpenCodePaths(home, openCodeOverride)
	codexHome := getenv("CODEX_HOME")
	codexPaths := CodexPaths(home, codexHome)
	if codexHome == "" {
		codexHome = filepath.Dir(codexPaths.ConfigPath)
	}
	codexDetected := codexCLI || isDirectory(codexHome)
	if codexDetected && !codexCLI {
		codexCommand = "<desktop>"
	}

	claude := inspectJSONState(ClaudeCode, "Claude Code", claudeDetected, claudeCommand, claudePaths, FormatJSON, inspectClaude)
	openCodeFormat := FormatJSON
	if filepath.Ext(openCodePaths.ConfigPath) == ".jsonc" {
		openCodeFormat = FormatJSONC
	}
	openCode := inspectJSONState(OpenCode, "opencode", openCodeDetected, openCodeCommand, openCodePaths, openCodeFormat, inspectOpenCode)
	openCode.pathOverridden = openCodeOverride != ""
	codex := inspectCodex(codexDetected, codexCommand, codexPaths)

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
	return jsonStringEquals(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:19099") &&
		hasNonemptyJSONString(env["ANTHROPIC_AUTH_TOKEN"]), false
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
	return jsonStringEquals(options["baseURL"], "http://127.0.0.1:19099/v1") &&
		hasNonemptyJSONString(options["apiKey"]), false
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
		authConfigured = hasNonemptyJSONString(auth["OPENAI_API_KEY"])
	}
	if !state.Exists {
		return state
	}

	content, err := readConfig(paths.ConfigPath)
	if err != nil {
		state.Invalid = true
		return state
	}
	values, valid := parseTOML(content)
	if !valid {
		state.Invalid = true
		return state
	}

	state.Configured = values["model_provider"] == `"custom"` &&
		values["model"] == `"gpt-5.5"` &&
		values["disable_response_storage"] == "true" &&
		values["model_providers.custom.name"] == `"9router"` &&
		values["model_providers.custom.wire_api"] == `"responses"` &&
		values["model_providers.custom.requires_openai_auth"] == "true" &&
		values["model_providers.custom.base_url"] == `"http://127.0.0.1:19099/v1"` &&
		authConfigured
	return state
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

func isDirectory(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
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

// parseTOML validates the TOML forms emitted and preserved by the setup
// scripts and returns only non-sensitive scalar values needed for detection.
func parseTOML(content []byte) (map[string]string, bool) {
	if len(content) > maxConfigSize || !utf8.Valid(content) {
		return nil, false
	}
	values := make(map[string]string)
	section := ""
	scanner := bufio.NewScanner(bytes.NewReader(content))
	scanner.Buffer(make([]byte, 4096), maxConfigSize)
	for scanner.Scan() {
		line, valid := stripTOMLComment(strings.TrimSpace(scanner.Text()))
		if !valid {
			return nil, false
		}
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if strings.HasPrefix(line, "[") {
			name, ok := parseTOMLHeader(line)
			if !ok {
				return nil, false
			}
			section = name
			continue
		}
		key, value, ok := splitTOMLAssignment(line)
		if !ok || !validTOMLKey(key) {
			return nil, false
		}
		if strings.HasPrefix(strings.TrimSpace(value), "[") {
			for !balancedTOMLArray(value) {
				if !scanner.Scan() {
					return nil, false
				}
				next, valid := stripTOMLComment(strings.TrimSpace(scanner.Text()))
				if !valid {
					return nil, false
				}
				value += "\n" + strings.TrimSpace(next)
			}
		}
		if !validTOMLValue(value) {
			return nil, false
		}
		fullKey := key
		if section != "" {
			fullKey = section + "." + key
		}
		if _, duplicate := values[fullKey]; duplicate {
			return nil, false
		}
		values[fullKey] = strings.TrimSpace(value)
	}
	return values, scanner.Err() == nil
}

func stripTOMLComment(line string) (string, bool) {
	quote := byte(0)
	escape := false
	for i := 0; i < len(line); i++ {
		ch := line[i]
		if quote != 0 {
			if quote == '"' && escape {
				escape = false
			} else if quote == '"' && ch == '\\' {
				escape = true
			} else if ch == quote {
				quote = 0
			}
			continue
		}
		if ch == '"' || ch == '\'' {
			quote = ch
		} else if ch == '#' {
			return line[:i], true
		}
	}
	return line, quote == 0
}

func parseTOMLHeader(line string) (string, bool) {
	array := strings.HasPrefix(line, "[[")
	start, end := 1, "]"
	if array {
		start, end = 2, "]]"
	}
	if !strings.HasSuffix(line, end) {
		return "", false
	}
	name := strings.TrimSpace(line[start : len(line)-len(end)])
	return name, name != "" && validTOMLKey(name)
}

func splitTOMLAssignment(line string) (string, string, bool) {
	quote := byte(0)
	escape := false
	for i := 0; i < len(line); i++ {
		ch := line[i]
		if quote != 0 {
			if quote == '"' && escape {
				escape = false
			} else if quote == '"' && ch == '\\' {
				escape = true
			} else if ch == quote {
				quote = 0
			}
			continue
		}
		if ch == '"' || ch == '\'' {
			quote = ch
		} else if ch == '=' {
			key, value := strings.TrimSpace(line[:i]), strings.TrimSpace(line[i+1:])
			return key, value, key != "" && value != ""
		}
	}
	return "", "", false
}

func validTOMLKey(key string) bool {
	for _, part := range strings.Split(key, ".") {
		part = strings.TrimSpace(part)
		if part == "" {
			return false
		}
		if (part[0] == '"' && part[len(part)-1] == '"') || (part[0] == '\'' && part[len(part)-1] == '\'') {
			continue
		}
		for _, ch := range part {
			if (ch < 'a' || ch > 'z') && (ch < 'A' || ch > 'Z') && (ch < '0' || ch > '9') && ch != '_' && ch != '-' {
				return false
			}
		}
	}
	return true
}

func validTOMLValue(value string) bool {
	value = strings.TrimSpace(value)
	if value == "true" || value == "false" || value == "inf" || value == "+inf" || value == "-inf" || value == "nan" || value == "+nan" || value == "-nan" {
		return true
	}
	if len(value) >= 2 && ((value[0] == '"' && value[len(value)-1] == '"') || (value[0] == '\'' && value[len(value)-1] == '\'')) {
		return true
	}
	if len(value) >= 2 && value[0] == '[' && value[len(value)-1] == ']' {
		return balancedTOMLArray(value)
	}
	if len(value) >= 2 && value[0] == '{' && value[len(value)-1] == '}' {
		return true
	}
	if value == "" {
		return false
	}
	first := value[0]
	if (first >= '0' && first <= '9') || first == '+' || first == '-' {
		return !strings.ContainsAny(value, "\t\r\n")
	}
	return false
}

func balancedTOMLArray(value string) bool {
	depth := 0
	quote := byte(0)
	escape := false
	for i := 0; i < len(value); i++ {
		ch := value[i]
		if quote != 0 {
			if quote == '"' && escape {
				escape = false
			} else if quote == '"' && ch == '\\' {
				escape = true
			} else if ch == quote {
				quote = 0
			}
			continue
		}
		switch ch {
		case '"', '\'':
			quote = ch
		case '[':
			depth++
		case ']':
			depth--
			if depth < 0 {
				return false
			}
		}
	}
	return depth == 0 && quote == 0
}
