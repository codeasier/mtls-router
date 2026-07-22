package agent

import (
	"bytes"
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
	writeModelFixture(t, openCodePath, `{"model":"mtls-router/model-b","headers":{"Authorization":"`+modelsSecretCanary+`"},"provider":{"mtls-router":{"options":{"apiKey":"`+modelsSecretCanary+`"},"models":{"model-b":{"name":"Safe name","reasoning":true,"limit":{"context":100,"output":20},"options":{"reasoningEffort":"high"},"extra":{"secret":"`+modelsSecretCanary+`"}}}}}}`)
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
		Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
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
	if !strings.Contains(string(result.Existing.ModelConfig), `"reasoning":true`) || !strings.Contains(string(result.Existing.ModelConfig), `"context":100`) || !strings.Contains(string(result.Existing.ModelConfig), `"reasoningEffort":"high"`) ||
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
		RouterBaseURL: "http://[::1]:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
	})
	if err != nil {
		t.Fatal(err)
	}
	if string(result.Existing.ModelConfig) != "{}" || len(result.Existing.UnavailableModels) != 0 || len(result.Existing.DriftedAgents) != 0 || result.CatalogToken == "" {
		t.Fatalf("result = %+v", result)
	}
}

func TestDiscoverModelsAuthoritativelySignsServiceSimplifyPolicy(t *testing.T) {
	for _, simplify := range []bool{false, true} {
		home := t.TempDir()
		service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Simplify: simplify, Detector: Detector{
			HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
		}})
		if err != nil {
			t.Fatal(err)
		}
		result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"model-a"}, modelconfig.CatalogClaims{
			Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
			RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3", Simplify: !simplify,
		})
		if err != nil {
			t.Fatal(err)
		}
		claims, err := service.signer.VerifyCatalog(result.CatalogToken)
		if err != nil || claims.Simplify != simplify {
			t.Fatalf("simplify %t claims = %+v, %v", simplify, claims, err)
		}
	}
}

