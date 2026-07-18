package agent

import (
	"encoding/json"
	"strconv"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

var codexManagedRootKeys = []string{
	"model_provider", "model", "cli_auth_credentials_store", "model_reasoning_effort",
	"model_reasoning_summary", "model_verbosity", "model_context_window",
	"model_auto_compact_token_limit", "model_auto_compact_token_limit_scope",
}

var codexRequiredRootKeys = []string{"model_provider", "model", "cli_auth_credentials_store"}

var codexCompetingAuthKeys = []string{"tokens", "last_refresh", "agent_identity", "personal_access_token", "bedrock_api_key"}

// CodexMergeAssessment keeps authentication approval separate from managed
// config ownership. Phase 4 binds these booleans into preview revisions.
type CodexMergeAssessment struct {
	ManagedConfigCollision bool
	AuthCollision          bool
	RequiresAuthApproval   bool
	HistoricalMigration    bool
}

func assessCodexMerge(config map[string]any, auth map[string]json.RawMessage) (CodexMergeAssessment, error) {
	if method, ok := config["forced_login_method"].(string); ok && method == "chatgpt" {
		return CodexMergeAssessment{}, operationError(CodeCodexAuthUnsupported, "Codex policy requires ChatGPT authentication")
	}
	assessment := CodexMergeAssessment{HistoricalMigration: exactHistoricalCodex(config, auth)}
	if provider, exists := func() (any, bool) {
		providers, ok := config["model_providers"].(map[string]any)
		if !ok {
			return nil, false
		}
		value, exists := providers["mtls-router"]
		return value, exists
	}(); exists {
		table, ok := provider.(map[string]any)
		assessment.ManagedConfigCollision = !ok || table["name"] != "mtls-router" || table["wire_api"] != "responses" || table["requires_openai_auth"] != true || !validAPIValue(table["base_url"])
	}
	mode, _ := rawString(auth["auth_mode"])
	assessment.AuthCollision = mode != "apikey" || !hasNonemptyJSONString(auth["OPENAI_API_KEY"])
	for _, key := range codexCompetingAuthKeys {
		if _, exists := auth[key]; exists {
			assessment.AuthCollision = true
		}
	}
	if store, ok := config["cli_auth_credentials_store"].(string); ok && store != "file" {
		assessment.AuthCollision = true
	}
	assessment.RequiresAuthApproval = assessment.AuthCollision || assessment.HistoricalMigration
	return assessment, nil
}

func renderCodexFragment(config *modelconfig.CodexConfig, apiBaseURL string) ([]byte, error) {
	return encodeTOML(codexManagedConfig(config, apiBaseURL))
}

func codexManagedConfig(config *modelconfig.CodexConfig, apiBaseURL string) map[string]any {
	result := map[string]any{
		"model_provider": "mtls-router", "model": config.Model,
		"cli_auth_credentials_store": "file",
		"model_providers": map[string]any{"mtls-router": map[string]any{
			"name": "mtls-router", "wire_api": "responses", "requires_openai_auth": true, "base_url": apiBaseURL,
		}},
	}
	if config.ReasoningEffort != nil {
		result["model_reasoning_effort"] = *config.ReasoningEffort
	}
	if config.ReasoningSummary != nil {
		result["model_reasoning_summary"] = *config.ReasoningSummary
	}
	if config.Verbosity != nil {
		result["model_verbosity"] = *config.Verbosity
	}
	if config.ContextWindow != nil {
		result["model_context_window"] = *config.ContextWindow
	}
	if config.AutoCompactTokenLimit != nil {
		result["model_auto_compact_token_limit"] = *config.AutoCompactTokenLimit
	}
	for key, value := range config.Extra {
		result[key] = value
	}
	return result
}

func mergeCodex(content []byte, config *modelconfig.CodexConfig, apiBaseURL string, ownedOptional []string, migrateHistorical bool) ([]byte, error) {
	root := map[string]any{}
	if len(content) != 0 {
		var ok bool
		root, ok = decodeTOML(content)
		if !ok {
			return nil, operationError(CodeConfigInvalid, "Codex configuration is invalid TOML")
		}
	}
	for _, key := range codexRequiredRootKeys {
		delete(root, key)
	}
	for _, key := range ownedOptional {
		delete(root, key)
	}
	if migrateHistorical {
		delete(root, "disable_response_storage")
	}
	providers, _ := root["model_providers"].(map[string]any)
	if providers == nil {
		providers = map[string]any{}
	}
	if migrateHistorical {
		delete(providers, "custom")
	}
	managed := codexManagedConfig(config, apiBaseURL)
	managedProviders := managed["model_providers"].(map[string]any)
	providers["mtls-router"] = managedProviders["mtls-router"]
	root["model_providers"] = providers
	delete(managed, "model_providers")
	for key, value := range managed {
		root[key] = value
	}
	return encodeTOML(root)
}

func renderCodexAuthFragment(key string, existing map[string]json.RawMessage) ([]byte, error) {
	result := cloneRawObject(existing)
	for _, name := range codexCompetingAuthKeys {
		delete(result, name)
	}
	result["auth_mode"] = json.RawMessage(strconv.Quote("apikey"))
	result["OPENAI_API_KEY"] = json.RawMessage(strconv.Quote(key))
	return marshalObject(result)
}

func exactHistoricalCodex(config map[string]any, auth map[string]json.RawMessage) bool {
	provider, ok := tomlTable(config, "model_providers", "custom")
	if !ok || len(provider) != 4 {
		return false
	}
	name, _ := provider["name"].(string)
	wire, _ := provider["wire_api"].(string)
	requires, _ := provider["requires_openai_auth"].(bool)
	baseOK := loopbackRouterURL(provider["base_url"], true)
	modelProvider, _ := config["model_provider"].(string)
	model, _ := config["model"].(string)
	disableStorage, disableOK := config["disable_response_storage"].(bool)
	_, authMode := rawString(auth["auth_mode"])
	return name == "9router" && wire == "responses" && requires && baseOK &&
		modelProvider == "custom" && model == "gpt-5.5" && disableOK && disableStorage && hasNonemptyJSONString(auth["OPENAI_API_KEY"]) &&
		(!authMode || jsonStringEquals(auth["auth_mode"], "apikey"))
}
