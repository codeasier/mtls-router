package agent

import (
	"bytes"
	"encoding/json"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

func renderOpenCodeFragment(config *modelconfig.OpenCodeConfig, apiBaseURL, key string) ([]byte, error) {
	provider := openCodeProvider(config, apiBaseURL, key)
	return marshalIndentedJSON(map[string]any{
		"model":    "mtls-router/" + config.DefaultModel,
		"provider": map[string]any{"mtls-router": provider},
	})
}

func mergeOpenCode(root map[string]json.RawMessage, config *modelconfig.OpenCodeConfig, apiBaseURL, key string, ownRootModel bool) ([]byte, error) {
	result := cloneRawObject(root)
	providers := make(map[string]json.RawMessage)
	if raw, ok := result["provider"]; ok && !bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		var valid bool
		providers, valid = decodeObject(raw)
		if !valid {
			return nil, operationError(CodeConfigInvalid, "opencode provider field is not an object")
		}
	}
	providerJSON, err := json.Marshal(openCodeProvider(config, apiBaseURL, key))
	if err != nil {
		return nil, err
	}
	providers["mtls-router"] = providerJSON
	providersJSON, err := json.Marshal(providers)
	if err != nil {
		return nil, err
	}
	result["provider"] = providersJSON
	if _, exists := result["model"]; !exists || ownRootModel {
		result["model"], _ = json.Marshal("mtls-router/" + config.DefaultModel)
	}
	return marshalObject(result)
}

func openCodeProvider(config *modelconfig.OpenCodeConfig, apiBaseURL, key string) map[string]any {
	models := make(map[string]any, len(config.Models))
	for id, config := range config.Models {
		entry := map[string]any{"name": id}
		if config.Name != nil {
			entry["name"] = *config.Name
		}
		if config.Reasoning != nil {
			entry["reasoning"] = *config.Reasoning
		}
		if config.Attachment != nil {
			entry["attachment"] = *config.Attachment
		}
		if config.ToolCall != nil {
			entry["tool_call"] = *config.ToolCall
		}
		if config.Temperature != nil {
			entry["temperature"] = *config.Temperature
		}
		if config.Limit != nil {
			entry["limit"] = config.Limit
		}
		if config.Modalities != nil {
			entry["modalities"] = config.Modalities
		}
		if config.Interleaved != nil {
			entry["interleaved"] = config.Interleaved
		}
		if config.Options != nil {
			entry["options"] = config.Options
		}
		models[id] = modelconfig.DeepMerge(config.Extra, entry)
	}
	return map[string]any{
		"npm": "@ai-sdk/openai-compatible", "name": "mtls-router",
		"options": map[string]any{"baseURL": apiBaseURL, "apiKey": key},
		"models":  models,
	}
}
