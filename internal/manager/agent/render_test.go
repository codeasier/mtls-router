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

func TestRenderCanonicalFragmentsAreRedactedDynamicAndFileIndependent(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	for _, path := range []string{filepath.Join(home, ".claude", "settings.json"), filepath.Join(home, ".config", "opencode", "opencode.json"), filepath.Join(home, ".codex", "config.toml")} {
		writeFile(t, path, "invalid-existing-secret-canary")
	}
	selected := []Kind{ClaudeCode, OpenCode, Codex}
	models := []string{"model-\"quoted", "model-primary", "model-sonnet"}
	token := renderCatalogToken(t, service, selected, models, "http://127.0.0.1:19443")
	raw := fullRenderConfig(t)
	result, err := service.Render(context.Background(), selected, token, raw)
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Fragments) != 4 || string(result.ModelConfig) == string(raw) {
		t.Fatalf("render result = %#v", result)
	}
	encoded, _ := json.Marshal(result)
	text := string(encoded)
	fragmentText := ""
	for _, fragment := range result.Fragments {
		fragmentText += fragment.Content
	}
	if strings.Contains(text, "invalid-existing-secret-canary") || !strings.Contains(fragmentText, RedactedAPIKey) {
		t.Fatalf("unsafe render output: %s", text)
	}
	for _, expected := range []string{"http://127.0.0.1:19443", "http://127.0.0.1:19443/v1", "model-primary", "model-sonnet", "model-\\\"quoted"} {
		if !strings.Contains(text, expected) {
			t.Fatalf("render missing %q: %s", expected, text)
		}
	}
	if strings.Contains(text, "gpt-5.5") || strings.Contains(text, "disable_response_storage") {
		t.Fatalf("static/obsolete output: %s", text)
	}
	if _, ok := decodeTOML([]byte(result.Fragments[2].Content)); !ok {
		t.Fatalf("Codex fragment is invalid TOML: %s", result.Fragments[2].Content)
	}
}

func TestClaudeMergePreservesUnrelatedValuesAndRemovesOnlyOwnedObsolete(t *testing.T) {
	config := legacyTestRenderInput().Config.Claude
	root, _ := decodeObject([]byte(`{"theme":"dark","env":{"OTHER":{"nested":true},"ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":"stale","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"stale","ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION":"owned-old","UNOWNED":"keep"}}`))
	content, err := mergeClaude(root, config, "http://127.0.0.1:19443", "secret", []string{"ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION"})
	if err != nil {
		t.Fatal(err)
	}
	merged, _ := decodeObject(content)
	env, _ := decodeObject(merged["env"])
	if jsonString(t, merged["theme"]) != "dark" || jsonString(t, env["UNOWNED"]) != "keep" || env["OTHER"] == nil {
		t.Fatalf("unrelated values lost: %s", content)
	}
	if _, exists := env["ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION"]; exists {
		t.Fatalf("owned obsolete extra survived: %s", content)
	}
	if _, exists := env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"]; exists {
		t.Fatalf("omitted optional name survived: %s", content)
	}
}