func TestDiscoverModelsReportsFilteredExistingAndPresetModelsUnavailable(t *testing.T) {
	home := t.TempDir()
	writeModelFixture(t, filepath.Join(home, ".claude", "settings.json"), `{"env":{"ANTHROPIC_MODEL":"provider/existing","ANTHROPIC_DEFAULT_HAIKU_MODEL":"provider/existing","ANTHROPIC_DEFAULT_SONNET_MODEL":"provider/existing","ANTHROPIC_DEFAULT_OPUS_MODEL":"provider/existing"}}`)
	preset, err := modelconfig.DecodeStructural([]byte(`{"version":1,"codex":{"model":"provider/preset"}}`))
	if err != nil {
		t.Fatal(err)
	}
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Preset: preset, Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode, Codex}, []string{"model-a"}, modelconfig.CatalogClaims{
		Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.Claude, modelconfig.Codex}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Join(result.Existing.UnavailableModels["claude"], ","); got != "provider/existing" {
		t.Fatalf("existing Claude unavailable = %q", got)
	}
	if got := strings.Join(result.Preset.UnavailableAgents["codex"], ","); got != "provider/preset" {
		t.Fatalf("preset Codex unavailable = %q", got)
	}
	if string(result.Existing.ModelConfig) != `{}` || string(result.Preset.ModelConfig) != `{}` {
		t.Fatalf("unavailable sections were returned: existing=%s preset=%s", result.Existing.ModelConfig, result.Preset.ModelConfig)
	}
	claims, err := service.signer.VerifyCatalog(result.CatalogToken)
	if err != nil || strings.Join(claims.Models, ",") != "model-a" {
		t.Fatalf("catalog claims = %+v, %v", claims, err)
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
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
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
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
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

func TestDiscoverModelsFablePresetAvailabilityIsClaudeAtomic(t *testing.T) {
	home := t.TempDir()
	presetConfig, err := modelconfig.DecodeStructural([]byte(`{"version":1,"claude":{"primary":{"model":"shared"},"fable":{"model":"fable-missing"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}},"opencode":{"default_model":"shared","models":{"shared":{}}}}`))
	if err != nil {
		t.Fatal(err)
	}
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Preset: presetConfig, Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode, OpenCode}, []string{"shared"}, modelconfig.CatalogClaims{
		Models: []string{"shared"}, Agents: []modelconfig.Agent{modelconfig.Claude, modelconfig.OpenCode}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Join(result.Preset.UnavailableAgents["claude"], ","); got != "fable-missing" {
		t.Fatalf("Claude unavailable = %q", got)
	}
	if strings.Contains(string(result.Preset.ModelConfig), `"claude"`) || !strings.Contains(string(result.Preset.ModelConfig), `"opencode"`) {
		t.Fatalf("preset section atomicity = %s", result.Preset.ModelConfig)
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
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
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
	writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"primary[1m]","ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":"Primary","ANTHROPIC_DEFAULT_FABLE_MODEL":"primary[1m]","ANTHROPIC_DEFAULT_FABLE_MODEL_NAME":"Primary","ANTHROPIC_DEFAULT_HAIKU_MODEL":"primary[1m]","ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":"Primary","ANTHROPIC_DEFAULT_SONNET_MODEL":"primary","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"Primary","ANTHROPIC_DEFAULT_OPUS_MODEL":"primary[1m]","ANTHROPIC_DEFAULT_OPUS_MODEL_NAME":"Different"}}`)
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"primary"}, modelconfig.CatalogClaims{
		Models: []string{"primary"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
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
	if config.Claude.Fable == nil || !config.Claude.Fable.InheritPrimary {
		t.Fatalf("equal Fable selection did not inherit: %#v", config.Claude.Fable)
	}
	if config.Claude.Sonnet.Selection == nil || config.Claude.Sonnet.Selection.Context != nil {
		t.Fatalf("different context was inherited: %#v", config.Claude.Sonnet)
	}
	if config.Claude.Opus.Selection == nil || config.Claude.Opus.Selection.Name == nil || *config.Claude.Opus.Selection.Name != "Different" {
		t.Fatalf("different name was inherited: %#v", config.Claude.Opus)
	}
}

func TestDiscoverModelsProjectsUppercaseClaudeContextSuffix(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"gpt-5.6-sol[1M]","ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":"Sol","ANTHROPIC_DEFAULT_HAIKU_MODEL":"gpt-5.6-sol[1M]","ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":"Sol","ANTHROPIC_DEFAULT_SONNET_MODEL":"gpt-5.4[1M]","ANTHROPIC_DEFAULT_OPUS_MODEL":"opus-4-8[1M]","ANTHROPIC_DEFAULT_FABLE_MODEL":"gpt-5.6-sol[1M]","ANTHROPIC_DEFAULT_FABLE_MODEL_NAME":"Sol"}}`)
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home,
		Getenv:  func(string) string { return "" },
		LookPath: func(string) (string, error) {
			return "", os.ErrNotExist
		},
	}})
	if err != nil {
		t.Fatal(err)
	}
	models := []string{"gpt-5.4", "gpt-5.6-sol", "opus-4-8"}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, models, modelconfig.CatalogClaims{
		Models: models, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli",
		RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := result.Existing.UnavailableModels["claude"]; len(got) != 0 {
		t.Fatalf("uppercase suffixes reported unavailable: %#v", got)
	}
	config, err := modelconfig.Decode(result.Existing.ModelConfig, []modelconfig.Agent{modelconfig.Claude}, models)
	if err != nil {
		t.Fatal(err)
	}
	oneMillion := modelconfig.ClaudeContext1M
	if config.Claude.Primary.Model != "gpt-5.6-sol" || !equalClaudeContext(config.Claude.Primary.Context, &oneMillion) {
		t.Fatalf("primary = %#v", config.Claude.Primary)
	}
	if !config.Claude.Haiku.InheritPrimary || config.Claude.Fable == nil || !config.Claude.Fable.InheritPrimary {
		t.Fatalf("normalized inheritance = haiku %#v, fable %#v", config.Claude.Haiku, config.Claude.Fable)
	}
	if config.Claude.Sonnet.Selection == nil || config.Claude.Sonnet.Selection.Model != "gpt-5.4" || !equalClaudeContext(config.Claude.Sonnet.Selection.Context, &oneMillion) {
		t.Fatalf("sonnet = %#v", config.Claude.Sonnet)
	}
	if config.Claude.Opus.Selection == nil || config.Claude.Opus.Selection.Model != "opus-4-8" || !equalClaudeContext(config.Claude.Opus.Selection.Context, &oneMillion) {
		t.Fatalf("opus = %#v", config.Claude.Opus)
	}
}

func TestDiscoverModelsFableNameAloneDoesNotEnable(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"primary","ANTHROPIC_DEFAULT_FABLE_MODEL_NAME":"Manual","ANTHROPIC_DEFAULT_HAIKU_MODEL":"primary","ANTHROPIC_DEFAULT_SONNET_MODEL":"primary","ANTHROPIC_DEFAULT_OPUS_MODEL":"primary"}}`)
	section, ids, ok := currentClaude(path)
	if !ok {
		t.Fatal("legacy Claude projection failed")
	}
	claude := section.(*modelconfig.ClaudeConfig)
	if claude.Fable != nil || strings.Join(ids, ",") != "primary,primary,primary,primary" {
		t.Fatalf("name-only Fable enabled: %#v ids=%#v", claude.Fable, ids)
	}
}

