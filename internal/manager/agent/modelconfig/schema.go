package modelconfig

import "encoding/json"

// GenerateSchema produces the versioned interchange schema. Catalog
// membership, selected-Agent section scope, protected recursive keys, and
// cross-field integer relationships remain authoritative manager validation.
func GenerateSchema() ([]byte, error) {
	integer := map[string]any{"type": "integer", "minimum": 1, "maximum": MaxSafeInteger}
	model := map[string]any{
		"type": "object", "additionalProperties": false, "required": []string{"model"},
		"properties": map[string]any{"model": map[string]any{"type": "string", "minLength": 1, "maxLength": 256, "not": map[string]any{"pattern": `\[1m\]$`}}, "name": map[string]any{"type": "string", "minLength": 1}, "context": map[string]any{"enum": []string{"1m"}}},
	}
	role := map[string]any{"oneOf": []any{
		map[string]any{"type": "object", "additionalProperties": false, "required": []string{"inherit_primary"}, "properties": map[string]any{"inherit_primary": map[string]any{"const": true}}},
		map[string]any{"$ref": "#/$defs/model"},
	}}
	limit := map[string]any{"type": "object", "additionalProperties": false, "required": []string{"context", "output"}, "properties": map[string]any{"context": integer, "input": integer, "output": integer}}
	modalityArray := map[string]any{"type": "array", "uniqueItems": true, "maxItems": 1024, "items": map[string]any{"enum": []string{"text", "audio", "image", "video", "pdf"}}}
	modalities := map[string]any{"type": "object", "additionalProperties": false, "properties": map[string]any{"input": modalityArray, "output": modalityArray}}
	interleaved := map[string]any{"oneOf": []any{map[string]any{"const": true}, map[string]any{"type": "object", "additionalProperties": false, "required": []string{"field"}, "properties": map[string]any{"field": map[string]any{"enum": []string{"reasoning", "reasoning_content", "reasoning_details"}}}}}}
	extension := map[string]any{"type": "object", "description": "Recursively bounded and protected-key validated by the manager"}
	variantOptions := map[string]any{"type": "object", "description": "Recursively bounded and protected-key validated by the manager"}
	variants := map[string]any{"type": "object", "propertyNames": map[string]any{"description": "Variant names exclude control characters. maxLength counts Unicode code points; authoritative manager validation enforces the 128 UTF-8-byte limit.", "minLength": 1, "maxLength": 128, "pattern": `^[^\u0000-\u001f\u007f-\u009f]+$`}, "additionalProperties": variantOptions}
	openModel := map[string]any{"type": "object", "additionalProperties": false, "properties": map[string]any{
		"name": map[string]any{"type": "string", "minLength": 1}, "reasoning": map[string]any{"type": "boolean"}, "attachment": map[string]any{"type": "boolean"}, "tool_call": map[string]any{"type": "boolean"}, "temperature": map[string]any{"type": "boolean"}, "limit": map[string]any{"$ref": "#/$defs/limit"}, "modalities": map[string]any{"$ref": "#/$defs/modalities"}, "interleaved": map[string]any{"$ref": "#/$defs/interleaved"}, "options": extension, "variants": variants, "extra": extension,
	}}
	claudeExtra := map[string]any{"type": "object", "additionalProperties": false, "properties": map[string]any{}}
	for _, key := range []string{"ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION", "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION", "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION", "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION"} {
		claudeExtra["properties"].(map[string]any)[key] = map[string]any{"type": "string", "maxLength": 16384}
	}
	root := map[string]any{
		"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://github.com/codeasier/mtls-router/schema/model-config-v1.schema.json", "title": "mtls-router canonical Agent model config v1", "type": "object", "additionalProperties": false, "required": []string{"version"},
		"properties": map[string]any{
			"version":  map[string]any{"const": Version},
			"claude":   map[string]any{"type": "object", "additionalProperties": false, "required": []string{"primary", "haiku", "sonnet", "opus"}, "properties": map[string]any{"primary": map[string]any{"$ref": "#/$defs/model"}, "fable": map[string]any{"$ref": "#/$defs/role"}, "haiku": map[string]any{"$ref": "#/$defs/role"}, "sonnet": map[string]any{"$ref": "#/$defs/role"}, "opus": map[string]any{"$ref": "#/$defs/role"}, "context_window": integer, "max_output_tokens": integer, "extra": claudeExtra}},
			"opencode": map[string]any{"type": "object", "additionalProperties": false, "required": []string{"default_model", "models"}, "properties": map[string]any{"default_model": map[string]any{"type": "string"}, "models": map[string]any{"type": "object", "minProperties": 1, "maxProperties": MaxReferencedModelsPerAgent, "additionalProperties": map[string]any{"$ref": "#/$defs/openCodeModel"}}}},
			"codex":    map[string]any{"type": "object", "additionalProperties": false, "required": []string{"model"}, "properties": map[string]any{"model": map[string]any{"type": "string"}, "reasoning_effort": map[string]any{"type": "string", "minLength": 1, "maxLength": 64, "pattern": "^[a-z0-9_-]+$"}, "reasoning_summary": map[string]any{"enum": []string{"auto", "concise", "detailed", "none"}}, "verbosity": map[string]any{"enum": []string{"low", "medium", "high"}}, "context_window": integer, "auto_compact_token_limit": integer, "extra": map[string]any{"type": "object", "additionalProperties": false, "properties": map[string]any{"model_auto_compact_token_limit_scope": map[string]any{"enum": []string{"total", "body_after_prefix"}}}}}},
		},
		"$defs": map[string]any{"model": model, "role": role, "limit": limit, "modalities": modalities, "interleaved": interleaved, "openCodeModel": openModel},
	}
	data, err := json.MarshalIndent(root, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(data, '\n'), nil
}