func TestClaudeInheritanceNameExtraAndDriftCombinations(t *testing.T) {
	primaryName := "Primary"
	explicitName := "Explicit"
	for _, test := range []struct {
		name       string
		primary    *string
		role       modelconfig.ClaudeRole
		extra      map[string]string
		wantModel  string
		wantName   *string
		staleExtra bool
	}{
		{name: "inherit named primary", primary: &primaryName, role: modelconfig.ClaudeRole{InheritPrimary: true}, wantModel: "primary", wantName: &primaryName},
		{name: "inherit unnamed primary", role: modelconfig.ClaudeRole{InheritPrimary: true}, wantModel: "primary"},
		{name: "explicit named role", primary: &primaryName, role: modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "role", Name: &explicitName}}, wantModel: "role", wantName: &explicitName},
		{name: "explicit unnamed role", primary: &primaryName, role: modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "role"}}, wantModel: "role"},
		{name: "owned extra replacement", role: modelconfig.ClaudeRole{InheritPrimary: true}, extra: map[string]string{"ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION": "new"}, wantModel: "primary", staleExtra: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			config := &modelconfig.ClaudeConfig{
				Primary: modelconfig.Model{Model: "primary", Name: test.primary},
				Haiku:   modelconfig.ClaudeRole{InheritPrimary: true}, Sonnet: test.role, Opus: modelconfig.ClaudeRole{InheritPrimary: true}, Extra: test.extra,
			}
			root, _ := decodeObject([]byte(`{"theme":"keep","env":{"UNRELATED":"keep","ANTHROPIC_DEFAULT_SONNET_MODEL":"drift","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"stale","ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION":"old"}}`))
			obsolete := []string(nil)
			if !test.staleExtra {
				obsolete = []string{"ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION"}
			}
			content, err := mergeClaude(root, config, "http://127.0.0.1:19099", "key", obsolete)
			if err != nil {
				t.Fatal(err)
			}
			merged, _ := decodeObject(content)
			env, _ := decodeObject(merged["env"])
			if jsonString(t, merged["theme"]) != "keep" || jsonString(t, env["UNRELATED"]) != "keep" || jsonString(t, env["ANTHROPIC_DEFAULT_SONNET_MODEL"]) != test.wantModel {
				t.Fatalf("merge = %s", content)
			}
			name, exists := env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"]
			if (test.wantName != nil) != exists || (exists && jsonString(t, name) != *test.wantName) {
				t.Fatalf("role name = %s, want %#v", content, test.wantName)
			}
			_, descriptionExists := env["ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION"]
			if test.staleExtra != descriptionExists {
				t.Fatalf("extra ownership = %s", content)
			}
		})
	}
}

func TestClaudeContextRenderingUsesEffectiveSelections(t *testing.T) {
	oneMillion := modelconfig.ClaudeContext1M
	primaryName := "Primary 1M"
	haikuName := "Haiku standard"
	config := &modelconfig.ClaudeConfig{
		Primary: modelconfig.Model{Model: "primary", Name: &primaryName, Context: &oneMillion},
		Fable:   &modelconfig.ClaudeRole{InheritPrimary: true},
		Haiku:   modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "haiku", Name: &haikuName}},
		Sonnet:  modelconfig.ClaudeRole{InheritPrimary: true},
		Opus:    modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "opus", Context: &oneMillion}},
	}
	env := claudeManagedEnv(config, "http://127.0.0.1:19099", "key")
	for key, want := range map[string]string{
		"ANTHROPIC_MODEL":                     "primary[1m]",
		"ANTHROPIC_CUSTOM_MODEL_OPTION":       "primary[1m]",
		"ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":  primaryName,
		"ANTHROPIC_DEFAULT_HAIKU_MODEL":       "haiku",
		"ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":  haikuName,
		"ANTHROPIC_DEFAULT_SONNET_MODEL":      "primary[1m]",
		"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": primaryName,
		"ANTHROPIC_DEFAULT_OPUS_MODEL":        "opus[1m]",
		"ANTHROPIC_DEFAULT_FABLE_MODEL":       "primary[1m]",
		"ANTHROPIC_DEFAULT_FABLE_MODEL_NAME":  primaryName,
	} {
		if env[key] != want {
			t.Errorf("%s = %q, want %q", key, env[key], want)
		}
	}
	if _, present := env["CLAUDE_CODE_DISABLE_1M_CONTEXT"]; present {
		t.Fatal("managed CLAUDE_CODE_DISABLE_1M_CONTEXT")
	}
}

func TestClaudeFableOwnershipAndDisabledManualPreservation(t *testing.T) {
	config := legacyTestRenderInput().Config.Claude
	root, _ := decodeObject([]byte(`{"env":{"ANTHROPIC_DEFAULT_FABLE_MODEL":"manual","ANTHROPIC_DEFAULT_FABLE_MODEL_NAME":"Manual","UNRELATED":"keep"}}`))
	content, err := mergeClaude(root, config, "http://127.0.0.1:19099", "key", nil)
	if err != nil {
		t.Fatal(err)
	}
	merged, _ := decodeObject(content)
	env, _ := decodeObject(merged["env"])
	if jsonString(t, env["ANTHROPIC_DEFAULT_FABLE_MODEL"]) != "manual" || jsonString(t, env["UNRELATED"]) != "keep" {
		t.Fatalf("disabled Fable changed manual values: %s", content)
	}

	config.Fable = &modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "managed"}}
	owned := claudeOwnedEnvKeys(config)
	ownedSet := make(map[string]bool, len(owned))
	for _, key := range owned {
		ownedSet[key] = true
	}
	if !ownedSet["ANTHROPIC_DEFAULT_FABLE_MODEL"] || !ownedSet["ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"] {
		t.Fatalf("enabled Fable ownership = %#v", owned)
	}
	content, err = mergeClaude(root, config, "http://127.0.0.1:19099", "key", nil)
	if err != nil {
		t.Fatal(err)
	}
	merged, _ = decodeObject(content)
	env, _ = decodeObject(merged["env"])
	if jsonString(t, env["ANTHROPIC_DEFAULT_FABLE_MODEL"]) != "managed" {
		t.Fatalf("enabled Fable not rendered: %s", content)
	}
}

