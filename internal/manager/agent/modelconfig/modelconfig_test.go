package modelconfig

import (
	"bytes"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestJCSVectors(t *testing.T) {
	var vectors []struct {
		Name      string   `json:"name"`
		Agents    []Agent  `json:"agents"`
		Catalog   []string `json:"catalog"`
		Input     string   `json:"input"`
		Canonical string   `json:"canonical"`
	}
	data, err := os.ReadFile(filepath.Join("testdata", "jcs-vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &vectors); err != nil {
		t.Fatal(err)
	}
	for _, v := range vectors {
		t.Run(v.Name, func(t *testing.T) {
			c, err := Decode([]byte(v.Input), v.Agents, v.Catalog)
			if err != nil {
				t.Fatal(err)
			}
			got, err := Canonical(c)
			if err != nil {
				t.Fatal(err)
			}
			if string(got) != v.Canonical {
				t.Fatalf("canonical mismatch\ngot  %s\nwant %s", got, v.Canonical)
			}
		})
	}
}

func TestDecodeRejectsInvalidDocuments(t *testing.T) {
	base := `{"version":1,"opencode":{"default_model":"m","models":{"m":{}}}}`
	tests := []struct {
		name, input, rule string
		agents            []Agent
		catalog           []string
	}{
		{"invalid UTF-8", string([]byte{'{', '"', 0xff, '"', '}'}), "utf8", []Agent{OpenCode}, []string{"m"}},
		{"duplicate key", `{"version":1,"version":1,"opencode":{"default_model":"m","models":{"m":{}}}}`, "duplicate_key", []Agent{OpenCode}, []string{"m"}},
		{"trailing JSON", base + ` {}`, "trailing_json", []Agent{OpenCode}, []string{"m"}},
		{"lone surrogate", `{"version":1,"opencode":{"default_model":"m","models":{"m":{"name":"\ud800"}}}}`, "unicode", []Agent{OpenCode}, []string{"m"}},
		{"unknown field", `{"version":1,"opencode":{"default_model":"m","models":{"m":{"bogus":true}}}}`, "unknown_field", []Agent{OpenCode}, []string{"m"}},
		{"section missing", `{"version":1}`, "section_scope", []Agent{OpenCode}, []string{"m"}},
		{"section extra", base[:len(base)-1] + `,"codex":{"model":"m"}}`, "section_scope", []Agent{OpenCode}, []string{"m"}},
		{"duplicate Agent", base, "unique_agents", []Agent{OpenCode, OpenCode}, []string{"m"}},
		{"catalog", base, "catalog", []Agent{OpenCode}, []string{"other"}},
		{"default", `{"version":1,"opencode":{"default_model":"x","models":{"m":{}}}}`, "selected_model", []Agent{OpenCode}, []string{"m", "x"}},
		{"unsafe integer", `{"version":1,"codex":{"model":"m","context_window":9007199254740992}}`, "positive_integer", []Agent{Codex}, []string{"m"}},
		{"limit relationship", `{"version":1,"opencode":{"default_model":"m","models":{"m":{"limit":{"context":10,"input":11,"output":1}}}}}`, "maximum_context", []Agent{OpenCode}, []string{"m"}},
		{"compact relationship", `{"version":1,"codex":{"model":"m","context_window":10,"auto_compact_token_limit":11}}`, "maximum_context", []Agent{Codex}, []string{"m"}},
		{"modality duplicate", `{"version":1,"opencode":{"default_model":"m","models":{"m":{"modalities":{"input":["text","text"]}}}}}`, "unique", []Agent{OpenCode}, []string{"m"}},
		{"interleaved false", `{"version":1,"opencode":{"default_model":"m","models":{"m":{"interleaved":false}}}}`, "enum", []Agent{OpenCode}, []string{"m"}},
		{"reasoning token", `{"version":1,"codex":{"model":"m","reasoning_effort":"HIGH"}}`, "lowercase_token", []Agent{Codex}, []string{"m"}},
		{"codex extra enum", `{"version":1,"codex":{"model":"m","extra":{"model_auto_compact_token_limit_scope":"bad"}}}`, "enum", []Agent{Codex}, []string{"m"}},
		{"Claude role shape", `{"version":1,"claude":{"primary":{"model":"m"},"haiku":{"inherit_primary":false},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`, "unknown_field", []Agent{Claude}, []string{"m"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Decode([]byte(tt.input), tt.agents, tt.catalog)
			var validation *ValidationError
			if !errors.As(err, &validation) {
				t.Fatalf("got %v, want validation error", err)
			}
			if validation.Rule != tt.rule {
				t.Fatalf("got rule %q at %q, want %q", validation.Rule, validation.Path, tt.rule)
			}
		})
	}
}

func TestDecodeStructuralBoundsReferencedModelsPerAgent(t *testing.T) {
	build := func(count int) []byte {
		models := make(map[string]any, count)
		for i := 0; i < count; i++ {
			models[fmt.Sprintf("model-%04d", i)] = map[string]any{}
		}
		data, err := json.Marshal(map[string]any{
			"version": Version,
			"opencode": map[string]any{
				"default_model": "model-0000",
				"models":        models,
			},
		})
		if err != nil {
			t.Fatal(err)
		}
		return data
	}

	if _, err := DecodeStructural(build(MaxReferencedModelsPerAgent)); err != nil {
		t.Fatalf("boundary preset rejected: %v", err)
	}
	_, err := DecodeStructural(build(MaxReferencedModelsPerAgent + 1))
	var validation *ValidationError
	if !errors.As(err, &validation) || validation.Rule != "max_properties" || validation.Path != "/opencode/models" {
		t.Fatalf("overlarge preset error = %#v, want /opencode/models max_properties", err)
	}
}

func TestExtensionConstraints(t *testing.T) {
	wrap := func(value string) string {
		return `{"version":1,"opencode":{"default_model":"m","models":{"m":{"options":` + value + `}}}}`
	}
	tests := []struct{ name, value, rule string }{{"protected normalized", `{"api-Key":"x"}`, "protected_path"}, {"protected nested", `{"safe":{"proxy.url":"x"}}`, "protected_path"}, {"null", `{"safe":null}`, "null"}, {"long key", `{"` + strings.Repeat("k", 129) + `":true}`, "key"}, {"long string", `{"safe":"` + strings.Repeat("x", (16<<10)+1) + `"}`, "string_size"}, {"array", `{"safe":[` + strings.Repeat("true,", 1024) + `true]}`, "array_size"}}
	deep := `true`
	for i := 0; i < 17; i++ {
		deep = `{"safe":` + deep + `}`
	}
	tests = append(tests, struct{ name, value, rule string }{"depth", deep, "depth"})
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Decode([]byte(wrap(tt.value)), []Agent{OpenCode}, []string{"m"})
			var v *ValidationError
			if !errors.As(err, &v) || v.Rule != tt.rule {
				t.Fatalf("got %v, want %s", err, tt.rule)
			}
		})
	}
}