func TestDiscoverModelsRejectsMalformedEnabledFable(t *testing.T) {
	for _, value := range []string{`""`, `1`, `"[1m]"`, `"model[1m][1m]"`} {
		t.Run(value, func(t *testing.T) {
			home := t.TempDir()
			path := filepath.Join(home, ".claude", "settings.json")
			writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"primary","ANTHROPIC_DEFAULT_FABLE_MODEL":`+value+`,"ANTHROPIC_DEFAULT_HAIKU_MODEL":"primary","ANTHROPIC_DEFAULT_SONNET_MODEL":"primary","ANTHROPIC_DEFAULT_OPUS_MODEL":"primary"}}`)
			if _, _, ok := currentClaude(path); ok {
				t.Fatalf("malformed Fable projected: %s", value)
			}
		})
	}
}

func TestDiscoverModelsProjectsClaudeBudgets(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeModelFixture(t, path, `{"env":{"ANTHROPIC_MODEL":"primary","ANTHROPIC_DEFAULT_HAIKU_MODEL":"primary","ANTHROPIC_DEFAULT_SONNET_MODEL":"primary","ANTHROPIC_DEFAULT_OPUS_MODEL":"primary","CLAUDE_CODE_MAX_CONTEXT_TOKENS":"353400","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"100000"}}`)
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"primary"}, modelconfig.CatalogClaims{
		Models: []string{"primary"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
	})
	if err != nil {
		t.Fatal(err)
	}
	config, err := modelconfig.Decode(result.Existing.ModelConfig, []modelconfig.Agent{modelconfig.Claude}, []string{"primary"})
	if err != nil {
		t.Fatal(err)
	}
	if config.Claude.ContextWindow == nil || *config.Claude.ContextWindow != 353400 || config.Claude.MaxOutputTokens == nil || *config.Claude.MaxOutputTokens != 100000 {
		t.Fatalf("Claude budgets = %#v", config.Claude)
	}
}

func TestDiscoverModelsRejectsInvalidClaudeBudgets(t *testing.T) {
	for _, test := range []struct {
		name, contextWindow, maxOutput, model string
		wantNoUnavailable                     bool
	}{
		{name: "exponent", contextWindow: "35e4", maxOutput: "100000", model: "primary"},
		{name: "zero", contextWindow: "0", maxOutput: "", model: "primary"},
		{name: "negative", contextWindow: "-1", maxOutput: "", model: "primary"},
		{name: "leading zero", contextWindow: "0353400", maxOutput: "", model: "primary"},
		{name: "leading plus", contextWindow: "+353400", maxOutput: "", model: "primary"},
		{name: "unsafe", contextWindow: "9007199254740992", maxOutput: "", model: "primary"},
		{name: "non-string", contextWindow: "353400", maxOutput: `100000`, model: "primary"},
		{name: "output equals context", contextWindow: "100", maxOutput: "100", model: "primary"},
		{name: "output exceeds context", contextWindow: "100", maxOutput: "101", model: "primary"},
		{name: "numeric context and 1m", contextWindow: "353400", maxOutput: "", model: "primary[1m]"},
		{name: "numeric context and uppercase 1M", contextWindow: "353400", maxOutput: "", model: "primary[1M]", wantNoUnavailable: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			home := t.TempDir()
			env := map[string]any{
				"ANTHROPIC_MODEL": test.model, "ANTHROPIC_DEFAULT_HAIKU_MODEL": test.model,
				"ANTHROPIC_DEFAULT_SONNET_MODEL": test.model, "ANTHROPIC_DEFAULT_OPUS_MODEL": test.model,
			}
			if test.contextWindow != "" {
				env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] = test.contextWindow
			}
			if test.maxOutput != "" {
				if test.name == "non-string" {
					env["CLAUDE_CODE_MAX_OUTPUT_TOKENS"] = 100000
				} else {
					env["CLAUDE_CODE_MAX_OUTPUT_TOKENS"] = test.maxOutput
				}
			}
			content, _ := json.Marshal(map[string]any{"env": env})
			path := filepath.Join(home, ".claude", "settings.json")
			writeModelFixture(t, path, string(content))
			service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
				HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
			}})
			if err != nil {
				t.Fatal(err)
			}
			result, err := service.DiscoverModels(context.Background(), []Kind{ClaudeCode}, []string{"primary"}, modelconfig.CatalogClaims{
				Models: []string{"primary"}, Agents: []modelconfig.Agent{modelconfig.Claude}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
			})
			if err != nil {
				t.Fatal(err)
			}
			if test.wantNoUnavailable {
				if got := result.Existing.UnavailableModels["claude"]; len(got) != 0 {
					t.Fatalf("invalid budget reported unavailable models: %#v", got)
				}
			}
			if strings.Contains(string(result.Existing.ModelConfig), `"claude"`) {
				t.Fatalf("invalid budget projected: %s", result.Existing.ModelConfig)
			}
		})
	}
}

