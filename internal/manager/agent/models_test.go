package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const modelsSecretCanary = "models-result-secret-canary-4b19"

func TestDiscoverModelsReturnsTypedPrefillUnavailableDriftAndNoSecrets(t *testing.T) {
	home := t.TempDir()
	claudePath := filepath.Join(home, ".claude", "settings.json")
	openCodePath := filepath.Join(home, ".config", "opencode", "opencode.json")
	codexPath := filepath.Join(home, ".codex", "config.toml")
	writeModelFixture(t, claudePath, `{"unrelated":"`+modelsSecretCanary+`","env":{"ANTHROPIC_AUTH_TOKEN":"`+modelsSecretCanary+`","ANTHROPIC_MODEL":"model-a","ANTHROPIC_DEFAULT_HAIKU_MODEL":"model-a","ANTHROPIC_DEFAULT_SONNET_MODEL":"gone","ANTHROPIC_DEFAULT_OPUS_MODEL":"model-a","HEADERS":"`+modelsSecretCanary+`"}}`)
	writeModelFixture(t, openCodePath, `{"model":"mtls-router/model-b","headers":{"Authorization":"`+modelsSecretCanary+`"},"provider":{"mtls-router":{"options":{"apiKey":"`+modelsSecretCanary+`"},"models":{"model-b":{"name":"Safe name","reasoning":true,"limit":{"context":100,"output":20},"options":{"apiKey":"`+modelsSecretCanary+`"},"extra":{"secret":"`+modelsSecretCanary+`"}}}}}}`)
	writeModelFixture(t, codexPath, "model_provider = \"mtls-router\"\nmodel = \"model-a\"\nmodel_reasoning_effort = \"high\"\nmodel_context_window = 200\nsecret_canary = \""+modelsSecretCanary+"\"\n")
	writeModelFixture(t, filepath.Join(home, ".codex", "auth.json"), `{"OPENAI_API_KEY":"`+modelsSecretCanary+`"}`)

	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	selected := []Kind{ClaudeCode, OpenCode, Codex}
	result, err := service.DiscoverModels(context.Background(), selected, []string{"model-a", "model-b"}, modelconfig.CatalogClaims{
		Models: []string{"model-b", "model-a"}, Agents: []modelconfig.Agent{modelconfig.Claude, modelconfig.OpenCode, modelconfig.Codex},
		Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), modelsSecretCanary) || strings.Contains(string(encoded), "ANTHROPIC_AUTH_TOKEN") || strings.Contains(string(encoded), "apiKey") || strings.Contains(string(encoded), "HEADERS") {
		t.Fatalf("result leaked secret or raw fields: %s", encoded)
	}
	if got := strings.Join(result.Existing.UnavailableModels["claude"], ","); got != "gone" {
		t.Fatalf("Claude unavailable = %q", got)
	}
	if got := strings.Join(result.Existing.DriftedAgents, ","); got != "claude,opencode,codex" {
		t.Fatalf("drifted Agents = %q", got)
	}
	if strings.Contains(string(result.Existing.ModelConfig), `"claude"`) || !strings.Contains(string(result.Existing.ModelConfig), `"opencode"`) || !strings.Contains(string(result.Existing.ModelConfig), `"codex"`) {
		t.Fatalf("typed prefill = %s", result.Existing.ModelConfig)
	}
	if !strings.Contains(string(result.Existing.ModelConfig), `"reasoning":true`) || !strings.Contains(string(result.Existing.ModelConfig), `"context":100`) ||
		!strings.Contains(string(result.Existing.ModelConfig), `"reasoning_effort":"high"`) || !strings.Contains(string(result.Existing.ModelConfig), `"context_window":200`) {
		t.Fatalf("supported typed fields missing from prefill: %s", result.Existing.ModelConfig)
	}
	claims, err := service.signer.VerifyCatalog(result.CatalogToken)
	if err != nil || strings.Join(claims.Models, ",") != "model-a,model-b" || strings.Join(modelAgentsToStrings(claims.Agents), ",") != "claude,opencode,codex" {
		t.Fatalf("catalog claims = %+v, %v", claims, err)
	}
}

