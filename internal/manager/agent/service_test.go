package agent

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const testAPIKey = "sk-agent-write-key-canary-7f2d"

func TestPreviewIsStructuredKeyFreeAndDoesNotWrite(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "manager-state")
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodeJSONC := filepath.Join(home, ".config", "opencode", "opencode.jsonc")
	codexConfig := filepath.Join(home, ".codex", "config.toml")
	codexAuth := filepath.Join(home, ".codex", "auth.json")
	writeFile(t, claudePath, `{"theme":"dark","env":{"ANTHROPIC_AUTH_TOKEN":"stored-claude-canary"}}`)
	writeFile(t, openCodeJSONC, `{
  // this formatting is intentionally lost
  "model": "anthropic/keep",
  "provider": {"anthropic": {"options": {"apiKey": "stored-opencode-canary"}}},
}`)
	writeFile(t, codexConfig, "approval_policy = \"on-request\"\n")
	writeFile(t, codexAuth, `{"OPENAI_API_KEY":"stored-codex-canary","extra":"drop"}`)

	service := newTestService(t, stateDir, home, map[string]bool{"claude": true, "opencode": true, "codex": true}, nil)
	assertOnlyLockState(t, stateDir)
	preview, err := service.Preview(context.Background(), []Kind{Codex, ClaudeCode, OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	if preview.RevisionToken == "" || len(preview.Agents) != 3 || len(preview.ModelConfig) == 0 || len(preview.Fragments) != 4 || preview.StateChange == nil {
		t.Fatalf("preview = %#v", preview)
	}
	if preview.Agents[0].Agent != ClaudeCode || preview.Agents[1].Agent != OpenCode || preview.Agents[2].Agent != Codex {
		t.Fatalf("Agent order = %#v", preview.Agents)
	}
	openCode := preview.Agents[1]
	if len(openCode.Files) != 1 || openCode.Files[0].Path != filepath.Join(filepath.Dir(openCodeJSONC), "opencode.json") {
		t.Fatalf("opencode files = %#v", openCode.Files)
	}
	if openCode.Files[0].Operation != OperationCreate || openCode.Files[0].Format != FormatJSON || !openCode.Files[0].Backup.Required {
		t.Fatalf("opencode migration = %#v", openCode.Files[0])
	}
	if openCode.Files[0].Warning != jsoncMigrationWarning || openCode.Files[0].Backup.Pattern == "" {
		t.Fatalf("opencode warning/backup = %#v", openCode.Files[0])
	}
	if len(preview.Agents[2].Files) != 2 || !preview.Agents[2].Files[1].ContainsAPIKey {
		t.Fatalf("Codex files = %#v", preview.Agents[2].Files)
	}

	encoded, err := json.Marshal(preview)
	if err != nil {
		t.Fatal(err)
	}
	for _, canary := range []string{testAPIKey, "stored-claude-canary", "stored-opencode-canary", "stored-codex-canary"} {
		if strings.Contains(string(encoded), canary) {
			t.Fatalf("preview exposed key canary %q: %s", canary, encoded)
		}
	}
	assertNoBackupFiles(t, home)
	if _, err := os.Stat(filepath.Join(stateDir, journalFileName)); !os.IsNotExist(err) {
		t.Fatalf("preview created transaction journal: %v", err)
	}
	if _, err := os.Stat(filepath.Join(filepath.Dir(openCodeJSONC), "opencode.json")); !os.IsNotExist(err) {
		t.Fatalf("preview created migration target: %v", err)
	}
}

func TestPreviewAcceptsAndPreservesCompatibleWindowsConfigs(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	codexPath := filepath.Join(home, ".codex", "config.toml")
	writeFile(t, claudePath, "\xef\xbb\xbf"+`{"theme":"dark","env":{"UNRELATED":"keep"}}`)
	writeFile(t, codexPath, `[projects.'e:\minecraft\example\.minecraft\versions\demo']
trust_level = "trusted"

[projects."e:\\minecraft\\example\\version.2\\demo"]
trust_level = "trusted"
`)

	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "codex": true}, nil)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode, Codex})
	if err != nil {
		t.Fatal(err)
	}
	if len(preview.Fragments) != 3 {
		t.Fatalf("preview fragments = %#v", preview.Fragments)
	}
	if _, err := service.Write(context.Background(), WriteRequest{
		Agents: []Kind{ClaudeCode, Codex}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey, ApproveCodexAuthChange: true,
	}); err != nil {
		t.Fatal(err)
	}

	claude, valid := decodeObject([]byte(readString(t, claudePath)))
	if !valid || !jsonStringEquals(claude["theme"], "dark") {
		t.Fatalf("Claude write did not preserve unrelated settings: %s", readString(t, claudePath))
	}
	codex, valid := decodeTOML([]byte(readString(t, codexPath)))
	if !valid {
		t.Fatalf("Codex write is invalid TOML: %s", readString(t, codexPath))
	}
	for _, projectPath := range []string{`e:\minecraft\example\.minecraft\versions\demo`, `e:\minecraft\example\version.2\demo`} {
		project, exists := tomlTable(codex, "projects", projectPath)
		if !exists || project["trust_level"] != "trusted" {
			t.Fatalf("Codex write did not preserve quoted dotted key %q: %#v", projectPath, codex)
		}
	}
}

