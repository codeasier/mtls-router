package agent

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDetectReturnsAllAgentsAndRespectsEnvironmentPaths(t *testing.T) {
	home := t.TempDir()
	claudeDir := filepath.Join(home, "claude-config")
	openCodePath := filepath.Join(home, "open-code", "custom.jsonc")
	codexHome := filepath.Join(home, "codex-home")
	env := map[string]string{
		"CLAUDE_CONFIG_DIR": claudeDir,
		"OPENCODE_CONFIG":   openCodePath,
		"CODEX_HOME":        codexHome,
	}
	detector := Detector{
		HomeDir: home,
		Getenv:  func(key string) string { return env[key] },
		LookPath: func(name string) (string, error) {
			if name == "codex" {
				return "", errors.New("not found")
			}
			return filepath.Join(home, "bin", name), nil
		},
	}
	states, err := detector.Detect()
	if err != nil {
		t.Fatal(err)
	}
	if len(states) != 3 {
		t.Fatalf("state count = %d", len(states))
	}
	if !states[0].Detected || states[0].Path != filepath.Join(claudeDir, "settings.json") {
		t.Fatalf("Claude state = %#v", states[0])
	}
	if !states[1].Detected || states[1].Path != openCodePath || states[1].Format != FormatJSONC {
		t.Fatalf("opencode state = %#v", states[1])
	}
	if !states[1].pathOverridden {
		t.Fatal("opencode override provenance was not retained")
	}
	withoutOverride := env["OPENCODE_CONFIG"]
	env["OPENCODE_CONFIG"] = ""
	canonical := mustDetect(t, detector)
	if canonical[1].pathOverridden {
		t.Fatal("empty opencode override retained provenance")
	}
	env["OPENCODE_CONFIG"] = withoutOverride
	publicState := states[1]
	publicState.pathOverridden = false
	gotJSON, err := json.Marshal(states[1])
	if err != nil {
		t.Fatal(err)
	}
	wantJSON, err := json.Marshal(publicState)
	if err != nil {
		t.Fatal(err)
	}
	if string(gotJSON) != string(wantJSON) {
		t.Fatalf("override provenance changed serialized state: got %s want %s", gotJSON, wantJSON)
	}
	if !states[2].Detected || states[2].Command != "" || states[2].Path != filepath.Join(codexHome, "config.toml") {
		t.Fatalf("Codex state = %#v", states[2])
	}
}

func TestDetectTreatsSupportedAgentsAsConfigurableWithoutCLIOrConfig(t *testing.T) {
	home := t.TempDir()
	states := mustDetect(t, testDetector(home, nil))

	for _, state := range states {
		if !state.Detected || state.Command != "" || state.Exists || !state.Writable {
			t.Errorf("unsupported configurable state = %#v", state)
		}
	}
}

func TestDetectClaudeConfiguredInvalidAndMissing(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:19443","ANTHROPIC_AUTH_TOKEN":"claude-secret-canary","ANTHROPIC_MODEL":"dynamic-main","ANTHROPIC_DEFAULT_HAIKU_MODEL":"dynamic-main","ANTHROPIC_DEFAULT_SONNET_MODEL":"dynamic-sonnet","ANTHROPIC_DEFAULT_OPUS_MODEL":"dynamic-main"}}`)

	states := mustDetect(t, testDetector(home, map[string]bool{"claude": true}))
	if !states[0].Exists || !states[0].Writable || !states[0].Configured || states[0].Invalid {
		t.Fatalf("configured Claude state = %#v", states[0])
	}

	writeFile(t, path, `{"env":`)
	states = mustDetect(t, testDetector(home, map[string]bool{"claude": true}))
	if !states[0].Invalid || states[0].Configured {
		t.Fatalf("invalid Claude state = %#v", states[0])
	}

	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	states = mustDetect(t, testDetector(home, map[string]bool{"claude": true}))
	if states[0].Exists || states[0].Configured || states[0].Invalid || !states[0].Writable {
		t.Fatalf("missing Claude state = %#v", states[0])
	}
}

func TestDetectClaudeAcceptsUTF8BOM(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, "\xef\xbb\xbf"+`{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:19443","ANTHROPIC_AUTH_TOKEN":"claude-secret-canary","ANTHROPIC_MODEL":"dynamic-main","ANTHROPIC_DEFAULT_HAIKU_MODEL":"dynamic-main","ANTHROPIC_DEFAULT_SONNET_MODEL":"dynamic-sonnet","ANTHROPIC_DEFAULT_OPUS_MODEL":"dynamic-main"}}`)

	state := mustDetect(t, testDetector(home, map[string]bool{"claude": true}))[0]
	if !state.Exists || !state.Writable || !state.Configured || state.Invalid {
		t.Fatalf("BOM-prefixed Claude state = %#v", state)
	}
	if state.Recovery.Eligible || len(state.Recovery.Reasons) != 0 {
		t.Fatalf("valid BOM-prefixed config recovery = %#v", state.Recovery)
	}
}