func TestDiscoverModelsNoStateReturnsEmptyKeyFreeResult(t *testing.T) {
	home := t.TempDir()
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{OpenCode}, []string{"model-a"}, modelconfig.CatalogClaims{
		Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.OpenCode}, Owner: "cli",
		RouterBaseURL: "http://[::1]:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	if string(result.Existing.ModelConfig) != "{}" || len(result.Existing.UnavailableModels) != 0 || len(result.Existing.DriftedAgents) != 0 || result.CatalogToken == "" {
		t.Fatalf("result = %+v", result)
	}
}

func TestDiscoverModelsPresetNoPresetReturnsEmptyObjects(t *testing.T) {
	home := t.TempDir()
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"model-a"}, modelconfig.CatalogClaims{
		Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	if string(result.Preset.ModelConfig) != `{}` || result.Preset.UnavailableAgents == nil || len(result.Preset.UnavailableAgents) != 0 {
		t.Fatalf("preset = %#v", result.Preset)
	}
}

func TestDiscoverModelsPresetSelectedScopeAndMixedValidity(t *testing.T) {
	home := t.TempDir()
	presetConfig, err := modelconfig.DecodeStructural([]byte(`{"version":1,"claude":{"primary":{"model":"shared"},"haiku":{"model":"missing-z"},"sonnet":{"model":"missing-a"},"opus":{"model":"missing-z"}},"opencode":{"default_model":"shared","models":{"shared":{"name":"Recommended"}}},"codex":{"model":"unrequested-missing"}}`))
	if err != nil {
		t.Fatal(err)
	}
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Preset: presetConfig, Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	// Mutating the caller's value after construction must not affect discovery.
	presetConfig.OpenCode.DefaultModel = "caller-mutated"
	presetConfig.OpenCode.Models["caller-mutated"] = modelconfig.OpenCodeModelConfig{}

	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode, OpenCode}, []string{"shared"}, modelconfig.CatalogClaims{
		Models: []string{"shared"}, Agents: []modelconfig.Agent{modelconfig.Claude, modelconfig.OpenCode}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Join(result.Preset.UnavailableAgents["claude"], ","); got != "missing-a,missing-z" {
		t.Fatalf("Claude missing = %q", got)
	}
	if _, reported := result.Preset.UnavailableAgents["codex"]; reported {
		t.Fatalf("unrequested Codex was reported: %#v", result.Preset.UnavailableAgents)
	}
	if strings.Contains(string(result.Preset.ModelConfig), `"claude"`) || strings.Contains(string(result.Preset.ModelConfig), `"codex"`) || strings.Contains(string(result.Preset.ModelConfig), "caller-mutated") {
		t.Fatalf("preset was repaired, uncropped, or mutated: %s", result.Preset.ModelConfig)
	}
	decoded, err := modelconfig.Decode(result.Preset.ModelConfig, []modelconfig.Agent{modelconfig.OpenCode}, []string{"shared"})
	if err != nil {
		t.Fatal(err)
	}
	if decoded.OpenCode == nil || decoded.OpenCode.DefaultModel != "shared" || decoded.OpenCode.Models["shared"].Name == nil {
		t.Fatalf("valid complete section changed: %#v", decoded.OpenCode)
	}
	for _, path := range []string{
		filepath.Join(home, "transactions", journalFileName),
		filepath.Join(home, "transactions", sidecarFileName),
		filepath.Join(home, ".claude", "settings.json"),
		filepath.Join(home, "opencode.json"),
		filepath.Join(home, ".codex", "config.toml"),
	} {
		if _, err := os.Stat(path); !os.IsNotExist(err) {
			t.Fatalf("preset discovery created %s: %v", path, err)
		}
	}
}