func TestV2PreviewAcceptsReportedClaudeConfigurationWithoutFable(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "manager-state")
	claudePath := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, claudePath, `{"theme":"dark","env":{"ANTHROPIC_MODEL":"old-model"}}`)
	service := newTestService(t, stateDir, home, map[string]bool{"claude": true}, nil)
	models := []string{"gpt-5.6-luna", "gpt-5.6-terra", "gpt-5.6-sol"}
	discovery, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, models, modelconfig.CatalogClaims{
		Models: models, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "desktop",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "test", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	raw := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"gpt-5.6-luna","name":"luna"},"haiku":{"model":"gpt-5.6-luna","name":"luna"},"sonnet":{"model":"gpt-5.6-terra","name":"terra"},"opus":{"model":"gpt-5.6-sol","name":"sol"},"context_window":372000,"max_output_tokens":10000}}`)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode}, discovery.CatalogToken, raw)
	if err != nil {
		t.Fatal(err)
	}
	if len(preview.Fragments) != 1 || !bytes.Contains(preview.ModelConfig, []byte(`"context_window":372000`)) || bytes.Contains(preview.ModelConfig, []byte(`"fable"`)) {
		t.Fatalf("preview = %#v", preview)
	}
	content := preview.Fragments[0].Content
	for _, expected := range []string{"gpt-5.6-luna", "gpt-5.6-terra", "gpt-5.6-sol", `"CLAUDE_CODE_MAX_CONTEXT_TOKENS": "372000"`, `"CLAUDE_CODE_MAX_OUTPUT_TOKENS": "10000"`} {
		if !strings.Contains(content, expected) {
			t.Fatalf("preview fragment missing %q: %s", expected, content)
		}
	}
	if strings.Contains(content, "FABLE") {
		t.Fatalf("disabled Fable was rendered: %s", content)
	}
}

func TestV2PreviewRejectsModelOutsideCatalogWithoutTransactionArtifacts(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "manager-state")
	service, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, map[string]bool{"claude": true}, nil)})
	if err != nil {
		t.Fatal(err)
	}
	discovery, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"model-a"}, modelconfig.CatalogClaims{
		Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "test", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"provider/slash"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	_, err = service.Preview(context.Background(), []Kind{ClaudeCode}, discovery.CatalogToken, config)
	assertCode(t, err, CodeModelConfigInvalid)
	for _, path := range []string{
		filepath.Join(home, ".claude", "settings.json"),
		filepath.Join(stateDir, journalFileName),
		filepath.Join(stateDir, sidecarFileName),
	} {
		if _, statErr := os.Stat(path); !os.IsNotExist(statErr) {
			t.Fatalf("invalid preview created %s: %v", path, statErr)
		}
	}
	assertNoBackupFiles(t, home)
}

func TestCatalogTokensBindSimplifyAcrossServiceProcesses(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "manager-state")
	detector := testServiceDetector(home, map[string]bool{"claude": true}, nil)
	full, err := NewService(Options{StateDir: stateDir, Detector: detector})
	if err != nil {
		t.Fatal(err)
	}
	fullCatalog, err := full.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"provider/slash"}, modelconfig.CatalogClaims{
		Models: []string{"provider/slash"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "test", ProtocolVersion: "2", Simplify: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	fullConfig := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"provider/slash"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)

	compatible, err := NewService(Options{StateDir: stateDir, Detector: detector})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := compatible.Render(context.Background(), []Kind{ClaudeCode}, fullCatalog.CatalogToken, fullConfig); err != nil {
		t.Fatalf("same-policy render in another process: %v", err)
	}
	if _, err := compatible.Preview(context.Background(), []Kind{ClaudeCode}, fullCatalog.CatalogToken, fullConfig); err != nil {
		t.Fatalf("same-policy preview in another process: %v", err)
	}

	simplified, err := NewService(Options{StateDir: stateDir, Detector: detector, Simplify: true})
	if err != nil {
		t.Fatal(err)
	}
	_, err = simplified.Render(context.Background(), []Kind{ClaudeCode}, fullCatalog.CatalogToken, fullConfig)
	assertCode(t, err, CodeModelCatalogStale)
	if err.Error() != "Agent model catalog is stale" {
		t.Fatalf("render stale message = %q", err)
	}
	_, err = simplified.Preview(context.Background(), []Kind{ClaudeCode}, fullCatalog.CatalogToken, fullConfig)
	assertCode(t, err, CodeModelCatalogStale)
	if err.Error() != "Agent model catalog is stale" {
		t.Fatalf("preview stale message = %q", err)
	}

	simplifiedCatalog, err := simplified.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"model-a"}, modelconfig.CatalogClaims{
		Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "test", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	simplifiedConfig := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-a"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	_, err = compatible.Render(context.Background(), []Kind{ClaudeCode}, simplifiedCatalog.CatalogToken, simplifiedConfig)
	assertCode(t, err, CodeModelCatalogStale)
}

func TestRevisionTokenSurvivesOneRequestManagerProcesses(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "manager-state")
	detector := testServiceDetector(home, map[string]bool{"claude": true}, nil)

	previewService, err := NewService(Options{StateDir: stateDir, Detector: detector, LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	preview, err := previewService.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}

	writeService, err := NewService(Options{StateDir: stateDir, Detector: detector, LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := writeService.Write(context.Background(), WriteRequest{
		Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey,
	}); err != nil {
		t.Fatalf("write in a second manager process: %v", err)
	}

}

func TestPreviewMissingInvalidConflictAndAlreadyConfigured(t *testing.T) {
	t.Run("missing CLIs and configurations are ready to create", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil, nil)
		preview, err := service.Preview(context.Background(), []Kind{ClaudeCode, OpenCode, Codex})
		if err != nil {
			t.Fatal(err)
		}
		if len(preview.Agents) != 3 {
			t.Fatalf("preview Agents = %#v", preview.Agents)
		}
		for _, agent := range preview.Agents {
			for _, file := range agent.Files {
				if file.Operation != OperationCreate {
					t.Fatalf("preview file = %#v", file)
				}
			}
		}
	})

	t.Run("invalid JSON", func(t *testing.T) {
		home := t.TempDir()
		writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{"env":`)
		service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
		_, err := service.Preview(context.Background(), []Kind{ClaudeCode})
		assertCode(t, err, CodeConfigInvalid)
		assertNoBackupFiles(t, home)
	})

	t.Run("invalid JSONC", func(t *testing.T) {
		home := t.TempDir()
		writeFile(t, filepath.Join(home, ".config", "opencode", "opencode.jsonc"), `{"provider": {/* unterminated`)
		service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"opencode": true}, nil)
		_, err := service.Preview(context.Background(), []Kind{OpenCode})
		assertCode(t, err, CodeConfigInvalid)
		assertNoBackupFiles(t, home)
	})

	t.Run("invalid TOML", func(t *testing.T) {
		home := t.TempDir()
		writeFile(t, filepath.Join(home, ".codex", "config.toml"), "model =\n")
		service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"codex": true}, nil)
		_, err := service.Preview(context.Background(), []Kind{Codex})
		assertCode(t, err, CodeConfigInvalid)
		assertNoBackupFiles(t, home)
	})

	t.Run("invalid Codex auth JSON", func(t *testing.T) {
		home := t.TempDir()
		writeFile(t, filepath.Join(home, ".codex", "config.toml"), "approval_policy = \"on-request\"\n")
		writeFile(t, filepath.Join(home, ".codex", "auth.json"), `{"OPENAI_API_KEY":`)
		service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"codex": true}, nil)
		_, err := service.Preview(context.Background(), []Kind{Codex})
		assertCode(t, err, CodeConfigInvalid)
		assertNoBackupFiles(t, home)
	})

	t.Run("explicit JSONC override is normalized in place", func(t *testing.T) {
		home := t.TempDir()
		dir := filepath.Join(home, "opencode")
		jsoncPath := filepath.Join(dir, "custom.jsonc")
		jsonPath := filepath.Join(dir, "opencode.json")
		original := `{
  // preserve this in the backup only
  "model": "keep",
  "provider": {
    "anthropic": {"options": {"literal": "/* value */"}},
  },
}`
		sibling := "{\"sentinel\":\"do-not-overwrite\",\"spacing\":[1,2]}\n"
		writeFile(t, jsoncPath, original)
		writeFile(t, jsonPath, sibling)
		env := map[string]string{"OPENCODE_CONFIG": jsoncPath}
		service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"opencode": true}, env)
		preview, err := service.Preview(context.Background(), []Kind{OpenCode})
		if err != nil {
			t.Fatal(err)
		}
		file := preview.Agents[0].Files[0]
		if file.Path != jsoncPath || file.SourcePath != "" || file.Format != FormatJSON || file.Operation != OperationReplace || !file.Backup.Required || file.Warning != jsoncOverrideWarning {
			t.Fatalf("explicit JSONC preview = %#v", file)
		}
		if len(file.Preserves) == 0 {
			t.Fatalf("explicit JSONC preview did not describe preserved settings: %#v", file)
		}
		if got := readString(t, jsonPath); got != sibling {
			t.Fatalf("sibling changed during preview: %q", got)
		}
		result, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{OpenCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
		if err != nil {
			t.Fatal(err)
		}
		if len(result.Agents[0].Backups) != 1 {
			t.Fatalf("explicit JSONC write backups = %#v", result)
		}
		configured := readJSONObject(t, jsoncPath)
		providers := rawObject(t, configured["provider"])
		anthropic := rawObject(t, providers["anthropic"])
		options := rawObject(t, anthropic["options"])
		if jsonString(t, configured["model"]) != "keep" || jsonString(t, options["literal"]) != "/* value */" {
			t.Fatalf("unrelated explicit JSONC settings were not preserved: %s", readString(t, jsoncPath))
		}
		router := rawObject(t, providers["mtls-router"])
		routerOptions := rawObject(t, router["options"])
		if jsonString(t, routerOptions["apiKey"]) != testAPIKey {
			t.Fatalf("explicit JSONC provider was not configured: %s", readString(t, jsoncPath))
		}
		if strings.Contains(readString(t, jsoncPath), "preserve this in the backup only") {
			t.Fatal("explicit JSONC comments survived strict JSON normalization")
		}
		if got := readString(t, jsonPath); got != sibling {
			t.Fatalf("sibling changed during explicit JSONC write: %q", got)
		}
		if got := readString(t, result.Agents[0].Backups[0]); got != original {
			t.Fatalf("explicit JSONC backup = %q, want original bytes %q", got, original)
		}
	})

	t.Run("already configured is replace with preserve details", func(t *testing.T) {
		home := t.TempDir()
		writeFile(t, filepath.Join(home, ".config", "opencode", "opencode.json"), `{"model":"keep","provider":{"mtls-router":{"options":{"apiKey":"old"}}}}`)
		service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"opencode": true}, nil)
		preview, err := service.Preview(context.Background(), []Kind{OpenCode})
		if err != nil {
			t.Fatal(err)
		}
		file := preview.Agents[0].Files[0]
		if file.Operation != OperationReplace || len(file.Preserves) == 0 || !file.Backup.Required || !file.Backup.Sensitive {
			t.Fatalf("already configured preview = %#v", file)
		}
	})
}

