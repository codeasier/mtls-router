package agent

import (
	"bytes"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/BurntSushi/toml"
)

func decodeTOML(content []byte) (map[string]any, bool) {
	if len(content) > maxConfigSize || !utf8.Valid(content) {
		return nil, false
	}
	value := map[string]any{}
	_, err := toml.Decode(string(content), &value)
	if err != nil {
		return nil, false
	}
	return value, true
}

func encodeTOML(value map[string]any) ([]byte, error) {
	var output bytes.Buffer
	if err := toml.NewEncoder(&output).Encode(value); err != nil {
		return nil, err
	}
	if output.Len() > maxRenderSize {
		return nil, fmt.Errorf("TOML output exceeds limit")
	}
	return output.Bytes(), nil
}

func tomlString(value any) (string, bool) {
	text, ok := value.(string)
	return text, ok && text != ""
}

func tomlTable(root map[string]any, keys ...string) (map[string]any, bool) {
	current := root
	for _, key := range keys {
		next, ok := current[key].(map[string]any)
		if !ok {
			return nil, false
		}
		current = next
	}
	return current, true
}

func loopbackRouterURL(value any, api bool) bool {
	text, ok := value.(string)
	if !ok {
		return false
	}
	parsedAPI, err := apiURL(strings.TrimSuffix(text, "/v1"))
	if err != nil {
		return false
	}
	if api {
		return parsedAPI == strings.TrimSuffix(text, "/")
	}
	return true
}