func TestDetectClassifiesRecoveryEligibility(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{"env":`)
	state := mustDetect(t, testDetector(home, nil))[0]
	if !state.Invalid || !state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Files[0].Reasons, RecoverySyntaxInvalid) {
		t.Fatalf("syntax-invalid recovery = %#v", state)
	}

	writeFile(t, path, `[]`)
	state = mustDetect(t, testDetector(home, nil))[0]
	if !state.Invalid || state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Reasons, RecoveryUnsupportedStructure) {
		t.Fatalf("unsupported recovery = %#v", state)
	}

	writeFile(t, path, `{"env":[]}`)
	state = mustDetect(t, testDetector(home, nil))[0]
	if state.Invalid || state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Reasons, RecoveryUnsupportedStructure) {
		t.Fatalf("unsupported nested recovery = %#v", state)
	}
}

func TestDetectRecoveryRejectsOversizedNonRegularAndLinkedTargets(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, make([]byte, maxConfigSize+1), 0o600); err != nil {
		t.Fatal(err)
	}
	state := mustDetect(t, testDetector(home, nil))[0]
	if state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Reasons, RecoveryOversized) {
		t.Fatalf("oversized recovery = %#v", state.Recovery)
	}

	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(path, 0o700); err != nil {
		t.Fatal(err)
	}
	state = mustDetect(t, testDetector(home, nil))[0]
	if state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Reasons, RecoveryNonRegular) {
		t.Fatalf("non-regular recovery = %#v", state.Recovery)
	}

	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	target := filepath.Join(home, "target.json")
	writeFile(t, target, `{"env":`)
	if err := os.Symlink(target, path); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}
	state = mustDetect(t, testDetector(home, nil))[0]
	if state.Invalid || state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Reasons, RecoveryLinked) {
		t.Fatalf("linked recovery = %#v", state.Recovery)
	}
}

func TestDetectOpenCodeJSONCConfiguredAndProviderInvalid(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.jsonc")
	writeFile(t, path, `{
  // comments and trailing commas are accepted
  "provider": {
    "mtls-router": {
      "options": {
        "baseURL": "http://127.0.0.1:19099/v1//literal",
        "apiKey": "open-code-secret-canary",
      },
    },
  },
}`)
	states := mustDetect(t, testDetector(home, map[string]bool{"opencode": true}))
	if states[1].Format != FormatJSONC || states[1].Configured || states[1].Invalid {
		t.Fatalf("nonmatching JSONC state = %#v", states[1])
	}

	writeFile(t, path, `{"model":"mtls-router/dynamic-main","provider":{"mtls-router":{"npm":"@ai-sdk/openai-compatible","name":"mtls-router","options":{"baseURL":"http://127.0.0.1:19443/v1","apiKey":"open-code-secret-canary"},"models":{"dynamic-main":{"name":"dynamic-main"}}}}}`)
	states = mustDetect(t, testDetector(home, map[string]bool{"opencode": true}))
	if !states[1].Configured || states[1].Invalid {
		t.Fatalf("configured JSONC state = %#v", states[1])
	}

	writeFile(t, path, `{"provider":"invalid"}`)
	states = mustDetect(t, testDetector(home, map[string]bool{"opencode": true}))
	if !states[1].Invalid || states[1].Configured {
		t.Fatalf("invalid provider state = %#v", states[1])
	}
}

func TestDetectCodexConfiguredAndInvalid(t *testing.T) {
	home := t.TempDir()
	codexHome := filepath.Join(home, ".codex")
	configPath := filepath.Join(codexHome, "config.toml")
	authPath := filepath.Join(codexHome, "auth.json")
	writeFile(t, configPath, `model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true
approval_policy = "on-request"
notify = [
  "keep",
]

[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"

[features]
js_repl = false
`)
	writeFile(t, authPath, `{"OPENAI_API_KEY":"codex-secret-canary"}`)
	states := mustDetect(t, testDetector(home, nil))
	if !states[2].Detected || states[2].Configured || !states[2].Migratable || states[2].Invalid || !states[2].Writable {
		t.Fatalf("migratable Codex state = %#v", states[2])
	}

	writeFile(t, configPath, `model_provider = "mtls-router"
model = "dynamic-main"
cli_auth_credentials_store = "file"
model_reasoning_effort = "medium"
[model_providers.mtls-router]
name = "mtls-router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19443/v1"
`)
	writeFile(t, authPath, `{"auth_mode":"apikey","OPENAI_API_KEY":"codex-secret-canary"}`)
	states = mustDetect(t, testDetector(home, nil))
	if !states[2].Configured || states[2].Migratable || states[2].Invalid {
		t.Fatalf("configured Codex v2 state = %#v", states[2])
	}

	writeFile(t, configPath, "model =\n")
	states = mustDetect(t, testDetector(home, nil))
	if !states[2].Invalid || states[2].Configured {
		t.Fatalf("invalid Codex state = %#v", states[2])
	}

	if err := os.Remove(configPath); err != nil {
		t.Fatal(err)
	}
	writeFile(t, authPath, `{"OPENAI_API_KEY":`)
	states = mustDetect(t, testDetector(home, nil))
	if states[2].Exists || !states[2].Invalid || states[2].Configured {
		t.Fatalf("invalid auth-only Codex state = %#v", states[2])
	}
}

func TestDetectCodexAcceptsDotsInsideQuotedKeys(t *testing.T) {
	home := t.TempDir()
	codexHome := filepath.Join(home, ".codex")
	writeFile(t, filepath.Join(codexHome, "config.toml"), `model_provider = "mtls-router"
model = "dynamic-main"
cli_auth_credentials_store = "file"

[model_providers.mtls-router]
name = "mtls-router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19443/v1"

[projects.'e:\minecraft\example\.minecraft\versions\demo']
trust_level = "trusted"

[projects."e:\\minecraft\\example\\version.2\\demo"]
trust_level = "trusted"
`)
	writeFile(t, filepath.Join(codexHome, "auth.json"), `{"auth_mode":"apikey","OPENAI_API_KEY":"codex-secret-canary"}`)

	state := mustDetect(t, testDetector(home, map[string]bool{"codex": true}))[2]
	if !state.Exists || !state.Writable || !state.Configured || state.Invalid {
		t.Fatalf("Codex state with quoted dotted key = %#v", state)
	}
	if state.Recovery.Eligible || len(state.Recovery.Files) != 2 || state.Recovery.Files[0].Role != "config" || state.Recovery.Files[1].Role != "auth" {
		t.Fatalf("valid Codex recovery = %#v", state.Recovery)
	}
}

func TestDetectCodexRecoveryRequiresCompleteSafeFileSet(t *testing.T) {
	home := t.TempDir()
	codexHome := filepath.Join(home, ".codex")
	configPath := filepath.Join(codexHome, "config.toml")
	authPath := filepath.Join(codexHome, "auth.json")
	writeFile(t, configPath, "model =\n")
	writeFile(t, authPath, `{"OPENAI_API_KEY":"secret"}`)
	state := mustDetect(t, testDetector(home, nil))[2]
	if !state.Recovery.Eligible || len(state.Recovery.Files) != 2 || !hasRecoveryReason(state.Recovery.Files[0].Reasons, RecoverySyntaxInvalid) {
		t.Fatalf("Codex complete recovery = %#v", state.Recovery)
	}

	if err := os.Remove(authPath); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(authPath, 0o700); err != nil {
		t.Fatal(err)
	}
	state = mustDetect(t, testDetector(home, nil))[2]
	if state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Files[1].Reasons, RecoveryNonRegular) {
		t.Fatalf("unsafe Codex companion recovery = %#v", state.Recovery)
	}
}

func TestDetectCodexRejectsDuplicateTOMLKeys(t *testing.T) {
	home := t.TempDir()
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), "model = \"first\"\nmodel = \"second\"\n")

	state := mustDetect(t, testDetector(home, map[string]bool{"codex": true}))[2]
	if !state.Invalid || state.Configured {
		t.Fatalf("Codex state with duplicate TOML key = %#v", state)
	}
}

func TestDetectionResultCannotReturnStoredAPIKeys(t *testing.T) {
	home := t.TempDir()
	canaries := []string{"claude-secret-canary", "open-code-secret-canary", "codex-secret-canary"}
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:19099","ANTHROPIC_AUTH_TOKEN":"`+canaries[0]+`"}}`)
	writeFile(t, filepath.Join(home, ".config", "opencode", "opencode.json"), `{"provider":{"mtls-router":{"options":{"baseURL":"http://127.0.0.1:19099/v1","apiKey":"`+canaries[1]+`"}}}}`)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), `model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true
[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"`)
	writeFile(t, filepath.Join(home, ".codex", "auth.json"), `{"OPENAI_API_KEY":"`+canaries[2]+`"}`)

	states := mustDetect(t, testDetector(home, map[string]bool{"claude": true, "opencode": true, "codex": true}))
	encoded, err := json.Marshal(states)
	if err != nil {
		t.Fatal(err)
	}
	for _, canary := range canaries {
		if strings.Contains(string(encoded), canary) {
			t.Fatalf("detection result exposed a stored key: %s", encoded)
		}
	}
}

func testDetector(home string, commands map[string]bool) Detector {
	return Detector{
		HomeDir: home,
		Getenv:  func(string) string { return "" },
		LookPath: func(name string) (string, error) {
			if commands[name] {
				return filepath.Join(home, "bin", name), nil
			}
			return "", errors.New("not found")
		},
	}
}

func mustDetect(t *testing.T, detector Detector) []State {
	t.Helper()
	states, err := detector.Detect()
	if err != nil {
		t.Fatal(err)
	}
	return states
}

func hasRecoveryReason(reasons []RecoveryReason, want RecoveryReason) bool {
	for _, reason := range reasons {
		if reason == want {
			return true
		}
	}
	return false
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0600); err != nil {
		t.Fatal(err)
	}
}