func TestWriteAllAgentsPreservesExactManagedSemantics(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	codexConfig := filepath.Join(home, ".codex", "config.toml")
	codexAuth := filepath.Join(home, ".codex", "auth.json")
	writeFile(t, claudePath, `{"SENTINEL":"keep","env":{"OTHER":"drop"},"permissions":{"edit":"allow"}}`)
	writeFile(t, openCodePath, `{"model":"anthropic/keep","provider":{"anthropic":{"options":{"apiKey":"keep-provider"}},"mtls-router":{"old":true}}}`)
	writeFile(t, codexConfig, `model = "old"
model_provider = "openai"
disable_response_storage = false
approval_policy = "on-request"

[model_providers.custom]
name = "old"
base_url = "http://old"

[features]
js_repl = false
`)
	writeFile(t, codexAuth, `{"OPENAI_API_KEY":"old","extra":"drop"}`)

	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true, "codex": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode, Codex}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Agents) != 3 || !result.Agents[0].Success || !result.Agents[1].Success || !result.Agents[2].Success {
		t.Fatalf("write result = %#v", result)
	}
	if len(result.Agents[0].Backups) != 1 || len(result.Agents[1].Backups) != 1 || len(result.Agents[2].Backups) != 2 {
		t.Fatalf("backup result = %#v", result)
	}

	claude := readJSONObject(t, claudePath)
	env := rawObject(t, claude["env"])
	if jsonString(t, claude["SENTINEL"]) != "keep" || jsonString(t, env["ANTHROPIC_AUTH_TOKEN"]) != testAPIKey {
		t.Fatalf("Claude config = %s", readString(t, claudePath))
	}
	if jsonString(t, env["OTHER"]) != "drop" || jsonString(t, env["ANTHROPIC_DEFAULT_SONNET_MODEL"]) != "model-sonnet" {
		t.Fatalf("Claude managed env = %s", readString(t, claudePath))
	}

	openCode := readJSONObject(t, openCodePath)
	providers := rawObject(t, openCode["provider"])
	router := rawObject(t, providers["mtls-router"])
	options := rawObject(t, router["options"])
	models := rawObject(t, router["models"])
	if jsonString(t, openCode["model"]) != "anthropic/keep" || jsonString(t, options["apiKey"]) != testAPIKey {
		t.Fatalf("opencode config = %s", readString(t, openCodePath))
	}
	if _, ok := providers["anthropic"]; !ok || len(models) != 2 || models["model-primary"] == nil || models["model-sonnet"] == nil {
		t.Fatalf("opencode providers/models = %s", readString(t, openCodePath))
	}

	codex := readString(t, codexConfig)
	for _, expected := range []string{
		`model_provider = "mtls-router"`, `model = "model-primary"`, `cli_auth_credentials_store = "file"`,
		`[model_providers.mtls-router]`, `name = "mtls-router"`, `approval_policy = "on-request"`, `[features]`, `js_repl = false`,
	} {
		if !strings.Contains(codex, expected) {
			t.Fatalf("Codex config missing %q: %s", expected, codex)
		}
	}
	if !strings.Contains(codex, `name = "old"`) || !strings.Contains(codex, "disable_response_storage = false") {
		t.Fatalf("Codex user custom provider or obsolete setting handling failed: %s", codex)
	}
	auth := readJSONObject(t, codexAuth)
	if jsonString(t, auth["auth_mode"]) != "apikey" || jsonString(t, auth["OPENAI_API_KEY"]) != testAPIKey {
		t.Fatalf("Codex auth = %s", readString(t, codexAuth))
	}
	if _, err := os.Stat(service.journalPath()); !os.IsNotExist(err) {
		t.Fatalf("journal retained after commit: %v", err)
	}
	assertResultKeyFree(t, result)
}