func TestExtraAllowlistAndClaudeValidation(t *testing.T) {
	tests := []string{
		`{"version":1,"opencode":{"default_model":"m","models":{"m":{"extra":{"headers":{}}}}}}`,
		`{"version":1,"opencode":{"default_model":"m","models":{"m":{"extra":{"id":"x"}}}}}`,
	}
	for _, input := range tests {
		if _, err := Decode([]byte(input), []Agent{OpenCode}, []string{"m"}); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
	valid := `{"version":1,"claude":{"primary":{"model":"m","name":"M"},"haiku":{"inherit_primary":true},"sonnet":{"model":"m"},"opus":{"inherit_primary":true},"extra":{"ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION":"long"}}}`
	if _, err := Decode([]byte(valid), []Agent{Claude}, []string{"m"}); err != nil {
		t.Fatal(err)
	}
}

func TestClaudeContextValidationAndOldV1Compatibility(t *testing.T) {
	valid := `{"version":1,"claude":{"primary":{"model":"m","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"model":"m","name":"Standard"},"opus":{"model":"m","context":"1m"}}}`
	config, err := Decode([]byte(valid), []Agent{Claude}, []string{"m"})
	if err != nil {
		t.Fatal(err)
	}
	if config.Claude.Primary.Context == nil || *config.Claude.Primary.Context != ClaudeContext1M || config.Claude.Sonnet.Selection.Context != nil || config.Claude.Opus.Selection.Context == nil {
		t.Fatalf("contexts = %#v", config.Claude)
	}

	oldV1 := `{"version":1,"claude":{"primary":{"model":"m"},"haiku":{"inherit_primary":true},"sonnet":{"model":"m"},"opus":{"inherit_primary":true}}}`
	oldConfig, err := Decode([]byte(oldV1), []Agent{Claude}, []string{"m"})
	if err != nil {
		t.Fatal(err)
	}
	if oldConfig.Claude.Primary.Context != nil || oldConfig.Claude.Sonnet.Selection.Context != nil {
		t.Fatalf("old v1 gained context: %#v", oldConfig.Claude)
	}

	for _, test := range []struct {
		name, value string
	}{
		{name: "empty", value: `""`},
		{name: "unsupported", value: `"standard"`},
		{name: "case sensitive", value: `"1M"`},
		{name: "number", value: `1`},
		{name: "boolean", value: `true`},
	} {
		t.Run(test.name, func(t *testing.T) {
			input := strings.Replace(oldV1, `"model":"m"`, `"model":"m","context":`+test.value, 1)
			_, err := Decode([]byte(input), []Agent{Claude}, []string{"m"})
			var validation *ValidationError
			if !errors.As(err, &validation) || validation.Path != "/claude/primary/context" || validation.Rule != "enum" {
				t.Fatalf("got %v, want context enum error", err)
			}
		})
	}
}

func TestClaudeCanonicalModelRejectsRenderedSuffix(t *testing.T) {
	input := `{"version":1,"claude":{"primary":{"model":"m[1m]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`
	_, err := Decode([]byte(input), []Agent{Claude}, []string{"m[1m]"})
	var validation *ValidationError
	if !errors.As(err, &validation) || validation.Path != "/claude/primary/model" || validation.Rule != "model_id" {
		t.Fatalf("got %v, want model_id error", err)
	}
}

func TestDecodeStructuralUsesCanonicalRulesWithoutCatalog(t *testing.T) {
	input := `{"version":1,"claude":{"primary":{"model":"not-live","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"model":"also-not-live"},"opus":{"inherit_primary":true}},"codex":{"model":"codex-not-live"}}`
	config, err := DecodeStructural([]byte(input))
	if err != nil {
		t.Fatal(err)
	}
	if config.Claude == nil || config.Codex == nil || config.OpenCode != nil {
		t.Fatalf("sections = %#v", config)
	}
	if _, err := Decode([]byte(input), []Agent{Claude, Codex}, nil); err == nil {
		t.Fatal("ordinary decode skipped catalog validation")
	}

	for _, test := range []struct {
		name, input, rule string
	}{
		{name: "no Agent section", input: `{"version":1}`, rule: "agents"},
		{name: "unknown field", input: `{"version":1,"codex":{"model":"m","unknown":true}}`, rule: "unknown_field"},
		{name: "protected extension", input: `{"version":1,"opencode":{"default_model":"m","models":{"m":{"options":{"api_key":"secret"}}}}}`, rule: "protected_path"},
		{name: "Claude suffix", input: `{"version":1,"claude":{"primary":{"model":"m[1m]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`, rule: "model_id"},
	} {
		t.Run(test.name, func(t *testing.T) {
			_, err := DecodeStructural([]byte(test.input))
			var validation *ValidationError
			if !errors.As(err, &validation) || validation.Rule != test.rule {
				t.Fatalf("got %v, want %s", err, test.rule)
			}
		})
	}
}

func TestAllTypedFieldsAndEnums(t *testing.T) {
	input := `{
        "version":1,
        "opencode":{"default_model":"m","models":{"m":{
          "name":"Model M","reasoning":true,"attachment":false,"tool_call":true,"temperature":false,
          "limit":{"context":100,"input":90,"output":10},
          "modalities":{"input":["text","audio","image","video","pdf"],"output":["text"]},
          "interleaved":{"field":"reasoning_details"},
          "options":{"reasoningEffort":"medium","nested":{"safe":true}},
          "extra":{"family":"m","status":"active","experimental":false,"variants":["fast"]}
        }}},
        "codex":{"model":"m","reasoning_effort":"ultra","reasoning_summary":"detailed","verbosity":"high","context_window":100,"auto_compact_token_limit":90,"extra":{"model_auto_compact_token_limit_scope":"body_after_prefix"}}
      }`
	config, err := Decode([]byte(input), []Agent{OpenCode, Codex}, []string{"m"})
	if err != nil {
		t.Fatal(err)
	}
	model := config.OpenCode.Models["m"]
	if model.Name == nil || model.Limit == nil || model.Modalities == nil || model.Interleaved == nil || model.Options == nil || model.Extra == nil {
		t.Fatalf("typed fields omitted: %#v", model)
	}

	for _, field := range []string{"reasoning", "reasoning_content", "reasoning_details"} {
		value := strings.Replace(input, `{"field":"reasoning_details"}`, `{"field":"`+field+`"}`, 1)
		if _, err := Decode([]byte(value), []Agent{OpenCode, Codex}, []string{"m"}); err != nil {
			t.Fatalf("interleaved %s: %v", field, err)
		}
	}
	for _, summary := range []string{"auto", "concise", "detailed", "none"} {
		value := strings.Replace(input, `"reasoning_summary":"detailed"`, `"reasoning_summary":"`+summary+`"`, 1)
		if _, err := Decode([]byte(value), []Agent{OpenCode, Codex}, []string{"m"}); err != nil {
			t.Fatalf("summary %s: %v", summary, err)
		}
	}
	for _, verbosity := range []string{"low", "medium", "high"} {
		value := strings.Replace(input, `"verbosity":"high"`, `"verbosity":"`+verbosity+`"`, 1)
		if _, err := Decode([]byte(value), []Agent{OpenCode, Codex}, []string{"m"}); err != nil {
			t.Fatalf("verbosity %s: %v", verbosity, err)
		}
	}
	for _, scope := range []string{"total", "body_after_prefix"} {
		value := strings.Replace(input, `"model_auto_compact_token_limit_scope":"body_after_prefix"`, `"model_auto_compact_token_limit_scope":"`+scope+`"`, 1)
		if _, err := Decode([]byte(value), []Agent{OpenCode, Codex}, []string{"m"}); err != nil {
			t.Fatalf("scope %s: %v", scope, err)
		}
	}
	trueInterleaved := strings.Replace(input, `{"field":"reasoning_details"}`, `true`, 1)
	if _, err := Decode([]byte(trueInterleaved), []Agent{OpenCode, Codex}, []string{"m"}); err != nil {
		t.Fatal(err)
	}
}

func TestDeepMergeIsRecursiveDeterministicAndNonMutating(t *testing.T) {
	base := map[string]any{"a": map[string]any{"x": 1, "y": []any{1}}, "b": true}
	overlay := map[string]any{"a": map[string]any{"z": 2, "y": []any{3}}, "b": "new"}
	got := DeepMerge(base, overlay)
	want := map[string]any{"a": map[string]any{"x": 1, "y": []any{3}, "z": 2}, "b": "new"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v", got)
	}
	got["a"].(map[string]any)["x"] = 9
	if base["a"].(map[string]any)["x"] != 1 {
		t.Fatal("base mutated")
	}
}

func TestDecodedExtensionsDeepMergeRecursively(t *testing.T) {
	input := `{"version":1,"opencode":{"default_model":"m","models":{"m":{"options":{"nested":{"left":1}}}}}}`
	config, err := Decode([]byte(input), []Agent{OpenCode}, []string{"m"})
	if err != nil {
		t.Fatal(err)
	}
	merged := DeepMerge(config.OpenCode.Models["m"].Options, map[string]any{"nested": map[string]any{"right": 2}})
	nested, ok := merged["nested"].(map[string]any)
	if !ok || len(nested) != 2 {
		t.Fatalf("nested merge failed: %#v", merged)
	}
}

func TestEmptyCodexExtra(t *testing.T) {
	if _, err := Decode([]byte(`{"version":1,"codex":{"model":"m","extra":{}}}`), []Agent{Codex}, []string{"m"}); err != nil {
		t.Fatal(err)
	}
}

func TestCanonicalNumberAndStringEncoding(t *testing.T) {
	v := object{"negzero": json.Number("-0"), "small": json.Number("0.000001"), "large": json.Number("1e21"), "control": "\x00\n"}
	got, err := marshalJCS(v)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"control":"\u0000\n","large":1e+21,"negzero":0,"small":0.000001}`
	if string(got) != want {
		t.Fatalf("got %s want %s", got, want)
	}
}

func TestSchemaIsCheckedIn(t *testing.T) {
	generated, err := GenerateSchema()
	if err != nil {
		t.Fatal(err)
	}
	checked, err := os.ReadFile(filepath.Join("schema", "model-config-v1.schema.json"))
	if err != nil {
		t.Fatal(err)
	}
	var generatedValue, checkedValue any
	if err := json.Unmarshal(generated, &generatedValue); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(checked, &checkedValue); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(generatedValue, checkedValue) {
		t.Fatal("schema is stale; regenerate model-config-v1.schema.json")
	}
}

func TestCatalogTokensBindEveryContextAndRejectTampering(t *testing.T) {
	key := make([]byte, 32)
	if _, err := rand.Read(key); err != nil {
		t.Fatal(err)
	}
	signer, err := NewTokenSigner(key, "generation-1234567890")
	if err != nil {
		t.Fatal(err)
	}
	claims := CatalogClaims{Models: []string{"z", "a", "a"}, Agents: []Agent{Codex, Claude}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "deployment-a", ProtocolVersion: "2"}
	token, err := signer.SignCatalog(claims)
	if err != nil {
		t.Fatal(err)
	}
	verified, err := signer.VerifyCatalog(token)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(verified.Models, []string{"a", "z"}) || !reflect.DeepEqual(verified.Agents, []Agent{Claude, Codex}) || verified.KeyGeneration != "generation-1234567890" {
		t.Fatalf("normalized claims = %#v", verified)
	}
	parts := strings.Split(token, ".")
	payload, _ := base64.RawURLEncoding.DecodeString(parts[0])
	payload = []byte(strings.Replace(string(payload), "deployment-a", "deployment-b", 1))
	forged := base64.RawURLEncoding.EncodeToString(payload) + "." + parts[1]
	if _, err := signer.VerifyCatalog(forged); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("tampered public data accepted: %v", err)
	}
	other, _ := NewTokenSigner(make([]byte, 32), "generation-1234567890")
	if _, err := other.VerifyCatalog(token); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("public-data forgery accepted: %v", err)
	}
}

func TestCatalogTokenVersionScopeGenerationAndLimits(t *testing.T) {
	key := make([]byte, 32)
	signer, _ := NewTokenSigner(key, "generation-a")
	base := CatalogClaims{Models: []string{"m"}, Agents: []Agent{OpenCode}, Owner: "desktop", RouterBaseURL: "http://[::1]:19099", DeploymentID: "deployment", ProtocolVersion: "2"}
	for _, mutate := range []func(*CatalogClaims){
		func(c *CatalogClaims) { c.Version = 2 },
		func(c *CatalogClaims) { c.Owner = "other" },
		func(c *CatalogClaims) { c.Agents = nil },
		func(c *CatalogClaims) { c.DeploymentID = "" },
		func(c *CatalogClaims) { c.ProtocolVersion = "" },
		func(c *CatalogClaims) { c.RouterBaseURL = "" },
	} {
		claims := base
		mutate(&claims)
		if _, err := signer.SignCatalog(claims); !errors.Is(err, ErrTokenInvalid) {
			t.Fatalf("invalid scope accepted: %#v err=%v", claims, err)
		}
	}
	token, err := signer.SignCatalog(base)
	if err != nil {
		t.Fatal(err)
	}
	rotated, _ := NewTokenSigner(key, "generation-b")
	if _, err := rotated.VerifyCatalog(token); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("wrong generation accepted: %v", err)
	}
	if _, err := signer.VerifyCatalog(strings.Repeat("a", MaxCatalogTokenSize+1)); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("oversized token accepted: %v", err)
	}
}

func TestCatalogTokenRejectsEveryCrossProcessContextMismatch(t *testing.T) {
	key := bytes.Repeat([]byte{0x42}, 32)
	claims := CatalogClaims{Models: []string{"m"}, Agents: []Agent{Claude, Codex}, Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "deployment", ProtocolVersion: "2"}
	signer, _ := NewTokenSigner(key, "generation-a")
	token, err := signer.SignCatalog(claims)
	if err != nil {
		t.Fatal(err)
	}
	secondProcess, _ := NewTokenSigner(append([]byte(nil), key...), "generation-a")
	if _, err := secondProcess.VerifyCatalog(token); err != nil {
		t.Fatalf("same trust context in another process failed: %v", err)
	}

	mutations := []struct {
		name   string
		mutate func(*CatalogClaims)
	}{
		{name: "version", mutate: func(c *CatalogClaims) { c.Version = 2 }},
		{name: "canonicalization", mutate: func(c *CatalogClaims) { c.Canonicalization = "other" }},
		{name: "models", mutate: func(c *CatalogClaims) { c.Models = []string{"other"} }},
		{name: "agents", mutate: func(c *CatalogClaims) { c.Agents = []Agent{Claude} }},
		{name: "owner", mutate: func(c *CatalogClaims) { c.Owner = "desktop" }},
		{name: "address", mutate: func(c *CatalogClaims) { c.RouterBaseURL = "http://127.0.0.1:19100" }},
		{name: "deployment", mutate: func(c *CatalogClaims) { c.DeploymentID = "other" }},
		{name: "protocol", mutate: func(c *CatalogClaims) { c.ProtocolVersion = "1" }},
		{name: "generation", mutate: func(c *CatalogClaims) { c.KeyGeneration = "generation-b" }},
	}
	for _, test := range mutations {
		t.Run(test.name, func(t *testing.T) {
			forged := tamperCatalogForTest(t, token, test.mutate)
			if _, err := secondProcess.VerifyCatalog(forged); !errors.Is(err, ErrTokenInvalid) {
				t.Fatalf("context mismatch accepted: %v", err)
			}
		})
	}
	if _, err := secondProcess.VerifyRevision(token); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("catalog token accepted in revision context: %v", err)
	}
}

func TestRevisionTokensBindConfigCatalogRouterDriftAndRevisions(t *testing.T) {
	key := make([]byte, 32)
	signer, _ := NewTokenSigner(key, "generation-r")
	claims := RevisionClaims{
		Agents: []Agent{OpenCode}, CanonicalConfig: json.RawMessage(`{"version":1}`), CatalogIdentity: "catalog-mac",
		SidecarRevision: "sidecar-mac", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "deployment",
		ProtocolVersion: "2", ManagedDrift: true, DriftedAgents: []Agent{OpenCode},
		Bindings: []RevisionBinding{{Context: "agent-file", Identity: "/config", Revision: "revision-mac"}},
	}
	token, err := signer.SignRevision(claims)
	if err != nil {
		t.Fatal(err)
	}
	verified, err := signer.VerifyRevision(token)
	if err != nil || !verified.ManagedDrift || string(verified.CanonicalConfig) != `{"version":1}` {
		t.Fatalf("revision claims = %#v err=%v", verified, err)
	}
	mac1, _ := signer.RevisionMAC("agent-file", "/config", []byte("secret"))
	mac2, _ := signer.RevisionMAC("backup-verification", "/config", []byte("secret"))
	if mac1 == mac2 || strings.Contains(mac1, hexDigest([]byte("secret"))) {
		t.Fatal("revision MAC contexts were not separated")
	}
	if _, err := signer.VerifyRevision(token + "x"); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("tampered revision token accepted: %v", err)
	}
}

func TestRevisionTokenRejectsEveryCrossProcessContextMismatch(t *testing.T) {
	key := bytes.Repeat([]byte{0x24}, 32)
	signer, _ := NewTokenSigner(key, "generation-r")
	claims := RevisionClaims{
		Agents: []Agent{Claude, OpenCode}, CanonicalConfig: json.RawMessage(`{"version":1}`), CatalogIdentity: "catalog-mac",
		SidecarRevision: "sidecar-mac", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "deployment",
		ProtocolVersion: "2", ManagedDrift: true, RequiresCodexAuthApproval: true, DriftedAgents: []Agent{Claude},
		Bindings: []RevisionBinding{{Context: "agent-file", Identity: "/config", Revision: "revision-mac"}},
	}
	token, err := signer.SignRevision(claims)
	if err != nil {
		t.Fatal(err)
	}
	secondProcess, _ := NewTokenSigner(append([]byte(nil), key...), "generation-r")
	if _, err := secondProcess.VerifyRevision(token); err != nil {
		t.Fatalf("same trust context in another process failed: %v", err)
	}
	mutations := []struct {
		name   string
		mutate func(*RevisionClaims)
	}{
		{name: "version", mutate: func(c *RevisionClaims) { c.Version = 2 }},
		{name: "agents", mutate: func(c *RevisionClaims) { c.Agents = []Agent{Claude} }},
		{name: "config", mutate: func(c *RevisionClaims) { c.CanonicalConfig = json.RawMessage(`{"version":2}`) }},
		{name: "catalog", mutate: func(c *RevisionClaims) { c.CatalogIdentity = "other" }},
		{name: "sidecar", mutate: func(c *RevisionClaims) { c.SidecarRevision = "other" }},
		{name: "address", mutate: func(c *RevisionClaims) { c.RouterBaseURL = "http://127.0.0.1:19100" }},
		{name: "deployment", mutate: func(c *RevisionClaims) { c.DeploymentID = "other" }},
		{name: "protocol", mutate: func(c *RevisionClaims) { c.ProtocolVersion = "1" }},
		{name: "canonicalization", mutate: func(c *RevisionClaims) { c.Canonicalization = "other" }},
		{name: "generation", mutate: func(c *RevisionClaims) { c.KeyGeneration = "other" }},
		{name: "managed drift", mutate: func(c *RevisionClaims) { c.ManagedDrift = false }},
		{name: "auth approval", mutate: func(c *RevisionClaims) { c.RequiresCodexAuthApproval = false }},
		{name: "drifted agents", mutate: func(c *RevisionClaims) { c.DriftedAgents = []Agent{OpenCode} }},
		{name: "binding context", mutate: func(c *RevisionClaims) { c.Bindings[0].Context = "backup-verification" }},
		{name: "binding identity", mutate: func(c *RevisionClaims) { c.Bindings[0].Identity = "/other" }},
		{name: "binding revision", mutate: func(c *RevisionClaims) { c.Bindings[0].Revision = "other" }},
	}
	for _, test := range mutations {
		t.Run(test.name, func(t *testing.T) {
			forged := tamperRevisionForTest(t, token, test.mutate)
			if _, err := secondProcess.VerifyRevision(forged); !errors.Is(err, ErrTokenInvalid) {
				t.Fatalf("context mismatch accepted: %v", err)
			}
		})
	}
	if _, err := secondProcess.VerifyCatalog(token); !errors.Is(err, ErrTokenInvalid) {
		t.Fatalf("revision token accepted in catalog context: %v", err)
	}
}

func tamperCatalogForTest(t *testing.T, token string, mutate func(*CatalogClaims)) string {
	t.Helper()
	payload, _ := base64.RawURLEncoding.DecodeString(strings.SplitN(token, ".", 2)[0])
	var claims CatalogClaims
	if err := json.Unmarshal(payload, &claims); err != nil {
		t.Fatal(err)
	}
	mutate(&claims)
	return tamperedTokenForTest(t, token, claims)
}

func tamperRevisionForTest(t *testing.T, token string, mutate func(*RevisionClaims)) string {
	t.Helper()
	payload, _ := base64.RawURLEncoding.DecodeString(strings.SplitN(token, ".", 2)[0])
	var claims RevisionClaims
	if err := json.Unmarshal(payload, &claims); err != nil {
		t.Fatal(err)
	}
	mutate(&claims)
	return tamperedTokenForTest(t, token, claims)
}

func tamperedTokenForTest(t *testing.T, token string, claims any) string {
	t.Helper()
	payload, err := marshalJCS(claims)
	if err != nil {
		t.Fatal(err)
	}
	signature := strings.SplitN(token, ".", 2)[1]
	return base64.RawURLEncoding.EncodeToString(payload) + "." + signature
}

func hexDigest(value []byte) string {
	digest := sha256.Sum256(value)
	return fmt.Sprintf("%x", digest[:])
}