func TestDiscoverModelsProjectsOnlyTypedOpenCodeOptionsAndVariants(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	writeModelFixture(t, path, `{"model":"mtls-router/model-a","headers":{"Authorization":"secret"},"provider":{"mtls-router":{"npm":"unsafe","name":"unsafe","options":{"baseURL":"https://secret","apiKey":"secret"},"models":{"model-a":{"name":"Safe","options":{"reasoningEffort":"high"},"variants":{"medium":{"reasoningEffort":"medium"}},"extra":{"secret":"drop"},"provider":"drop","auth":"drop","url":"drop","headers":{"Authorization":"drop"}}}}}}`)
	service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
		HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := service.DiscoverModels(context.Background(), []Kind{OpenCode}, []string{"model-a"}, modelconfig.CatalogClaims{
		Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.OpenCode}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
	})
	if err != nil {
		t.Fatal(err)
	}
	text := string(result.Existing.ModelConfig)
	if !strings.Contains(text, `"options":{"reasoningEffort":"high"}`) || !strings.Contains(text, `"variants":{"medium":{"reasoningEffort":"medium"}}`) {
		t.Fatalf("typed fields missing: %s", text)
	}
	for _, prohibited := range []string{"secret", `"extra"`, `"provider"`, `"auth"`, `"url"`, `"headers"`, "baseURL", "apiKey"} {
		if strings.Contains(text, prohibited) {
			t.Fatalf("projected prohibited %q: %s", prohibited, text)
		}
	}
}