func TestWriteCreatesMissingConfigurationsWithoutBackups(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true, "codex": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode, Codex}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	for _, agent := range preview.Agents {
		for _, file := range agent.Files {
			if file.Operation != OperationCreate || file.Backup.Required {
				t.Fatalf("missing-file preview = %#v", file)
			}
		}
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	if err != nil {
		t.Fatal(err)
	}
	for _, status := range result.Agents {
		if !status.Success || len(status.Backups) != 0 {
			t.Fatalf("missing-file result = %#v", status)
		}
	}
	for _, path := range []string{
		filepath.Join(home, ".claude", "settings.json"),
		filepath.Join(home, ".config", "opencode", "opencode.json"),
		filepath.Join(home, ".codex", "config.toml"),
		filepath.Join(home, ".codex", "auth.json"),
	} {
		if !isRegularFile(path) {
			t.Fatalf("missing configuration was not created: %s", path)
		}
	}
	assertResultKeyFree(t, result)
}

func TestWriteMigratesJSONCAndRetainsSensitiveBackup(t *testing.T) {
	home := t.TempDir()
	jsoncPath := filepath.Join(home, ".config", "opencode", "opencode.jsonc")
	jsonPath := filepath.Join(filepath.Dir(jsoncPath), "opencode.json")
	writeFile(t, jsoncPath, `{
 // keep values, not syntax
 "model": "keep",
 "provider": {"anthropic": {"options": {"literal": "/* value */"}}},
}`)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"opencode": true}, nil)
	preview, err := service.Preview(context.Background(), []Kind{OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{OpenCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	if err != nil {
		t.Fatal(err)
	}
	if !isRegularFile(jsoncPath) || !isRegularFile(jsonPath) || len(result.Agents[0].Backups) != 1 {
		t.Fatalf("migration files/result = %#v", result)
	}
	if got := jsonString(t, readJSONObject(t, jsonPath)["model"]); got != "keep" {
		t.Fatalf("migrated model = %q", got)
	}
	if !strings.Contains(readString(t, result.Agents[0].Backups[0]), "keep values") {
		t.Fatal("JSONC backup did not preserve original comments")
	}
}

func TestOpenCodeMissingJSONCOverrideCreatesExactPath(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, "custom-opencode")
	jsoncPath := filepath.Join(dir, "profile.jsonc")
	jsonPath := filepath.Join(dir, "opencode.json")
	sibling := `{"sentinel":"existing-json"}`
	writeFile(t, jsonPath, sibling)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"opencode": true}, map[string]string{"OPENCODE_CONFIG": jsoncPath})
	preview, err := service.Preview(context.Background(), []Kind{OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	file := preview.Agents[0].Files[0]
	if file.Path != jsoncPath || file.SourcePath != "" || file.Format != FormatJSON || file.Operation != OperationCreate || file.Backup.Required || file.Warning != "" {
		t.Fatalf("missing explicit JSONC preview = %#v", file)
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{OpenCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Agents[0].Backups) != 0 || !isRegularFile(jsoncPath) {
		t.Fatalf("missing explicit JSONC write = %#v", result)
	}
	if got := readString(t, jsonPath); got != sibling {
		t.Fatalf("sibling changed during missing explicit JSONC write: %q", got)
	}
	root := readJSONObject(t, jsoncPath)
	providers := rawObject(t, root["provider"])
	router := rawObject(t, providers["mtls-router"])
	options := rawObject(t, router["options"])
	if jsonString(t, options["apiKey"]) != testAPIKey {
		t.Fatalf("exact explicit JSONC path was not configured: %s", readString(t, jsoncPath))
	}
}

func TestOpenCodeExplicitJSONCRejectsStalePreview(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, "custom-opencode")
	jsoncPath := filepath.Join(dir, "profile.jsonc")
	writeFile(t, jsoncPath, "{\n  // preview bytes\n  \"sentinel\": \"before\",\n}\n")
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"opencode": true}, map[string]string{"OPENCODE_CONFIG": jsoncPath})
	preview, err := service.Preview(context.Background(), []Kind{OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	changed := "{\"sentinel\":\"changed-after-preview\"}\n"
	writeFile(t, jsoncPath, changed)
	_, err = service.Write(context.Background(), WriteRequest{Agents: []Kind{OpenCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodePreviewStale)
	if got := readString(t, jsoncPath); got != changed {
		t.Fatalf("stale explicit JSONC was modified: %q", got)
	}
	assertNoBackupFiles(t, home)
}

func TestWriteRejectsStalePreviewBeforeBackupOrMutation(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{"sentinel":"preview"}`)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, path, `{"sentinel":"changed"}`)
	_, err = service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodePreviewStale)
	if got := readString(t, path); !strings.Contains(got, "changed") {
		t.Fatalf("stale file was modified: %s", got)
	}
	assertNoBackupFiles(t, home)
}

func TestWriteBackupFailureDoesNotMutate(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	original := `{"sentinel":"keep"}`
	writeFile(t, path, original)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeBackup = func(string) error { return errors.New("injected backup failure") }
	_, err = service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeBackupFailed)
	if got := readString(t, path); got != original {
		t.Fatalf("backup failure modified source: %q", got)
	}
	assertNoBackupFiles(t, home)
}

func TestLaterBackupFailureCleansEarlierBackupsWithoutMutation(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	claudeOriginal := `{"sentinel":"claude"}`
	openCodeOriginal := `{"sentinel":"opencode"}`
	writeFile(t, claudePath, claudeOriginal)
	writeFile(t, openCodePath, openCodeOriginal)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeBackup = func(path string) error {
		if path == openCodePath {
			return errors.New("injected second backup failure")
		}
		return nil
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeBackupFailed)
	if readString(t, claudePath) != claudeOriginal || readString(t, openCodePath) != openCodeOriginal {
		t.Fatal("backup failure changed a target")
	}
	if len(result.Agents[0].Backups) != 0 || len(result.Agents[1].Backups) != 0 {
		t.Fatalf("removed backups remained in result: %#v", result)
	}
	assertNoBackupFiles(t, home)
}

func TestWriteFailureRollsBackAllAgentsAndRetainsBackups(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	claudeOriginal := `{"sentinel":"claude-original"}`
	openCodeOriginal := `{"sentinel":"opencode-original"}`
	writeFile(t, claudePath, claudeOriginal)
	writeFile(t, openCodePath, openCodeOriginal)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeReplace = func(path string) error {
		if path == openCodePath {
			return errors.New("injected write failure")
		}
		return nil
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeWriteFailed)
	if readString(t, claudePath) != claudeOriginal || readString(t, openCodePath) != openCodeOriginal {
		t.Fatalf("transaction was not restored: claude=%q opencode=%q", readString(t, claudePath), readString(t, openCodePath))
	}
	if !result.Agents[0].RolledBack || result.Agents[1].RolledBack || len(result.Agents[0].RollbackBackups) != 1 {
		t.Fatalf("rollback result = %#v", result)
	}
	if len(result.Agents[0].Backups) != 1 || len(result.Agents[1].Backups) != 1 {
		t.Fatalf("diagnostic backups not retained: %#v", result)
	}
	if !strings.Contains(readString(t, result.Agents[0].RollbackBackups[0]), testAPIKey) {
		t.Fatal("rollback diagnostic backup did not preserve failed replacement")
	}
	if _, err := os.Stat(service.journalPath()); !os.IsNotExist(err) {
		t.Fatalf("journal retained after successful rollback: %v", err)
	}
	assertResultKeyFree(t, result)
}

func TestCodexAuthFailureRollsBackConfigAndAuthWithoutTouchingKeyring(t *testing.T) {
	home := t.TempDir()
	configPath := filepath.Join(home, ".codex", "config.toml")
	authPath := filepath.Join(home, ".codex", "auth.json")
	configOriginal := "cli_auth_credentials_store = \"keyring\"\napproval_policy = \"on-request\"\n"
	authOriginal := `{"auth_mode":"chatgpt","tokens":{"access_token":"old"},"metadata":{"keyring_id":"untouched"}}`
	writeFile(t, configPath, configOriginal)
	writeFile(t, authPath, authOriginal)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"codex": true}, nil)
	preview, err := service.Preview(context.Background(), []Kind{Codex})
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeReplace = func(path string) error {
		if path == authPath {
			return errors.New("injected auth replacement failure")
		}
		return nil
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{Codex}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeWriteFailed)
	if got := readString(t, configPath); got != configOriginal {
		t.Fatalf("keyring config not restored: %q", got)
	}
	if got := readString(t, authPath); got != authOriginal {
		t.Fatalf("auth file not restored: %q", got)
	}
	if len(result.Agents) != 1 || !result.Agents[0].RolledBack {
		t.Fatalf("rollback result = %#v", result)
	}
}

func TestRollbackFailureDisablesWrites(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	writeFile(t, claudePath, `{"sentinel":"claude"}`)
	writeFile(t, openCodePath, `{"sentinel":"opencode"}`)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeReplace = func(path string) error {
		if path == openCodePath {
			return errors.New("injected write failure")
		}
		return nil
	}
	service.hooks.beforeRollback = func(string) error { return errors.New("injected rollback failure") }
	_, err = service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeRollbackFailed)
	if !service.WritesDisabled() {
		t.Fatal("writes were not disabled after rollback failure")
	}
	if _, err := os.Stat(service.journalPath()); err != nil {
		t.Fatalf("recovery journal missing after rollback failure: %v", err)
	}
	_, err = service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeRollbackFailed)
	if _, err := service.Preview(context.Background(), selected); err != nil {
		t.Fatalf("read-only preview should remain available: %v", err)
	}
}

func TestDeadlineAfterFirstReplacementRollsBack(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	claudeOriginal := `{"sentinel":"claude"}`
	openCodeOriginal := `{"sentinel":"opencode"}`
	writeFile(t, claudePath, claudeOriginal)
	writeFile(t, openCodePath, openCodeOriginal)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	service.hooks.afterReplace = func(string) { cancel() }
	result, err := service.Write(ctx, WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeOperationTimeout)
	if readString(t, claudePath) != claudeOriginal || readString(t, openCodePath) != openCodeOriginal {
		t.Fatal("deadline rollback did not restore transaction")
	}
	if !result.Agents[0].RolledBack || service.WritesDisabled() {
		t.Fatalf("deadline result/service = %#v disabled=%t", result, service.WritesDisabled())
	}
}

func TestJournalExistsWithoutKeyBeforeWriteAndTracksPerFileProgress(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	writeFile(t, claudePath, `{}`)
	writeFile(t, openCodePath, `{}`)
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	seenBefore := false
	service.hooks.beforeReplace = func(path string) error {
		journalText := readString(t, service.journalPath())
		if strings.Contains(journalText, testAPIKey) {
			t.Fatal("journal exposed API key before replacement")
		}
		if path == claudePath {
			seenBefore = true
		}
		return nil
	}
	progressChecked := false
	service.hooks.afterReplace = func(path string) {
		if path != claudePath {
			return
		}
		journal, err := decodeJournal(service.journalPath())
		if err != nil {
			t.Fatal(err)
		}
		if journal.Entries[0].Progress != progressReplaced || journal.Entries[1].Progress != progressPending {
			t.Fatalf("journal progress = %#v", journal.Entries)
		}
		if strings.Contains(readString(t, service.journalPath()), testAPIKey) {
			t.Fatal("progress journal exposed API key")
		}
		progressChecked = true
	}
	if _, err := service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey}); err != nil {
		t.Fatal(err)
	}
	if !seenBefore || !progressChecked {
		t.Fatalf("journal checks did not execute: before=%t progress=%t", seenBefore, progressChecked)
	}
}

func TestStartupRecoversManagerCrashAfterReplacement(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	path := filepath.Join(home, ".claude", "settings.json")
	original := `{"sentinel":"before-crash"}`
	writeFile(t, path, original)
	service := newTestService(t, stateDir, home, map[string]bool{"claude": true}, nil)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.afterReplace = func(string) { panic("simulated manager crash") }
	func() {
		defer func() {
			if recover() == nil {
				t.Fatal("expected simulated crash")
			}
		}()
		_, _ = service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	}()
	if !strings.Contains(readString(t, path), testAPIKey) {
		t.Fatal("simulated crash did not occur after replacement")
	}
	journalText := readString(t, service.journalPath())
	if strings.Contains(journalText, testAPIKey) {
		t.Fatal("crash journal exposed API key")
	}
	journal, err := decodeJournal(service.journalPath())
	if err != nil {
		t.Fatal(err)
	}
	backupPath := journal.Entries[0].BackupPath

	recovered, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, map[string]bool{"claude": true}, nil), LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	if recovered.WritesDisabled() || readString(t, path) != original {
		t.Fatalf("startup recovery failed: disabled=%t content=%q", recovered.WritesDisabled(), readString(t, path))
	}
	if _, err := os.Stat(backupPath); err != nil {
		t.Fatalf("diagnostic backup was not retained: %v", err)
	}
	if _, err := os.Stat(filepath.Join(stateDir, journalFileName)); !os.IsNotExist(err) {
		t.Fatalf("journal retained after startup recovery: %v", err)
	}
}

func TestStartupDiscardsDurablyCommittedJournalWithoutRollback(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	path := filepath.Join(home, ".claude", "settings.json")
	committed := []byte(`{"sentinel":"committed"}`)
	writeFile(t, path, string(committed))
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatal(err)
	}
	journal := transactionJournal{
		Version: 1, TransactionID: "committed-cleanup", Committed: true,
		Entries: []journalEntry{{
			Agent: ClaudeCode, TargetPath: path,
			PreRevision:  revisionForContent([]byte(`{"sentinel":"before"}`), 0o600),
			PostRevision: revisionForContent(committed, 0o600),
			BackupPath:   filepath.Join(filepath.Dir(path), "settings.json.bak-missing-after-commit"),
			RestoreFrom:  path, TargetMode: 0o600, Progress: progressReplaced,
		}},
	}
	content, err := json.Marshal(journal)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(stateDir, journalFileName), content, 0o600); err != nil {
		t.Fatal(err)
	}
	service, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, map[string]bool{"claude": true}, nil)})
	if err != nil {
		t.Fatal(err)
	}
	if service.WritesDisabled() || readString(t, path) != string(committed) {
		t.Fatalf("committed recovery changed target: disabled=%t content=%q", service.WritesDisabled(), readString(t, path))
	}
	if _, err := os.Stat(filepath.Join(stateDir, journalFileName)); !os.IsNotExist(err) {
		t.Fatalf("committed journal was not cleaned up: %v", err)
	}
}

func TestStartupRecoveryFailureReturnsDisabledService(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(stateDir, journalFileName), []byte(`{"api_key":"`+testAPIKey+`"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	service, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, nil, nil)})
	assertCode(t, err, CodeRollbackFailed)
	if service == nil || !service.WritesDisabled() || CodeOf(service.RecoveryError()) != CodeRollbackFailed {
		t.Fatalf("recovery state = service=%#v err=%v", service, err)
	}
	states, detectErr := service.Detect(context.Background())
	if detectErr != nil {
		t.Fatal(detectErr)
	}
	for _, state := range states {
		if state.Recovery.Eligible || !hasRecoveryReason(state.Recovery.Reasons, RecoveryWritesDisabled) {
			t.Fatalf("disabled service recovery metadata = %#v", state.Recovery)
		}
	}
	_, err = service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: "stale", APIKey: testAPIKey})
	assertCode(t, err, CodeRollbackFailed)
}

