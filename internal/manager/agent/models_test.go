package agent

import (
	"context"
	"encoding/json"
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