func TestClaudeBudgetRenderingPreservesUnownedValues(t *testing.T) {
	contextWindow, maxOutput := int64(353400), int64(100000)
	config := legacyTestRenderInput().Config.Claude
	config.ContextWindow = &contextWindow
	config.MaxOutputTokens = &maxOutput
	env := claudeManagedEnv(config, "http://127.0.0.1:19099", "key")
	if env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] != "353400" || env["CLAUDE_CODE_MAX_OUTPUT_TOKENS"] != "100000" {
		t.Fatalf("budget env = %#v", env)
	}

	config.ContextWindow = nil
	config.MaxOutputTokens = nil
	root, _ := decodeObject([]byte(`{"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"999","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"888","UNRELATED":"keep"}}`))
	content, err := mergeClaude(root, config, "http://127.0.0.1:19099", "key", nil)
	if err != nil {
		t.Fatal(err)
	}
	merged, _ := decodeObject(content)
	mergedEnv, _ := decodeObject(merged["env"])
	if jsonString(t, mergedEnv["CLAUDE_CODE_MAX_CONTEXT_TOKENS"]) != "999" || jsonString(t, mergedEnv["CLAUDE_CODE_MAX_OUTPUT_TOKENS"]) != "888" || jsonString(t, mergedEnv["UNRELATED"]) != "keep" {
		t.Fatalf("unowned budgets were not preserved = %s", content)
	}
}

func TestOpenCodeExactSubsetTypedExtrasAndRootOwnership(t *testing.T) {
	input := legacyTestRenderInput()
	truth := true
	input.Config.OpenCode.Models = map[string]modelconfig.OpenCodeModelConfig{
		"model-primary": {Reasoning: &truth, Options: map[string]any{"reasoningEffort": "high"}, Variants: map[string]map[string]any{"medium": {"reasoningEffort": "medium"}}, Extra: map[string]any{"status": "active"}},
	}
	root, _ := decodeObject([]byte(`{"model":"user/default","small_model":"keep","provider":{"other":{"keep":true},"mtls-router":{"models":{"stale":{}}}}}`))
	content, err := mergeOpenCode(root, input.Config.OpenCode, input.APIBaseURL, "secret", false)
	if err != nil {
		t.Fatal(err)
	}
	merged, _ := decodeObject(content)
	if jsonString(t, merged["model"]) != "user/default" || jsonString(t, merged["small_model"]) != "keep" {
		t.Fatalf("unowned roots changed: %s", content)
	}
	providers, _ := decodeObject(merged["provider"])
	provider, _ := decodeObject(providers["mtls-router"])
	if jsonString(t, provider["name"]) != "CodeasierRouter" {
		t.Fatalf("provider display name = %s", provider["name"])
	}
	models, _ := decodeObject(provider["models"])
	if len(models) != 1 || models["stale"] != nil {
		t.Fatalf("selected subset not exact: %s", content)
	}
	model, _ := decodeObject(models["model-primary"])
	variants, _ := decodeObject(model["variants"])
	medium, _ := decodeObject(variants["medium"])
	if jsonString(t, medium["reasoningEffort"]) != "medium" {
		t.Fatalf("typed variants missing: %s", content)
	}
	content, _ = mergeOpenCode(root, input.Config.OpenCode, input.APIBaseURL, "secret", true)
	merged, _ = decodeObject(content)
	if jsonString(t, merged["model"]) != "mtls-router/model-primary" {
		t.Fatalf("owned root not replaced: %s", content)
	}
}