func TestDiscoverModelsRoundTripsRenderedOpenCodeVariantShapes(t *testing.T) {
	for _, test := range []struct {
		name       string
		variants   string
		wantConfig string
	}{
		{name: "legacy extension array", variants: `["fast"]`, wantConfig: `"extra":{"variants":["fast"]}`},
		{name: "typed object map", variants: `{"medium":{"reasoningEffort":"medium"}}`, wantConfig: `"variants":{"medium":{"reasoningEffort":"medium"}}`},
		{name: "invalid legacy extension", variants: `[{"safe":{"connection":"drop"}}]`},
	} {
		t.Run(test.name, func(t *testing.T) {
			home := t.TempDir()
			path := filepath.Join(home, ".config", "opencode", "opencode.json")
			writeModelFixture(t, path, `{"model":"mtls-router/model-a","provider":{"mtls-router":{"models":{"model-a":{"name":"Safe","variants":`+test.variants+`,"extra":{"secret":"drop"},"connection":{"token":"drop"}}}}}}`)
			service, err := NewService(Options{StateDir: filepath.Join(home, "transactions"), Detector: Detector{
				HomeDir: home, Getenv: func(string) string { return "" }, LookPath: func(string) (string, error) { return "", os.ErrNotExist },
			}})
			if err != nil {
				t.Fatal(err)
			}
			result, err := service.DiscoverModels(context.Background(), []Kind{OpenCode}, []string{"model-a"}, modelconfig.CatalogClaims{
				Models: []string{"model-a"}, Agents: []modelconfig.Agent{modelconfig.OpenCode}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
			})
			if err != nil {
				t.Fatal(err)
			}
			text := string(result.Existing.ModelConfig)
			if test.wantConfig == "" {
				if strings.Contains(text, `"opencode"`) {
					t.Fatalf("invalid legacy extension projected: %s", text)
				}
				return
			}
			if !strings.Contains(text, test.wantConfig) {
				t.Fatalf("projected config = %s", text)
			}
			for _, prohibited := range []string{"secret", "connection", "token"} {
				if strings.Contains(text, prohibited) {
					t.Fatalf("projected prohibited %q: %s", prohibited, text)
				}
			}
			config, err := modelconfig.Decode(result.Existing.ModelConfig, []modelconfig.Agent{modelconfig.OpenCode}, []string{"model-a"})
			if err != nil {
				t.Fatal(err)
			}
			rendered, err := renderOpenCodeFragment(config.OpenCode, "http://127.0.0.1:19099/v1", "redacted")
			if err != nil {
				t.Fatal(err)
			}
			var compact bytes.Buffer
			if err := json.Compact(&compact, rendered); err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(compact.String(), `"variants":`+test.variants) {
				t.Fatalf("rendered config = %s", rendered)
			}
		})
	}
}

func TestDiscoverModelsClaudeSuffixProjectionIsExact(t *testing.T) {
	for _, test := range []struct {
		name, value, unavailable string
		wantSection              bool
	}{
		{name: "middle marker stays base ID", value: "model[1m]-variant", unavailable: "model[1m]-variant"},
		{name: "uppercase middle marker stays base ID", value: "model[1M]-variant", unavailable: "model[1M]-variant"},
		{name: "unsupported mixed spelling stays base ID", value: "model[1Mm]", unavailable: "model[1Mm]"},
		{name: "repeated suffix rejected", value: "model[1m][1m]"},
		{name: "uppercase repeated suffix rejected", value: "model[1M][1M]"},
		{name: "lower then upper repeated suffix rejected", value: "model[1m][1M]"},
		{name: "upper then lower repeated suffix rejected", value: "model[1M][1m]"},
		{name: "lower compound marker and suffix rejected", value: "model[1m]-variant[1m]"},
		{name: "upper compound marker and lower suffix rejected", value: "model[1M]-variant[1m]"},
		{name: "lower compound marker and upper suffix rejected", value: "model[1m]-variant[1M]"},
		{name: "upper compound marker and suffix rejected", value: "model[1M]-variant[1M]"},
		{name: "empty base rejected", value: "[1m]"},
		{name: "uppercase empty base rejected", value: "[1M]"},
		{name: "exact suffix uses available base", value: "model[1m]", wantSection: true},
		{name: "exact uppercase suffix uses available base", value: "model[1M]", wantSection: true},
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
				RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "3",
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