func TestStartupRecoveryFailsClosedOnUnrelatedPendingEdit(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	path := filepath.Join(home, ".claude", "settings.json")
	original := []byte(`{"sentinel":"before"}`)
	concurrent := `{"sentinel":"unrelated-concurrent-edit"}`
	writeFile(t, path, string(original))
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatal(err)
	}
	backupPath, err := createPrivateBackup(path, original, 0o600, "bak")
	if err != nil {
		t.Fatal(err)
	}
	journal := transactionJournal{
		Version: 1, TransactionID: "pending-conflict",
		Entries: []journalEntry{{
			Agent: ClaudeCode, TargetPath: path,
			PreRevision:  revisionForContent(original, 0o600),
			PostRevision: revisionForContent([]byte(`{"env":{"ANTHROPIC_AUTH_TOKEN":"not-stored"}}`), 0o600),
			BackupPath:   backupPath, RestoreFrom: path, TargetMode: 0o600, Progress: progressPending,
		}},
	}
	content, err := json.Marshal(journal)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(stateDir, journalFileName), content, 0o600); err != nil {
		t.Fatal(err)
	}
	writeFile(t, path, concurrent)
	service, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, map[string]bool{"claude": true}, nil)})
	assertCode(t, err, CodeRollbackFailed)
	if service == nil || !service.WritesDisabled() {
		t.Fatal("recovery conflict did not disable writes")
	}
	if got := readString(t, path); got != concurrent {
		t.Fatalf("recovery overwrote unrelated edit: %q", got)
	}
}

