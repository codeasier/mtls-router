package preset

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

func TestLoadAbsentAndValid(t *testing.T) {
	withEncoded(t, "")
	config, err := Load()
	if err != nil || config != nil {
		t.Fatalf("Load() = %#v, %v", config, err)
	}

	input := `{"version":1,"claude":{"primary":{"model":"model-a","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}},"codex":{"model":"model-b"}}`
	withEncoded(t, base64.StdEncoding.EncodeToString([]byte(input)))
	config, err = Load()
	if err != nil {
		t.Fatal(err)
	}
	if config.Claude == nil || config.Codex == nil || config.OpenCode != nil {
		t.Fatalf("config = %#v", config)
	}
}

func TestLoadRejectsMalformedInputsWithoutLeakage(t *testing.T) {
	decodedCanary := "decoded-preset-secret-canary"
	tests := []struct {
		name    string
		encoded string
	}{
		{name: "invalid Base64", encoded: "encoded-preset-secret-canary%%%"},
		{name: "noncanonical Base64 bits", encoded: "Zh=="},
		{name: "Base64 whitespace", encoded: encode(`{"version":1,"codex":{"model":"m"}}`) + "\n"},
		{name: "malformed JSON", encoded: encode(`{"version":1,"codex":`)},
		{name: "invalid UTF-8", encoded: base64.StdEncoding.EncodeToString([]byte{0xff, 0xfe})},
		{name: "duplicate key", encoded: encode(`{"version":1,"codex":{"model":"m","model":"` + decodedCanary + `"}}`)},
		{name: "version only", encoded: encode(`{"version":1}`)},
		{name: "unknown field", encoded: encode(`{"version":1,"codex":{"model":"m","` + decodedCanary + `":true}}`)},
		{name: "protected key", encoded: encode(`{"version":1,"opencode":{"default_model":"m","models":{"m":{"options":{"api_key":"` + decodedCanary + `"}}}}}`)},
		{name: "trailing data", encoded: encode(`{"version":1,"codex":{"model":"m"}} ` + decodedCanary)},
		{name: "oversized", encoded: base64.StdEncoding.EncodeToString([]byte(strings.Repeat("x", modelconfig.MaxConfigSize+1)))},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			withEncoded(t, test.encoded)
			_, err := Load()
			if err == nil || err.Error() != "invalid embedded Agent model preset" {
				t.Fatalf("error = %v", err)
			}
			if strings.Contains(err.Error(), test.encoded) || strings.Contains(err.Error(), decodedCanary) {
				t.Fatalf("error leaked preset input: %q", err)
			}
		})
	}
}

func TestLoadRejectsStructurallyOverlargeAgentModelSet(t *testing.T) {
	models := make(map[string]any, modelconfig.MaxReferencedModelsPerAgent+1)
	for i := 0; i <= modelconfig.MaxReferencedModelsPerAgent; i++ {
		models[fmt.Sprintf("model-%04d", i)] = map[string]any{}
	}
	data, err := json.Marshal(map[string]any{
		"version": modelconfig.Version,
		"opencode": map[string]any{
			"default_model": "model-0000",
			"models":        models,
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	withEncoded(t, base64.StdEncoding.EncodeToString(data))
	if _, err := Load(); err == nil || err.Error() != "invalid embedded Agent model preset" {
		t.Fatalf("Load() error = %v", err)
	}
}

func encode(value string) string {
	return base64.StdEncoding.EncodeToString([]byte(value))
}

func withEncoded(t *testing.T, value string) {
	t.Helper()
	previous := Encoded
	Encoded = value
	t.Cleanup(func() { Encoded = previous })
}