func TestOpenCodeLegacyExtraVariantsRenderAtModelTopLevel(t *testing.T) {
	input := legacyTestRenderInput()
	input.Config.OpenCode.Models = map[string]modelconfig.OpenCodeModelConfig{
		"model-primary": {Extra: map[string]any{"variants": map[string]any{"legacy": map[string]any{"reasoningEffort": "high"}}}},
	}
	content, err := renderOpenCodeFragment(input.Config.OpenCode, input.APIBaseURL, "secret")
	if err != nil {
		t.Fatal(err)
	}
	root, _ := decodeObject(content)
	providers, _ := decodeObject(root["provider"])
	provider, _ := decodeObject(providers["mtls-router"])
	models, _ := decodeObject(provider["models"])
	model, _ := decodeObject(models["model-primary"])
	variants, _ := decodeObject(model["variants"])
	legacy, _ := decodeObject(variants["legacy"])
	if jsonString(t, legacy["reasoningEffort"]) != "high" {
		t.Fatalf("legacy extra variants not rendered at model top level: %s", content)
	}
}

func TestOpenCodeCollisionEscapingRemovalAndOwnershipMatrix(t *testing.T) {
	input := legacyTestRenderInput()
	input.Config.OpenCode.DefaultModel = "quoted-\"-雪"
	input.Config.OpenCode.Models = map[string]modelconfig.OpenCodeModelConfig{"quoted-\"-雪": {}, "second": {}}
	for _, test := range []struct {
		name       string
		root       string
		ownModel   bool
		wantModel  string
		wantOther  bool
		wantSecond bool
	}{
		{name: "missing root is claimed", root: `{}`, wantModel: `mtls-router/quoted-"-雪`, wantSecond: true},
		{name: "unowned collision preserved", root: `{"model":"user/model","provider":{"other":{"keep":true},"mtls-router":{"models":{"stale":{}}}}}`, wantModel: "user/model", wantOther: true, wantSecond: true},
		{name: "owned collision replaced", root: `{"model":"old","provider":{"mtls-router":{"models":{"stale":{}}}}}`, ownModel: true, wantModel: `mtls-router/quoted-"-雪`, wantSecond: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			root, _ := decodeObject([]byte(test.root))
			content, err := mergeOpenCode(root, input.Config.OpenCode, input.APIBaseURL, "key-\"-雪", test.ownModel)
			if err != nil || !json.Valid(content) {
				t.Fatalf("invalid merge: %s: %v", content, err)
			}
			merged, _ := decodeObject(content)
			providers, _ := decodeObject(merged["provider"])
			provider, _ := decodeObject(providers["mtls-router"])
			models, _ := decodeObject(provider["models"])
			options, _ := decodeObject(provider["options"])
			if jsonString(t, merged["model"]) != test.wantModel || len(models) != 2 || models["stale"] != nil || (models["second"] != nil) != test.wantSecond || jsonString(t, options["apiKey"]) != "key-\"-雪" {
				t.Fatalf("merge = %s", content)
			}
			_, hasOther := providers["other"]
			if hasOther != test.wantOther {
				t.Fatalf("provider ownership = %s", content)
			}
		})
	}
}