func TestDiscoverModelsPresetReportsCompleteBoundedUnavailableModels(t *testing.T) {
	home := t.TempDir()
	models := make(map[string]modelconfig.OpenCodeModelConfig, modelconfig.MaxReferencedModelsPerAgent)
	for i := 0; i < modelconfig.MaxReferencedModelsPerAgent; i++ {
		models[fmt.Sprintf("missing-%04d", i)] = modelconfig.OpenCodeModelConfig{}
	}
	presetConfig := &modelconfig.Config{Version: modelconfig.Version, OpenCode: &modelconfig.OpenCodeConfig{
		DefaultModel: "missing-0000",
		Models:       models,
	}}
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Preset: presetConfig, Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{OpenCode}, []string{"available"}, modelconfig.CatalogClaims{
		Models: []string{"available"}, Agents: []modelconfig.Agent{modelconfig.OpenCode}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	missing := result.Preset.UnavailableAgents["opencode"]
	if len(missing) != modelconfig.MaxReferencedModelsPerAgent || missing[0] != "missing-0000" || missing[len(missing)-1] != "missing-0999" {
		t.Fatalf("missing models boundary = %d, first %q, last %q", len(missing), missing[0], missing[len(missing)-1])
	}
	if string(result.Preset.ModelConfig) != "{}" {
		t.Fatalf("unavailable section was partially returned: %s", result.Preset.ModelConfig)
	}
}

func TestDiscoverModelsProjectsClaudeContextNameAndInheritance(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"primary[1m]","ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":"Primary","ANTHROPIC_DEFAULT_HAIKU_MODEL":"primary[1m]","ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":"Primary","ANTHROPIC_DEFAULT_SONNET_MODEL":"primary","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"Primary","ANTHROPIC_DEFAULT_OPUS_MODEL":"primary[1m]","ANTHROPIC_DEFAULT_OPUS_MODEL_NAME":"Different"}}`)
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"primary"}, modelconfig.CatalogClaims{
		Models: []string{"primary"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err != nil {
		t.Fatal(err)
	}
	config, err := modelconfig.Decode(result.Existing.ModelConfig, []modelconfig.Agent{modelconfig.Claude}, []string{"primary"})
	if err != nil {
		t.Fatal(err)
	}
	if config.Claude == nil || config.Claude.Primary.Context == nil || *config.Claude.Primary.Context != modelconfig.ClaudeContext1M {
		t.Fatalf("primary = %#v", config.Claude)
	}
	if !config.Claude.Haiku.InheritPrimary {
		t.Fatalf("equal selection did not inherit: %#v", config.Claude.Haiku)
	}
	if config.Claude.Sonnet.Selection == nil || config.Claude.Sonnet.Selection.Context != nil {
		t.Fatalf("different context was inherited: %#v", config.Claude.Sonnet)
	}
	if config.Claude.Opus.Selection == nil || config.Claude.Opus.Selection.Name == nil || *config.Claude.Opus.Selection.Name != "Different" {
		t.Fatalf("different name was inherited: %#v", config.Claude.Opus)
	}
}

func TestDiscoverModelsClaudeSuffixProjectionIsExact(t *testing.T) {
	for _, test := range []struct {
		name, value, unavailable string
		wantSection              bool
	}{
		{name: "middle marker stays base ID", value: "model[1m]-variant", unavailable: "model[1m]-variant"},
		{name: "alternate case stays base ID", value: "model[1M]", unavailable: "model[1M]"},
		{name: "repeated suffix rejected", value: "model[1m][1m]"},
		{name: "empty base rejected", value: "[1m]"},
		{name: "exact suffix uses available base", value: "model[1m]", wantSection: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			home := t.TempDir()
			path := filepath.Join(home, ".claude", "settings.json")
			writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"`+test.value+`","ANTHROPIC_DEFAULT_HAIKU_MODEL":"`+test.value+`","ANTHROPIC_DEFAULT_SONNET_MODEL":"`+test.value+`","ANTHROPIC_DEFAULT_OPUS_MODEL":"`+test.value+`"}}`)
			service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
				HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
			}})
			if err != nil {
				t.Fatal(err)
			}
			result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"model"}, modelconfig.CatalogClaims{
				Models: []string{"model"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
				RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2",
			})
			if err != nil {
				t.Fatal(err)
			}
			if got := strings.Join(result.Existing.UnavailableModels["claude"], ","); got != test.unavailable {
				t.Fatalf("unavailable = %q, want %q", got, test.unavailable)
			}
			hasSection := strings.Contains(string(result.Existing.ModelConfig), `"claude"`)
			if hasSection != test.wantSection {
				t.Fatalf("model config = %s, want section %t", result.Existing.ModelConfig, test.wantSection)
			}
		})
	}
}

func writeModelFixture(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
}

func modelAgentsToStrings(values []modelconfig.Agent) []string {
	result := make([]string, len(values))
	for index, value := range values {
		result[index] = string(value)
	}
	return result
}
