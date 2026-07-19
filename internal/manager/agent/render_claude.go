package agent

import (
	"bytes"
	"encoding/json"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

var claudeFixedEnvKeys = []string{
	"ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_MODEL",
	"ANTHROPIC_CUSTOM_MODEL_OPTION", "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
	"ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
	"ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
	"ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
	"ENABLE_TOOL_SEARCH", "DISABLE_AUTOUPDATER",
}

func renderClaudeFragment(config *modelconfig.ClaudeConfig, routerBaseURL, key string) ([]byte, error) {
	env := claudeManagedEnv(config, routerBaseURL, key)
	return marshalIndentedJSON(map[string]any{"env": env})
}

func mergeClaude(root map[string]json.RawMessage, config *modelconfig.ClaudeConfig, routerBaseURL, key string, obsoleteOwnedExtras []string) ([]byte, error) {
	result := cloneRawObject(root)
	env := make(map[string]json.RawMessage)
	if raw, exists := result["env"]; exists && !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		var valid bool
		env, valid = decodeObject(raw)
		if !valid {
			return nil, operationError(CodeConfigInvalid, "Claude Code env setting is not an object")
		}
	}
	for _, key := range claudeFixedEnvKeys {
		delete(env, key)
	}
	for _, key := range obsoleteOwnedExtras {
		delete(env, key)
	}
	for name, value := range claudeManagedEnv(config, routerBaseURL, key) {
		raw, err := json.Marshal(value)
		if err != nil {
			return nil, err
		}
		env[name] = raw
	}
	envJSON, err := json.Marshal(env)
	if err != nil {
		return nil, err
	}
	result["env"] = envJSON
	return marshalObject(result)
}

func claudeManagedEnv(config *modelconfig.ClaudeConfig, routerBaseURL, key string) map[string]string {
	result := map[string]string{
		"ANTHROPIC_BASE_URL":            routerBaseURL,
		"ANTHROPIC_AUTH_TOKEN":          key,
		"ANTHROPIC_MODEL":               claudeModelValue(config.Primary),
		"ANTHROPIC_CUSTOM_MODEL_OPTION": claudeModelValue(config.Primary),
		"ENABLE_TOOL_SEARCH":            "true",
		"DISABLE_AUTOUPDATER":           "1",
	}
	if config.Primary.Name != nil {
		result["ANTHROPIC_CUSTOM_MODEL_OPTION_NAME"] = *config.Primary.Name
	}
	for role, selection := range map[string]modelconfig.ClaudeRole{"HAIKU": config.Haiku, "SONNET": config.Sonnet, "OPUS": config.Opus} {
		model := claudeModelValue(config.Primary)
		name := config.Primary.Name
		if selection.Selection != nil {
			model = claudeModelValue(*selection.Selection)
			name = selection.Selection.Name
		}
		result["ANTHROPIC_DEFAULT_"+role+"_MODEL"] = model
		if name != nil {
			result["ANTHROPIC_DEFAULT_"+role+"_MODEL_NAME"] = *name
		}
	}
	for name, value := range config.Extra {
		result[name] = value
	}
	return result
}

func claudeModelValue(selection modelconfig.Model) string {
	if selection.Context != nil && *selection.Context == modelconfig.ClaudeContext1M {
		return selection.Model + "[1m]"
	}
	return selection.Model
}