func TestRollbackRestoresTransactionFilesBeforeFailingOnConcurrentEdit(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	firstPath := filepath.Join(home, ".claude", "settings.json")
	secondPath := filepath.Join(home, ".config", "opencode", "opencode.json")
	firstOriginal := []byte(`{"sentinel":"first-before"}`)
	firstWritten := []byte(`{"sentinel":"first-transaction"}`)
	secondOriginal := []byte(`{"sentinel":"second-before"}`)
	secondConcurrent := `{"sentinel":"second-concurrent"}`
	writeFile(t, firstPath, string(firstWritten))
	writeFile(t, secondPath, secondConcurrent)
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatal(err)
	}
	firstBackup, err := createPrivateBackup(firstPath, firstOriginal, 0o600, "bak")
	if err != nil {
		t.Fatal(err)
	}
	secondBackup, err := createPrivateBackup(secondPath, secondOriginal, 0o600, "bak")
	if err != nil {
		t.Fatal(err)
	}
	journal := transactionJournal{Version: 1, TransactionID: "partial-with-conflict", Entries: []journalEntry{
		{Agent: ClaudeCode, TargetPath: firstPath, PreRevision: revisionForContent(firstOriginal, 0o600), PostRevision: revisionForContent(firstWritten, 0o600), BackupPath: firstBackup, RestoreFrom: firstPath, TargetMode: 0o600, Progress: progressReplaced},
		{Agent: OpenCode, TargetPath: secondPath, PreRevision: revisionForContent(secondOriginal, 0o600), PostRevision: revisionForContent([]byte(`{"sentinel":"second-transaction"}`), 0o600), BackupPath: secondBackup, RestoreFrom: secondPath, TargetMode: 0o600, Progress: progressPending},
	}}
	content, err := json.Marshal(journal)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(stateDir, journalFileName), content, 0o600); err != nil {
		t.Fatal(err)
	}
	service, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, map[string]bool{"claude": true, "opencode": true}, nil)})
	assertCode(t, err, CodeRollbackFailed)
	if service == nil || !service.WritesDisabled() {
		t.Fatal("conflicted recovery did not fail closed")
	}
	if got := readString(t, firstPath); got != string(firstOriginal) {
		t.Fatalf("transaction replacement was not restored: %q", got)
	}
	if got := readString(t, secondPath); got != secondConcurrent {
		t.Fatalf("concurrent edit was overwritten: %q", got)
	}
}