func TestCodexEncodingMigrationAuthAndPolicy(t *testing.T) {
	config := &modelconfig.CodexConfig{Model: "model-\"slash\\unicode-\u96ea"}
	content, err := renderCodexFragment(config, "http://[::1]:19443/v1")
	if err != nil {
		t.Fatal(err)
	}
	decoded, ok := decodeTOML(content)
	if !ok || decoded["model"] != config.Model || decoded["model_provider"] != "mtls-router" || decoded["disable_response_storage"] != nil {
		t.Fatalf("Codex TOML round trip failed: %s", content)
	}
	providers := decoded["model_providers"].(map[string]any)
	provider := providers["mtls-router"].(map[string]any)
	if provider["name"] != "CodeasierRouter" {
		t.Fatalf("provider display name = %#v", provider["name"])
	}
	auth, _ := decodeObject([]byte(`{"auth_mode":"chatgpt","tokens":{"access_token":"old"},"accepted_metadata":{"keep":true}}`))
	authContent, err := renderCodexAuthFragment("secret", auth)
	if err != nil {
		t.Fatal(err)
	}
	authOut, _ := decodeObject(authContent)
	if jsonString(t, authOut["auth_mode"]) != "apikey" || authOut["tokens"] != nil || authOut["accepted_metadata"] == nil {
		t.Fatalf("auth merge failed: %s", authContent)
	}
	historical, _ := decodeTOML([]byte(`model_provider="custom"
model="gpt-5.5"
disable_response_storage=true
[model_providers.custom]
name="9router"
wire_api="responses"
requires_openai_auth=true
base_url="http://127.0.0.1:19099/v1"`))
	historicalAuth, _ := decodeObject([]byte(`{"OPENAI_API_KEY":"old"}`))
	assessment, err := assessCodexMerge(historical, historicalAuth)
	if err != nil || !assessment.HistoricalMigration || !assessment.RequiresAuthApproval {
		t.Fatalf("historical assessment = %#v err=%v", assessment, err)
	}
	historical["model"] = "changed"
	assessment, _ = assessCodexMerge(historical, historicalAuth)
	if assessment.HistoricalMigration {
		t.Fatal("partial historical signature was migrated")
	}
	_, err = assessCodexMerge(map[string]any{"forced_login_method": "chatgpt"}, map[string]json.RawMessage{})
	if CodeOf(err) != CodeCodexAuthUnsupported {
		t.Fatalf("forced policy error = %v", err)
	}
	for _, name := range []string{"mtls-router", "CodeasierRouter"} {
		managed := map[string]any{"model_providers": map[string]any{"mtls-router": map[string]any{
			"name": name, "wire_api": "responses", "requires_openai_auth": true, "base_url": "http://127.0.0.1:19443/v1",
		}}}
		assessment, err = assessCodexMerge(managed, map[string]json.RawMessage{"auth_mode": json.RawMessage(`"apikey"`), "OPENAI_API_KEY": json.RawMessage(`"key"`)})
		if err != nil || assessment.ManagedConfigCollision {
			t.Fatalf("managed provider name %q assessed as collision: %#v err=%v", name, assessment, err)
		}
	}
}

func TestCodexHistoricalSignatureRequiresEveryProviderRootAndAuthField(t *testing.T) {
	baseConfig := `model_provider="custom"
model="gpt-5.5"
disable_response_storage=true
[model_providers.custom]
name="9router"
wire_api="responses"
requires_openai_auth=true
base_url="http://127.0.0.1:19099/v1"`
	baseAuth := `{"auth_mode":"apikey","OPENAI_API_KEY":"old"}`
	mutations := []struct{ name, old, new string }{
		{"provider name", `name="9router"`, `name="user"`}, {"wire api", `wire_api="responses"`, `wire_api="chat"`},
		{"requires auth", `requires_openai_auth=true`, `requires_openai_auth=false`}, {"base URL", `base_url="http://127.0.0.1:19099/v1"`, `base_url="https://example.test/v1"`},
		{"root provider", `model_provider="custom"`, `model_provider="other"`}, {"root model", `model="gpt-5.5"`, `model="other"`},
		{"storage flag", `disable_response_storage=true`, `disable_response_storage=false`}, {"extra provider field", `base_url="http://127.0.0.1:19099/v1"`, "base_url=\"http://127.0.0.1:19099/v1\"\nuser_owned=true"},
	}
	for _, test := range mutations {
		t.Run(test.name, func(t *testing.T) {
			config, ok := decodeTOML([]byte(strings.Replace(baseConfig, test.old, test.new, 1)))
			if !ok {
				t.Fatal("fixture TOML invalid")
			}
			auth, _ := decodeObject([]byte(baseAuth))
			if exactHistoricalCodex(config, auth) {
				t.Fatal("partial historical signature accepted")
			}
			merged, err := mergeCodex([]byte(strings.Replace(baseConfig, test.old, test.new, 1)), &modelconfig.CodexConfig{Model: "new"}, "http://127.0.0.1:19099/v1", nil, false)
			if err != nil || !strings.Contains(string(merged), "[model_providers.custom]") {
				t.Fatalf("partial custom provider was not preserved: %s: %v", merged, err)
			}
		})
	}
	for _, authText := range []string{`{}`, `{"OPENAI_API_KEY":""}`, `{"auth_mode":"chatgpt","OPENAI_API_KEY":"old"}`, `{"auth_mode":"apikey"}`} {
		config, _ := decodeTOML([]byte(baseConfig))
		auth, _ := decodeObject([]byte(authText))
		if exactHistoricalCodex(config, auth) {
			t.Fatalf("partial auth signature accepted: %s", authText)
		}
	}
}