func TestBackupsAreCollisionResistantPrivateAndTargetsPreserveMode(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX mode-bit assertions are covered by Windows ACL-specific implementation and cross-compilation")
	}
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{"sentinel":"permissions"}`)
	if err := os.Chmod(path, 0o666); err != nil {
		t.Fatal(err)
	}
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
	first := writeOneClaude(t, service)
	second := writeOneClaude(t, service)
	if first == second {
		t.Fatalf("backup paths collided: %q", first)
	}
	for _, backup := range []string{first, second} {
		info, err := os.Stat(backup)
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm()&0o077 != 0 {
			t.Fatalf("backup permissions = %04o, want user-only", info.Mode().Perm())
		}
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o666 {
		t.Fatalf("target permissions = %04o, want preserved 0666", info.Mode().Perm())
	}
}

func TestPreviewRejectsNonWritableConfig(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows ACL writability requires a Windows runtime")
	}
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{}`)
	if err := os.Chmod(path, 0o400); err != nil {
		t.Fatal(err)
	}
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
	_, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	assertCode(t, err, CodeConfigNotWritable)
}

func TestRollbackBackupIsUserPrivate(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX mode-bit assertions are covered by Windows ACL-specific implementation and cross-compilation")
	}
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	writeFile(t, claudePath, `{}`)
	writeFile(t, openCodePath, `{}`)
	if err := os.Chmod(claudePath, 0o666); err != nil {
		t.Fatal(err)
	}
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
	selected := []Kind{ClaudeCode, OpenCode}
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeReplace = func(path string) error {
		if path == openCodePath {
			return errors.New("fail")
		}
		return nil
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	assertCode(t, err, CodeWriteFailed)
	backup := result.Agents[0].RollbackBackups[0]
	info, err := os.Stat(backup)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm()&0o077 != 0 {
		t.Fatalf("rollback backup permissions = %04o, want user-only", info.Mode().Perm())
	}
}