func TestCodexAuthPolicyAndKeyringPermutations(t *testing.T) {
	for _, test := range []struct {
		name            string
		config          map[string]any
		auth            string
		wantApproval    bool
		wantUnsupported bool
	}{
		{name: "already file apikey", config: map[string]any{"cli_auth_credentials_store": "file"}, auth: `{"auth_mode":"apikey","OPENAI_API_KEY":"old"}`},
		{name: "keyring selection", config: map[string]any{"cli_auth_credentials_store": "keyring"}, auth: `{"auth_mode":"apikey","OPENAI_API_KEY":"old"}`, wantApproval: true},
		{name: "auto selection", config: map[string]any{"cli_auth_credentials_store": "auto"}, auth: `{"auth_mode":"apikey","OPENAI_API_KEY":"old"}`, wantApproval: true},
		{name: "chatgpt auth", config: map[string]any{}, auth: `{"auth_mode":"chatgpt","tokens":{"access_token":"old"}}`, wantApproval: true},
		{name: "forced chatgpt", config: map[string]any{"forced_login_method": "chatgpt"}, auth: `{}`, wantUnsupported: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			auth, _ := decodeObject([]byte(test.auth))
			assessment, err := assessCodexMerge(test.config, auth)
			if (CodeOf(err) == CodeCodexAuthUnsupported) != test.wantUnsupported || (!test.wantUnsupported && assessment.RequiresAuthApproval != test.wantApproval) {
				t.Fatalf("assessment=%#v err=%v", assessment, err)
			}
		})
	}
}

func TestRenderRejectsTokenScopeConfigAndUnsafePath(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	token := renderCatalogToken(t, service, []Kind{ClaudeCode}, []string{"model-primary", "model-sonnet"}, "http://127.0.0.1:19443")
	_, err := service.Render(context.Background(), []Kind{OpenCode}, token, fullRenderConfig(t))
	if CodeOf(err) != CodeModelCatalogStale {
		t.Fatalf("scope error = %v", err)
	}
	_, err = service.Render(context.Background(), []Kind{ClaudeCode}, token, json.RawMessage(`{"version":1,"claude":{"primary":{"model":"absent"}}}`))
	if CodeOf(err) != CodeModelConfigInvalid {
		t.Fatalf("config error = %v", err)
	}
	service.detector.HomeDir = filepath.Join(home, "unsafe\npath")
	_, err = service.Render(context.Background(), []Kind{ClaudeCode}, token, claudeOnlyConfig())
	if CodeOf(err) != CodeConfigInvalid {
		t.Fatalf("unsafe path error = %v", err)
	}
}

func renderCatalogToken(t *testing.T, service *Service, selected []Kind, models []string, baseURL string) string {
	t.Helper()
	if err := service.ensureSigner(); err != nil {
		t.Fatal(err)
	}
	agents := make([]modelconfig.Agent, len(selected))
	for i, kind := range selected {
		agents[i] = modelAgent(kind)
	}
	token, err := service.signer.SignCatalog(modelconfig.CatalogClaims{Models: models, Agents: agents, Owner: "cli", RouterBaseURL: baseURL, DeploymentID: "test-deployment", ProtocolVersion: "4"})
	if err != nil {
		t.Fatal(err)
	}
	return token
}

func fullRenderConfig(t *testing.T) json.RawMessage {
	t.Helper()
	return json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-primary","name":"Primary"},"haiku":{"inherit_primary":true},"sonnet":{"model":"model-sonnet"},"opus":{"model":"model-\"quoted"},"extra":{"ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION":"safe"}},"opencode":{"default_model":"model-primary","models":{"model-primary":{"reasoning":true,"limit":{"context":1000,"output":100},"options":{"reasoningEffort":"high"},"extra":{"status":"active"}},"model-\"quoted":{}}},"codex":{"model":"model-\"quoted","reasoning_effort":"high","context_window":1000,"auto_compact_token_limit":900,"extra":{"model_auto_compact_token_limit_scope":"body_after_prefix"}}}`)
}

func claudeOnlyConfig() json.RawMessage {
	return json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-primary"},"haiku":{"inherit_primary":true},"sonnet":{"model":"model-sonnet"},"opus":{"inherit_primary":true}}}`)
}