func writeOneClaude(t *testing.T, service *Service) string {
	t.Helper()
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	if err != nil {
		t.Fatal(err)
	}
	return result.Agents[0].Backups[0]
}

func newTestService(t *testing.T, stateDir, home string, commands map[string]bool, env map[string]string) *Service {
	t.Helper()
	service, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, commands, env), LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	return service
}

func legacyTestRenderInput() *LegacyRenderInput {
	name := "Primary"
	return &LegacyRenderInput{
		RouterBaseURL: "http://127.0.0.1:19099", APIBaseURL: "http://127.0.0.1:19099/v1",
		Config: &modelconfig.Config{Version: 1,
			Claude: &modelconfig.ClaudeConfig{
				Primary: modelconfig.Model{Model: "model-primary", Name: &name},
				Haiku:   modelconfig.ClaudeRole{InheritPrimary: true}, Sonnet: modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "model-sonnet"}}, Opus: modelconfig.ClaudeRole{InheritPrimary: true},
			},
			OpenCode: &modelconfig.OpenCodeConfig{DefaultModel: "model-primary", Models: map[string]modelconfig.OpenCodeModelConfig{"model-primary": {}, "model-sonnet": {}}},
			Codex:    &modelconfig.CodexConfig{Model: "model-primary"},
		},
	}
}

func testServiceDetector(home string, commands map[string]bool, env map[string]string) Detector {
	return Detector{
		HomeDir: home,
		Getenv:  func(key string) string { return env[key] },
		LookPath: func(name string) (string, error) {
			if commands[name] {
				return filepath.Join(home, "bin", name), nil
			}
			return "", errors.New("not found")
		},
	}
}

func assertCode(t *testing.T, err error, want ErrorCode) {
	t.Helper()
	if got := CodeOf(err); got != want {
		t.Fatalf("error = %v, code = %q, want %q", err, got, want)
	}
}

func assertNoBackupFiles(t *testing.T, root string) {
	t.Helper()
	err := filepath.WalkDir(root, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() && strings.Contains(entry.Name(), ".bak-") {
			t.Fatalf("unexpected backup file: %s", path)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func assertOnlyLockState(t *testing.T, stateDir string) {
	t.Helper()
	entries, err := os.ReadDir(stateDir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || entries[0].Name() != lockFileName {
		t.Fatalf("unexpected startup state: %#v", entries)
	}
}

func assertResultKeyFree(t *testing.T, result WriteResult) {
	t.Helper()
	encoded, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), testAPIKey) {
		t.Fatalf("write result exposed API key: %s", encoded)
	}
}

func readString(t *testing.T, path string) string {
	t.Helper()
	content, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(content)
}

func readJSONObject(t *testing.T, path string) map[string]json.RawMessage {
	t.Helper()
	root, valid := decodeObject([]byte(readString(t, path)))
	if !valid {
		t.Fatalf("invalid JSON object at %s", path)
	}
	return root
}

func rawObject(t *testing.T, raw json.RawMessage) map[string]json.RawMessage {
	t.Helper()
	value, valid := decodeObject(raw)
	if !valid {
		t.Fatalf("invalid nested JSON object: %s", raw)
	}
	return value
}

func jsonString(t *testing.T, raw json.RawMessage) string {
	t.Helper()
	var value string
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatalf("invalid JSON string %s: %v", raw, err)
	}
	return value
}