func TestRenderCreatesNoAgentFiles(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	service := newTestService(t, stateDir, home, nil)
	token := renderCatalogToken(t, service, []Kind{ClaudeCode}, []string{"model-primary", "model-sonnet"}, "http://127.0.0.1:19443")
	if _, err := service.Render(context.Background(), []Kind{ClaudeCode}, token, claudeOnlyConfig()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(home, ".claude", "settings.json")); !os.IsNotExist(err) {
		t.Fatalf("render created Agent file: %v", err)
	}
}

func TestCompatibilityManifestAndGeneratedFormats(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("testdata", "compatibility.json"))
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		ManifestVersion int    `json:"manifest_version"`
		Retrieved       string `json:"retrieved"`
		ClaudeCode      struct {
			Version   string `json:"tested_version"`
			Source    string `json:"source"`
			Revision  string `json:"revision"`
			SHA256    string `json:"sha256"`
			Integrity string `json:"integrity"`
			Evidence  struct {
				Platform                          string   `json:"platform"`
				Source                            string   `json:"source"`
				Revision                          string   `json:"revision"`
				SHA256                            string   `json:"sha256"`
				Integrity                         string   `json:"integrity"`
				BinaryPath                        string   `json:"binary_path"`
				BinarySHA256                      string   `json:"binary_sha256"`
				Extraction                        string   `json:"extraction"`
				CustomModelNameVariables          []string `json:"custom_model_name_variables"`
				SelectionSyntax                   string   `json:"selection_syntax"`
				TerminalSuffixMarker              string   `json:"terminal_suffix_marker"`
				RepeatedSuffixNormalizationMarker string   `json:"repeated_suffix_normalization_marker"`
			} `json:"implementation_evidence"`
		} `json:"claude_code"`
		OpenCode struct {
			Version      string `json:"tested_version"`
			Source       string `json:"source"`
			Revision     string `json:"revision"`
			SHA256       string `json:"sha256"`
			Integrity    string `json:"integrity"`
			SchemaSource string `json:"schema_source"`
			SchemaSHA256 string `json:"schema_sha256"`
		} `json:"opencode"`
		Codex struct {
			Version             string `json:"tested_version"`
			Source              string `json:"source"`
			Revision            string `json:"revision"`
			SHA256              string `json:"sha256"`
			Integrity           string `json:"integrity"`
			ConfigRevision      string `json:"config_revision"`
			ConfigArchiveSHA256 string `json:"config_source_archive_sha256"`
		} `json:"codex"`
	}
	if err := json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	if manifest.ManifestVersion != 1 || manifest.Retrieved == "" {
		t.Fatalf("invalid compatibility manifest header: %#v", manifest)
	}
	for name, pin := range map[string]struct{ version, source, revision, sha256, integrity string }{
		"claude":   {manifest.ClaudeCode.Version, manifest.ClaudeCode.Source, manifest.ClaudeCode.Revision, manifest.ClaudeCode.SHA256, manifest.ClaudeCode.Integrity},
		"opencode": {manifest.OpenCode.Version, manifest.OpenCode.Source, manifest.OpenCode.Revision, manifest.OpenCode.SHA256, manifest.OpenCode.Integrity},
		"codex":    {manifest.Codex.Version, manifest.Codex.Source, manifest.Codex.Revision, manifest.Codex.SHA256, manifest.Codex.Integrity},
	} {
		if pin.version == "" || !strings.HasPrefix(pin.source, "https://") || pin.revision == "" || len(pin.sha256) != 64 || !strings.HasPrefix(pin.integrity, "sha512-") {
			t.Fatalf("incomplete %s compatibility pin: %#v", name, pin)
		}
	}
	if !strings.HasPrefix(manifest.OpenCode.SchemaSource, "https://") || len(manifest.OpenCode.SchemaSHA256) != 64 || len(manifest.Codex.ConfigRevision) != 40 || len(manifest.Codex.ConfigArchiveSHA256) != 64 {
		t.Fatal("schema/source revisions are not pinned")
	}
	if manifest.ClaudeCode.Version != "2.1.214" || manifest.ClaudeCode.Evidence.Platform != "darwin-arm64" ||
		manifest.ClaudeCode.Evidence.Source != "https://registry.npmjs.org/@anthropic-ai/claude-code-darwin-arm64/-/claude-code-darwin-arm64-2.1.214.tgz" ||
		manifest.ClaudeCode.Evidence.Revision != "npm:@anthropic-ai/claude-code-darwin-arm64@2.1.214" ||
		manifest.ClaudeCode.Evidence.SHA256 != "063331d0cf00f73f21a2f94d779788c1a1ce783d2f11286a2b5fc77cfaaba6bb" ||
		manifest.ClaudeCode.Evidence.Integrity != "sha512-z99kjSImARBWdE6lGoCXSi83tbiabtIv7vtFyuwrHD56WZTFSguedBb9F8wlUncEEfUVtqHKa9nCZ55j6spiIA==" ||
		manifest.ClaudeCode.Evidence.BinaryPath != "package/claude" || manifest.ClaudeCode.Evidence.BinarySHA256 != "59796dd18e9d77f1256f367db6d28ce4bd9cd5968e402ad3a327aac36abc6dec" ||
		manifest.ClaudeCode.Evidence.Extraction != "tar -xzf ARTIFACT && strings -a package/claude" {
		t.Fatalf("incomplete Claude implementation evidence: %#v", manifest.ClaudeCode.Evidence)
	}
	wantNameVariables := []string{
		"ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
		"ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
		"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
		"ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
	}
	if strings.Join(manifest.ClaudeCode.Evidence.CustomModelNameVariables, "\n") != strings.Join(wantNameVariables, "\n") ||
		manifest.ClaudeCode.Evidence.SelectionSyntax != "[1m]" || manifest.ClaudeCode.Evidence.TerminalSuffixMarker != `\[1m\]$` ||
		manifest.ClaudeCode.Evidence.RepeatedSuffixNormalizationMarker != `(\[1m\])+$` {
		t.Fatalf("Claude compatibility markers changed: %#v", manifest.ClaudeCode.Evidence)
	}

	oneMillion := modelconfig.ClaudeContext1M
	primaryName, haikuName, sonnetName, opusName := "Primary", "Fast", "Balanced", "Deep"
	compatibilityConfig := &modelconfig.ClaudeConfig{
		Primary: modelconfig.Model{Model: "primary", Name: &primaryName, Context: &oneMillion},
		Haiku:   modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "haiku", Name: &haikuName}},
		Sonnet:  modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "sonnet", Name: &sonnetName, Context: &oneMillion}},
		Opus:    modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "opus", Name: &opusName, Context: &oneMillion}},
	}
	compatibilityEnv := claudeManagedEnv(compatibilityConfig, "http://127.0.0.1:19099", RedactedAPIKey)
	for key, want := range map[string]string{
		"ANTHROPIC_MODEL":                     "primary[1m]",
		"ANTHROPIC_CUSTOM_MODEL_OPTION":       "primary[1m]",
		"ANTHROPIC_CUSTOM_MODEL_OPTION_NAME":  primaryName,
		"ANTHROPIC_DEFAULT_HAIKU_MODEL":       "haiku",
		"ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME":  haikuName,
		"ANTHROPIC_DEFAULT_SONNET_MODEL":      "sonnet[1m]",
		"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": sonnetName,
		"ANTHROPIC_DEFAULT_OPUS_MODEL":        "opus[1m]",
		"ANTHROPIC_DEFAULT_OPUS_MODEL_NAME":   opusName,
	} {
		if got := compatibilityEnv[key]; got != want {
			t.Errorf("Claude compatibility env %s = %q, want %q", key, got, want)
		}
	}

	input := legacyTestRenderInput()
	claude, err := renderClaudeFragment(input.Config.Claude, input.RouterBaseURL, RedactedAPIKey)
	if err != nil || !json.Valid(claude) {
		t.Fatalf("pinned Claude settings are invalid JSON: %v", err)
	}
	opencode, err := renderOpenCodeFragment(input.Config.OpenCode, input.APIBaseURL, RedactedAPIKey)
	if err != nil || !json.Valid(opencode) {
		t.Fatalf("pinned opencode config is invalid JSON: %v", err)
	}
	codex, err := renderCodexFragment(input.Config.Codex, input.APIBaseURL)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := decodeTOML(codex); !ok {
		t.Fatal("pinned Codex config is invalid TOML")
	}
}
