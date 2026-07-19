# Agent Presets and Claude Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-selection Claude Code display names and 1M context mode, plus a build-injected Agent preset that is validated against authenticated discovery and used as an editable per-Agent default.

**Architecture:** Keep `modelconfig` as the only authoritative structural validator, represent Claude 1M mode as canonical `context: "1m"`, and append `[1m]` only in the Claude renderer. Decode the build-time Base64 preset during manager startup, pass the typed key-free preset into the Agent service, validate each requested section against the live catalog, and return it separately from existing configuration so every client can apply `existing > preset > empty`.

**Tech Stack:** Go 1.26, JSON/JCS and JSON Schema, private newline-delimited JSON protocol v2, Bash/jq, Windows PowerShell 5.1, React 19/TypeScript/Vitest, Rust/Tauri/Serde, GitHub Actions.

---

## File Map

### Go Canonical Configuration

- Modify `internal/manager/agent/modelconfig/types.go`: add Claude context to `Model`.
- Modify `internal/manager/agent/modelconfig/decode.go`: add the preset structural-decode entry point while retaining strict decoding.
- Modify `internal/manager/agent/modelconfig/validate.go`: share validation with and without catalog membership; reject canonical Claude `[1m]` suffixes.
- Modify `internal/manager/agent/modelconfig/schema.go`: add the `context` enum.
- Regenerate `internal/manager/agent/modelconfig/schema/model-config-v1.schema.json`.
- Modify `internal/manager/agent/modelconfig/modelconfig_test.go`: context, structural preset validation, schema, and canonical tests.

### Claude Rendering and Existing Projection

- Modify `internal/manager/agent/render_claude.go`: render one terminal `[1m]` for selected context.
- Modify `internal/manager/agent/models.go`: parse existing suffixes and compare model/name/context before inheritance.
- Modify `internal/manager/agent/render_test.go`: standard/1M rendering and inheritance tests.
- Modify `internal/manager/agent/models_test.go`: projection and unavailable-base-ID tests.

### Manager Preset and Protocol

- Create `internal/manager/preset/preset.go`: link variable, Base64 decode, size bound, and startup structural validation.
- Create `internal/manager/preset/preset_test.go`: malformed build input and no-content-leakage tests.
- Modify `internal/manager/agent/service.go`: accept an immutable typed preset.
- Modify `internal/manager/agent/models.go`: return live-catalog-valid preset sections and bounded unavailability metadata.
- Modify `internal/manager/agent/models_test.go`: crop and per-Agent validity tests.
- Modify `internal/manager/protocol/types.go`: add stable preset result types.
- Modify `internal/manager/protocol/server_test.go`: exact result-shape contract.
- Modify `internal/manager/app/app.go`: fail startup on a broken injected preset and map preset discovery output.
- Modify `internal/manager/app/app_test.go`: key-free complete `agent.models` result.
- Modify `cmd/mtls-router-manager/main_test.go`: startup and subprocess protocol coverage.

### Desktop Bridge and UI

- Modify `desktop/src-tauri/src/model_config.rs`: Rust `context` type and validation.
- Modify `desktop/src-tauri/src/commands.rs`: strict manager preset response and forwarding.
- Modify Rust tests in those files.
- Modify `desktop/src/ipc.ts`: TypeScript context and preset types.
- Modify `desktop/src/AgentPage.tsx`: per-Agent initialization, Claude metadata controls, source notices.
- Modify `desktop/src/AgentPage.test.tsx`, `desktop/src/ipc.test.ts`, and `desktop/src/modelConfigVectors.test.ts`.
- Modify `desktop/src/locales/en.ts`, `desktop/src/locales/zh-CN.ts`, `desktop/src/i18n.test.ts`, and `desktop/src/styles.css`.

### Setup Clients

- Modify `setup.sh`: prompt defaults, per-Agent source merge, and Claude context.
- Modify `tests/setup_agent_v2_flow_test.sh`: preset defaults, override, mixed source, and key secrecy.
- Modify `setup.ps1` while preserving its UTF-8 BOM: equivalent behavior.
- Modify `tests/setup_powershell_v2_flow_test.sh` and `tests/setup_powershell_wizard_test.sh`.

### Build, Release, and Documentation

- Modify `scripts/build.sh` and `desktop/scripts/build-sidecars.sh`: optional linker injection.
- Modify `.github/workflows/release.yml`: inject the same preset into standalone and desktop managers.
- Modify `tests/desktop_workflow_test.sh` and `tests/setup_release_packaging_test.sh`: workflow contract.
- Modify `internal/manager/agent/testdata/compatibility.json`: current verified Claude evidence.
- Modify English and Chinese README, Agent, Desktop, Build, Troubleshooting, and changelog documents.

## Task 1: Extend Canonical Claude Selection Validation

**Files:**
- Modify: `internal/manager/agent/modelconfig/types.go:28-31`
- Modify: `internal/manager/agent/modelconfig/decode.go:15-36`
- Modify: `internal/manager/agent/modelconfig/validate.go:16-143`
- Modify: `internal/manager/agent/modelconfig/schema.go:8-45`
- Modify: `internal/manager/agent/modelconfig/schema/model-config-v1.schema.json`
- Test: `internal/manager/agent/modelconfig/modelconfig_test.go`

- [ ] **Step 1: Write failing context and structural-validation tests**

Add table cases proving that omitted context and exact `"1m"` pass, while
`"200k"`, booleans, and model IDs ending in `[1m]` fail. Add a startup-style
test proving a valid nonempty multi-Agent document can be decoded without a
catalog while an empty version-only document cannot.

```go
func TestClaudeContextAndBaseCatalogIdentity(t *testing.T) {
	valid := []byte(`{"version":1,"claude":{"primary":{"model":"model-a","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	config, err := Decode(valid, []Agent{Claude}, []string{"model-a"})
	if err != nil || config.Claude.Primary.Context == nil || *config.Claude.Primary.Context != "1m" {
		t.Fatalf("Decode() = %#v, %v", config, err)
	}

	for _, raw := range []string{
		`{"version":1,"claude":{"primary":{"model":"model-a","context":"200k"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`,
		`{"version":1,"claude":{"primary":{"model":"model-a[1m]"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`,
	} {
		if _, err := Decode([]byte(raw), []Agent{Claude}, []string{"model-a", "model-a[1m]"}); err == nil {
			t.Fatalf("Decode(%s) succeeded", raw)
		}
	}
}

func TestDecodeStructureRequiresNonemptyAgentConfig(t *testing.T) {
	raw := []byte(`{"version":1,"claude":{"primary":{"model":"model-a"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}},"codex":{"model":"model-b"}}`)
	config, agents, err := DecodeStructure(raw)
	if err != nil || config.Claude == nil || config.Codex == nil || len(agents) != 2 {
		t.Fatalf("DecodeStructure() = %#v, %v, %v", config, agents, err)
	}
	if _, _, err := DecodeStructure([]byte(`{"version":1}`)); err == nil {
		t.Fatal("empty preset succeeded")
	}
}
```

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```bash
go test ./internal/manager/agent/modelconfig -run 'ClaudeContext|DecodeStructure|SchemaIsCheckedIn'
```

Expected: compilation fails because `Model.Context` and `DecodeStructure` do not
exist, or assertions fail because context is rejected.

- [ ] **Step 3: Add the typed field and one shared validation path**

Add the optional field:

```go
type Model struct {
	Model   string  `json:"model"`
	Name    *string `json:"name,omitempty"`
	Context *string `json:"context,omitempty"`
}
```

Keep `Decode` as the catalog-enforcing public API. Add `DecodeStructure`, which
uses the same strict byte decoder, derives present Agent sections in canonical
Claude/opencode/Codex order, rejects a version-only document, and calls the same
parser with membership disabled:

```go
type validationOptions struct {
	selected       []Agent
	catalog        []string
	requireCatalog bool
}

func Decode(data []byte, selected []Agent, catalog []string) (*Config, error) {
	return decode(data, validationOptions{selected: selected, catalog: catalog, requireCatalog: true})
}

func DecodeStructure(data []byte) (*Config, []Agent, error) {
	root, err := strictObject(data)
	if err != nil {
		return nil, nil, err
	}
	selected := presentAgents(root)
	if len(selected) == 0 {
		return nil, nil, invalid("", "agent_scope")
	}
	config, err := parseConfig(root, validationOptions{selected: selected})
	return config, selected, err
}
```

Extract the existing byte checks into `strictObject` rather than decoding JSON a
second time. Thread `requireCatalog` into `parseModel` and the opencode/Codex
membership checks. Structural mode still enforces `validID`; membership mode
also requires `cat[id]`.

For Claude selections, require exact keys `model`, `name`, and `context`, reject
terminal `[1m]` independently of catalog contents, and accept only `"1m"`:

```go
if strings.HasSuffix(id, "[1m]") {
	return nil, invalid(path+"/model", "base_model")
}
if value, exists := o["context"]; exists {
	context, ok := value.(string)
	if !ok || context != "1m" {
		return nil, invalid(path+"/context", "enum")
	}
	m.Context = &context
}
```

- [ ] **Step 4: Update and regenerate the schema**

Add this property to the shared Claude selection definition in
`GenerateSchema`:

```go
"context": map[string]any{"enum": []string{"1m"}},
```

Run the existing schema parity test once to obtain the generated mismatch, then
update `model-config-v1.schema.json` to exactly match `GenerateSchema`. Do not
hand-add unrelated schema fields.

- [ ] **Step 5: Run modelconfig tests and formatting**

Run:

```bash
gofmt -w internal/manager/agent/modelconfig/types.go internal/manager/agent/modelconfig/decode.go internal/manager/agent/modelconfig/validate.go internal/manager/agent/modelconfig/schema.go internal/manager/agent/modelconfig/modelconfig_test.go
go test ./internal/manager/agent/modelconfig
```

Expected: PASS, including checked-in schema parity and existing JCS/token tests.

- [ ] **Step 6: Commit canonical configuration support**

```bash
git add internal/manager/agent/modelconfig
git commit -m "feat: add Claude context selection"
```

## Task 2: Render and Recover Claude 1M Selections

**Files:**
- Modify: `internal/manager/agent/render_claude.go:55-82`
- Modify: `internal/manager/agent/models.go:198-239`
- Test: `internal/manager/agent/render_test.go`
- Test: `internal/manager/agent/models_test.go`

- [ ] **Step 1: Write failing renderer and projection tests**

Add tests with a 1M primary, inherited Sonnet, standard explicit Haiku, and 1M
explicit Opus. Assert IDs receive the suffix but names do not. Add existing-file
projection cases proving only one terminal suffix is removed and that equal base
IDs with different context or names do not collapse into inheritance.

```go
func TestClaudeManagedEnvRendersContextPerSelection(t *testing.T) {
	oneM := "1m"
	primaryName, opusName := "Sonnet 1M", "Opus 1M"
	config := &modelconfig.ClaudeConfig{
		Primary: modelconfig.Model{Model: "sonnet", Name: &primaryName, Context: &oneM},
		Haiku: modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "haiku"}},
		Sonnet: modelconfig.ClaudeRole{InheritPrimary: true},
		Opus: modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "opus", Name: &opusName, Context: &oneM}},
	}
	env := claudeManagedEnv(config, "http://127.0.0.1:19099", "key")
	for key, want := range map[string]string{
		"ANTHROPIC_MODEL": "sonnet[1m]",
		"ANTHROPIC_CUSTOM_MODEL_OPTION": "sonnet[1m]",
		"ANTHROPIC_DEFAULT_HAIKU_MODEL": "haiku",
		"ANTHROPIC_DEFAULT_SONNET_MODEL": "sonnet[1m]",
		"ANTHROPIC_DEFAULT_OPUS_MODEL": "opus[1m]",
		"ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": "Sonnet 1M",
	} {
		if env[key] != want { t.Fatalf("%s = %q, want %q", key, env[key], want) }
	}
}
```

- [ ] **Step 2: Run focused tests and verify failure**

```bash
go test ./internal/manager/agent -run 'Claude.*Context|DiscoverModels.*Claude'
```

Expected: new assertions fail because the renderer emits base IDs and existing
projection treats `[1m]` as part of the catalog identity.

- [ ] **Step 3: Implement one rendering helper**

Add one non-mutating helper and use it for the primary and each effective role:

```go
func claudeModelValue(selection modelconfig.Model) string {
	if selection.Context != nil && *selection.Context == "1m" {
		return selection.Model + "[1m]"
	}
	return selection.Model
}
```

Do not modify display names and do not add
`CLAUDE_CODE_DISABLE_1M_CONTEXT` to manager-owned keys.

- [ ] **Step 4: Implement exact existing-value projection**

Add a parser that rejects empty or repeated bases and strips only one exact
terminal suffix:

```go
func currentClaudeSelection(id string, name *string) (modelconfig.Model, bool) {
	selection := modelconfig.Model{Model: id, Name: name}
	if strings.HasSuffix(id, "[1m]") {
		base := strings.TrimSuffix(id, "[1m]")
		if base == "" || strings.HasSuffix(base, "[1m]") {
			return modelconfig.Model{}, false
		}
		context := "1m"
		selection.Model = base
		selection.Context = &context
	}
	return selection, true
}
```

Parse primary and role selections completely before deciding inheritance. Add a
small equality helper that compares model, optional name value, and optional
context value. Return base IDs from `currentClaude` so `unavailable` reports the
catalog identity.

- [ ] **Step 5: Run focused and package tests**

```bash
gofmt -w internal/manager/agent/render_claude.go internal/manager/agent/models.go internal/manager/agent/render_test.go internal/manager/agent/models_test.go
go test ./internal/manager/agent -run 'Claude|DiscoverModels'
go test ./internal/manager/agent
```

Expected: PASS. Existing name, merge, drift, and redaction behavior remains
green.

- [ ] **Step 6: Commit renderer and projection behavior**

```bash
git add internal/manager/agent/render_claude.go internal/manager/agent/models.go internal/manager/agent/render_test.go internal/manager/agent/models_test.go
git commit -m "feat: render Claude 1M model variants"
```

## Task 3: Load and Validate the Build-Time Preset

**Files:**
- Create: `internal/manager/preset/preset.go`
- Create: `internal/manager/preset/preset_test.go`
- Modify: `internal/manager/agent/service.go:263-336`

- [ ] **Step 1: Write failing preset loader tests**

Test empty input, valid Base64, invalid Base64, decoded data over
`modelconfig.MaxConfigSize`, malformed JSON, version-only JSON, and protected
fields. Assert errors never contain the encoded or decoded input.

```go
func TestLoad(t *testing.T) {
	raw := `{"version":1,"codex":{"model":"model-a"}}`
	encoded := base64.StdEncoding.EncodeToString([]byte(raw))
	config, err := Load(encoded)
	if err != nil || config == nil || config.Codex == nil || config.Codex.Model != "model-a" {
		t.Fatalf("Load() = %#v, %v", config, err)
	}
	if config, err := Load(""); err != nil || config != nil {
		t.Fatalf("empty Load() = %#v, %v", config, err)
	}
}

func TestLoadRejectsInvalidInputWithoutEcho(t *testing.T) {
	secret := `{"version":1,"codex":{"model":"model-a","api_key":"do-not-echo"}}`
	_, err := Load(base64.StdEncoding.EncodeToString([]byte(secret)))
	if err == nil || strings.Contains(err.Error(), "do-not-echo") {
		t.Fatalf("Load() error = %q", err)
	}
}
```

- [ ] **Step 2: Run tests and verify failure**

```bash
go test ./internal/manager/preset
```

Expected: package or symbols do not exist.

- [ ] **Step 3: Implement the focused preset package**

Use the fixed linker variable and return only sanitized errors:

```go
package preset

import (
	"encoding/base64"
	"errors"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

var Encoded string

func Load(encoded string) (*modelconfig.Config, error) {
	if encoded == "" { return nil, nil }
	decoded, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil { return nil, errors.New("decode Agent model preset") }
	if len(decoded) > modelconfig.MaxConfigSize {
		return nil, errors.New("Agent model preset exceeds size limit")
	}
	config, _, err := modelconfig.DecodeStructure(decoded)
	if err != nil { return nil, errors.New("validate Agent model preset") }
	return config, nil
}
```

Use strict standard Base64. Do not accept raw JSON or alternate encodings.

- [ ] **Step 4: Add immutable preset injection to the Agent service**

Extend `agent.Options` and `Service`:

```go
type Options struct {
	StateDir string
	Detector Detector
	Preset *modelconfig.Config
	LegacyRenderInput *LegacyRenderInput
}

type Service struct {
	// existing fields
	preset *modelconfig.Config
}
```

Canonicalize and decode the supplied preset once in `NewService` before storing
it, so callers cannot mutate maps through a shared pointer:

```go
if options.Preset != nil {
	raw, err := modelconfig.Canonical(options.Preset)
	if err != nil { return nil, operationError(CodeModelStateInvalid, "Agent model preset is invalid") }
	copy, _, err := modelconfig.DecodeStructure(raw)
	if err != nil { return nil, operationError(CodeModelStateInvalid, "Agent model preset is invalid") }
	service.preset = copy
}
```

- [ ] **Step 5: Run tests**

```bash
gofmt -w internal/manager/preset internal/manager/agent/service.go
go test ./internal/manager/preset ./internal/manager/agent
```

Expected: PASS.

- [ ] **Step 6: Commit preset loading**

```bash
git add internal/manager/preset internal/manager/agent/service.go
git commit -m "feat: load build-time agent preset"
```

## Task 4: Return Per-Agent Presets from `agent.models`

**Files:**
- Modify: `internal/manager/agent/models.go:12-100`
- Modify: `internal/manager/agent/models_test.go`
- Modify: `internal/manager/protocol/types.go:239-251`
- Modify: `internal/manager/protocol/server_test.go`
- Modify: `internal/manager/app/app.go:132-181,606-653`
- Modify: `internal/manager/app/app_test.go`
- Modify: `cmd/mtls-router-manager/main_test.go`

- [ ] **Step 1: Write failing Agent service tests for section independence**

Construct a service preset containing all three Agents, select all three, and
provide a catalog that satisfies Claude and opencode but not Codex. Assert the
returned preset contains complete Claude/opencode sections, omits Codex, and
reports only the missing Codex base ID.

```go
func TestDiscoverModelsReturnsIndependentPresetSections(t *testing.T) {
	service := newPresetTestService(t, &modelconfig.Config{
		Version: modelconfig.Version,
		Claude: &modelconfig.ClaudeConfig{
			Primary: modelconfig.Model{Model: "claude-a"},
			Haiku: modelconfig.ClaudeRole{InheritPrimary: true},
			Sonnet: modelconfig.ClaudeRole{InheritPrimary: true},
			Opus: modelconfig.ClaudeRole{InheritPrimary: true},
		},
		OpenCode: &modelconfig.OpenCodeConfig{
			DefaultModel: "open-a",
			Models: map[string]modelconfig.OpenCodeModelConfig{"open-a": {}},
		},
		Codex: &modelconfig.CodexConfig{Model: "codex-a"},
	})
	ctx := context.Background()
	claims := modelconfig.CatalogClaims{
		Models: []string{"claude-a", "open-a"},
		Agents: []modelconfig.Agent{modelconfig.Claude, modelconfig.OpenCode, modelconfig.Codex},
		Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099",
		DeploymentID: "deployment", ProtocolVersion: "2",
	}
	result, err := service.DiscoverModels(ctx, []Kind{Claude, OpenCode, Codex},
		[]string{"claude-a", "open-a"}, claims)
	if err != nil { t.Fatal(err) }
	var config modelconfig.Config
	if err := json.Unmarshal(result.Preset.ModelConfig, &config); err != nil { t.Fatal(err) }
	if config.Claude == nil || config.OpenCode == nil || config.Codex != nil {
		t.Fatalf("preset config = %#v", config)
	}
	missing := result.Preset.UnavailableAgents[string(Codex)]
	if diff := cmp.Diff([]string{"codex-a"}, missing.Models); diff != "" { t.Fatal(diff) }
}
```

Implement `newPresetTestService` beside the test using `t.TempDir()`, a Detector
whose Claude/opencode/Codex paths are absent, and
`NewService(Options{StateDir: ..., Detector: ..., Preset: config})`. Reuse the
test package's existing detector/path helpers where they already provide that
exact behavior; do not add a production helper.

Also test selected-Agent cropping and stable `{}` values when there is no preset.

- [ ] **Step 2: Run focused Agent tests and verify failure**

```bash
go test ./internal/manager/agent -run 'Preset|DiscoverModels'
```

Expected: `ModelsResult.Preset` does not exist.

- [ ] **Step 3: Implement typed preset discovery**

Add service result types separate from `ModelsExisting`:

```go
type ModelsPresetUnavailable struct {
	Code   ErrorCode
	Models []string
}

type ModelsPreset struct {
	ModelConfig       json.RawMessage
	UnavailableAgents map[string]ModelsPresetUnavailable
}

type ModelsResult struct {
	CatalogToken string
	Existing     ModelsExisting
	Preset       ModelsPreset
}
```

Initialize raw objects and maps eagerly. For each selected Agent, obtain the
typed preset section, gather its bounded base IDs, and compare them to the live
catalog. If any are missing, record sorted unique IDs and omit the whole section.
If all exist, wrap only that section in a versioned config and run normal
`modelconfig.Decode` with a single selected Agent before adding it to the result.
Canonicalize the aggregate valid preset once at the end. Do not read or write
Agent files for preset processing.

- [ ] **Step 4: Add stable protocol types and exact JSON tests**

Add:

```go
type AgentModelsPresetUnavailable struct {
	Code   ErrorCode `json:"code"`
	Models []string  `json:"models,omitempty"`
}

type AgentModelsPreset struct {
	ModelConfig       json.RawMessage                          `json:"model_config"`
	UnavailableAgents map[string]AgentModelsPresetUnavailable `json:"unavailable_agents"`
}
```

Add this field to `AgentModelsResult`:

```go
Preset AgentModelsPreset `json:"preset"`
```

Extend
the exact-shape protocol test to require `{}` rather than `null` for empty
`model_config` and `unavailable_agents`.

- [ ] **Step 5: Wire startup and app mapping**

Import `internal/manager/preset` in `app.go`. At the start of `New`, before
transaction recovery or protocol construction, load `preset.Encoded`:

```go
agentPreset, err := preset.Load(preset.Encoded)
if err != nil { return nil, err }
```

Pass it to `agent.NewService`. Extend `agentModels` mapping to translate Agent
error codes through the existing protocol error-code mapping and return stable
maps. The API key must already be cleared before preset processing, as it is in
the current flow.

- [ ] **Step 6: Add app and subprocess tests**

Extend `TestAgentModelsReturnsCompleteKeyFreeCatalogResult` and subprocess tests
to assert the new field. Add a manager startup test that temporarily sets
`preset.Encoded` to invalid Base64 and asserts `run` fails without serving or
echoing input. Restore the package variable with `t.Cleanup`.

- [ ] **Step 7: Run focused tests**

```bash
gofmt -w internal/manager/agent/models.go internal/manager/agent/models_test.go internal/manager/protocol/types.go internal/manager/protocol/server_test.go internal/manager/app/app.go internal/manager/app/app_test.go cmd/mtls-router-manager/main_test.go
go test ./internal/manager/agent -run 'Preset|DiscoverModels'
go test ./internal/manager/protocol -run 'ResultJSONExactShapes|V2Agent'
go test ./internal/manager/app -run AgentModels
go test ./cmd/mtls-router-manager -run 'Serve|ModelsPreviewRefreshWrite'
```

Expected: PASS.

- [ ] **Step 8: Commit protocol preset support**

```bash
git add internal/manager/agent/models.go internal/manager/agent/models_test.go internal/manager/protocol internal/manager/app cmd/mtls-router-manager/main_test.go
git commit -m "feat: return validated agent presets"
```

## Task 5: Update Rust Validation and Desktop Bridge

**Files:**
- Modify: `desktop/src-tauri/src/model_config.rs:21-27,142-180`
- Modify: `desktop/src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing Rust model-config tests**

Add cases accepting `context: "1m"`, rejecting another value through strict
Serde typing, and rejecting a canonical model ending in `[1m]` even when that
exact value appears in the catalog.

```rust
#[test]
fn claude_context_uses_base_catalog_identity() {
    let raw = r#"{"version":1,"claude":{"primary":{"model":"model-a","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}"#;
    let config = import_json(raw.as_bytes()).expect("import context");
    validate(&config, &["claude".into()], &["model-a".into()]).expect("validate context");

    let suffixed = raw.replace("model-a\",", "model-a[1m]\",");
    let config = import_json(suffixed.as_bytes()).expect("parse suffixed");
    assert!(validate(&config, &["claude".into()], &["model-a[1m]".into()]).is_err());
}
```

- [ ] **Step 2: Run Rust test and verify failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked model_config
```

Expected: `context` is an unknown field.

- [ ] **Step 3: Add the strict Rust context type**

Use an enum so Serde rejects all unsupported strings:

```rust
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ClaudeContext {
    #[serde(rename = "1m")]
    OneMillion,
}

pub struct ModelSelection {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ClaudeContext>,
}
```

In `selected_model`, reject `value.model.ends_with("[1m]")` before checking
catalog membership. Update constructors and canonical vectors to include
`context: None` where required.

- [ ] **Step 4: Write failing bridge shape tests**

In `commands.rs`, add manager response fixtures containing both usable preset
sections and unavailable metadata. Assert `agent_models_command` forwards them
unchanged and never stores preset content in `ModelFlow`.

- [ ] **Step 5: Add strict Rust preset response structs**

Add snake-case serializable types mirroring the Go response:

```rust
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentModelsPresetUnavailable {
    code: String,
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentModelsPreset {
    model_config: Value,
    unavailable_agents: HashMap<String, AgentModelsPresetUnavailable>,
}
```

Add `preset` to manager and desktop results. In result validation require both
objects, validate every Agent key against `claude|opencode|codex`, bound model
arrays, and ensure each missing model is a valid catalog-style identifier. Do
not add preset data to `ModelFlow`.

- [ ] **Step 6: Run Rust formatting and tests**

```bash
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked model_config
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked commands
```

Expected: PASS.

- [ ] **Step 7: Commit Rust support**

```bash
git add desktop/src-tauri/src/model_config.rs desktop/src-tauri/src/commands.rs
git commit -m "feat: bridge agent presets to desktop"
```

## Task 6: Add Desktop Initialization and Claude Controls

**Files:**
- Modify: `desktop/src/ipc.ts:108-192`
- Modify: `desktop/src/ipc.test.ts`
- Modify: `desktop/src/AgentPage.tsx:58-74,398-428,746-817`
- Modify: `desktop/src/AgentPage.test.tsx`
- Modify: `desktop/src/modelConfigVectors.test.ts`
- Modify: `desktop/src/locales/en.ts`
- Modify: `desktop/src/locales/zh-CN.ts`
- Modify: `desktop/src/i18n.test.ts`
- Modify: `desktop/src/styles.css`

- [ ] **Step 1: Add failing TypeScript tests for the new IPC shape**

Update test fixtures to include:

```ts
preset: {
  model_config: {
    version: 1,
    claude: {
      primary: { model: "model-a", name: "Model A", context: "1m" },
      haiku: { inherit_primary: true },
      sonnet: { inherit_primary: true },
      opus: { inherit_primary: true },
    },
  },
  unavailable_agents: {
    codex: { code: "MODEL_NOT_AVAILABLE", models: ["model-c"] },
  },
},
```

Assert `discoverModels` returns it without adding it to later key-bearing write
requests.

- [ ] **Step 2: Add failing Agent page tests**

Add one discovery fixture with existing Claude, preset opencode, unavailable
Codex, and all Agents selected. Assert Claude uses existing, opencode uses the
preset, Codex remains empty, and notices identify the sources. Add a Claude test
that edits primary and explicit-role names/context, toggles inheritance, then
previews the exact canonical config. Add a test proving a model change clears
both `name` and `context`.

- [ ] **Step 3: Run focused Vitest and verify failure**

```bash
npm test --prefix desktop -- src/ipc.test.ts src/AgentPage.test.tsx src/modelConfigVectors.test.ts
```

Expected: type or assertion failures because preset/context fields and controls
do not exist.

- [ ] **Step 4: Add TypeScript types and per-Agent initializer**

Extend `ModelSelection`:

```ts
export interface ModelSelection {
  model: string;
  name?: string;
  context?: "1m";
}
```

Add typed `preset` fields to `AgentModelsResult`. Introduce a pure helper in
`AgentPage.tsx` that applies exact per-Agent precedence:

```ts
function initialConfig(
  agents: AgentId[],
  existing: Partial<ModelConfig>,
  preset: Partial<ModelConfig>,
): ModelConfig {
  const result = emptyConfig(agents);
  for (const agent of agents) {
    const section = existing[agent] ?? preset[agent];
    if (section) Object.assign(result, { [agent]: section });
  }
  return result;
}
```

Use this after discovery. Keep desktop import behavior unchanged so import
replaces the current form.

- [ ] **Step 5: Add Claude display-name and context controls**

For primary and each explicit role, render one optional text input and one
select with values `""` and `"1m"`. Update only the edited selection. On model
change replace it with `{ model: event.target.value }`, deliberately clearing
name/context. On inheritance, replace the role with
`{ inherit_primary: true }`; when disabling inheritance, use `{ model: "" }`.

Use translation keys rather than hard-coded explanatory text:

```ts
agents.modelDisplayName
agents.contextMode
agents.contextStandard
agents.contextOneMillion
agents.presetApplied
agents.presetUnavailable
agents.configSourceExisting
agents.configSourcePreset
```

Render source and unavailable notices with existing `drift-note` semantics or a
small dedicated class; do not add a new wizard stage.

- [ ] **Step 6: Update locale parity and canonical vectors**

Add every key to both locale files and extend shared config vectors with one
Claude `context: "1m"` document. Keep exact English/Chinese key parity.

- [ ] **Step 7: Run desktop frontend checks**

```bash
npm run format --prefix desktop
npm test --prefix desktop -- src/ipc.test.ts src/AgentPage.test.tsx src/modelConfigVectors.test.ts src/i18n.test.ts
npm run typecheck --prefix desktop
```

Expected: PASS.

- [ ] **Step 8: Commit desktop UI support**

```bash
git add desktop/src
git commit -m "feat: configure Claude context in desktop"
```

## Task 7: Add Shell Preset Defaults

**Files:**
- Modify: `setup.sh:439-474`
- Modify: `tests/setup_agent_v2_flow_test.sh`

- [ ] **Step 1: Extend the shell protocol fixture and write failing assertions**

Make the fake `agent.models` response include mixed existing/preset/unavailable
sections. Feed blank lines for preset-backed fields and explicit replacement
values for another run. Assert the emitted `agent.preview` or `agent.render`
request contains:

```json
{
  "version": 1,
  "claude": {
    "primary": {
      "model": "claude-a",
      "name": "Claude A",
      "context": "1m"
    },
    "haiku": {"inherit_primary": true},
    "sonnet": {"inherit_primary": true},
    "opus": {"inherit_primary": true}
  }
}
```

Also assert existing sections win, one unavailable Agent receives empty prompts,
and the API key does not appear in stdout, stderr, arguments, or temporary files.

- [ ] **Step 2: Run the shell test and verify failure**

```bash
bash tests/setup_agent_v2_flow_test.sh
```

Expected: request mismatch because the wizard ignores preset values and lacks
Claude context.

- [ ] **Step 3: Add prompt helpers with safe defaults**

Add a helper that keeps the current hidden-input behavior and uses a supplied
default only when the user enters an empty line:

```bash
prompt_value_default() {
  local label="$1" default_value="$2"
  if [[ -n "$default_value" ]]; then
    prompt_value "$label [$default_value]: "
    [[ -n "$PROMPT_VALUE" ]] || PROMPT_VALUE="$default_value"
  else
    prompt_value "$label: "
  fi
}
```

Never use `eval` or interpolate JSON as shell code. Extract defaults with `jq`
and rebuild canonical JSON with `jq -cn` as today.

- [ ] **Step 4: Build the per-Agent initial document**

After `agent.models`, construct one key-free JSON document by selecting each
Agent section from existing first, then preset, then leaving it absent:

```jq
reduce $agents[] as $a
  ({version:1};
   if ($existing[$a] != null) then . + {($a):$existing[$a]}
   elif ($preset[$a] != null) then . + {($a):$preset[$a]}
   else . end)
```

Pass each initial section to `build_model_config`. Print only source labels and
missing model IDs. Keep `--model-config` as a complete replacement that bypasses
the interactive builder.

- [ ] **Step 5: Add Claude context defaults**

Prompt `standard` or `1m` for primary and every explicit role. Omit `context`
for `standard`; emit `"context":"1m"` only for `1m`. Reject other values before
calling the manager. Inherited roles emit only `inherit_primary`.

- [ ] **Step 6: Run shell coverage**

```bash
bash tests/setup_agent_v2_flow_test.sh
make test-shell
```

Expected: PASS.

- [ ] **Step 7: Commit shell behavior**

```bash
git add setup.sh tests/setup_agent_v2_flow_test.sh
git commit -m "feat: apply agent presets in shell setup"
```

## Task 8: Add PowerShell Preset Defaults

**Files:**
- Modify: `setup.ps1:388-428`
- Modify: `tests/setup_powershell_v2_flow_test.sh`
- Modify: `tests/setup_powershell_wizard_test.sh`
- Test: `main_test.go`

- [ ] **Step 1: Write failing PowerShell flow tests**

Mirror the shell cases: preset-backed blank input, existing-over-preset,
unavailable Agent fallback, explicit override, Unicode name, and `context:
"1m"`. Keep assertions that the key is absent and the file has a UTF-8 BOM.

- [ ] **Step 2: Run focused tests and verify failure**

```bash
bash tests/setup_powershell_v2_flow_test.sh
bash tests/setup_powershell_wizard_test.sh
go test . -run PowerShell
```

Expected: request or static-wizard assertions fail.

- [ ] **Step 3: Add typed PowerShell defaults**

Add default-aware readers without stringifying entire JSON documents:

```powershell
function Read-StringDefault([string]$Label, [AllowEmptyString()][string]$Default = '') {
    $suffix = if ($Default) { " [$Default]" } else { '' }
    $value = Read-Host "$Label$suffix"
    if ([string]::IsNullOrEmpty($value)) { return $Default }
    return $value
}
```

Build an ordered initial document per Agent from
`existing.model_config` first and `preset.model_config` second. Pass the section
to `New-AgentModelConfig`; do not deep-merge sections.

- [ ] **Step 4: Add Claude context prompts and source notices**

Accept only empty/`standard`/`1m`. Add `context = '1m'` only for `1m`. Preserve
role inheritance and optional names. Report preset source/unavailability without
printing credentials or raw unredacted fragments.

- [ ] **Step 5: Preserve BOM and run tests**

Use the repository editing mechanism that preserves the existing UTF-8 BOM.
Then run:

```bash
bash tests/setup_powershell_v2_flow_test.sh
bash tests/setup_powershell_wizard_test.sh
go test . -run PowerShell
make test-shell
```

Expected: PASS, including BOM assertions.

- [ ] **Step 6: Commit PowerShell behavior**

```bash
git add setup.ps1 tests/setup_powershell_v2_flow_test.sh tests/setup_powershell_wizard_test.sh main_test.go
git commit -m "feat: apply agent presets in PowerShell setup"
```

## Task 9: Inject Presets into Every Manager Build

**Files:**
- Modify: `scripts/build.sh:27-55`
- Modify: `desktop/scripts/build-sidecars.sh:65-87`
- Modify: `.github/workflows/release.yml:120-140`
- Modify: `tests/desktop_workflow_test.sh`
- Modify: `tests/setup_release_packaging_test.sh`

- [ ] **Step 1: Write failing build-contract tests**

Extend workflow/static tests to require the exact linker symbol
`github.com/codeasier/mtls-router/internal/manager/preset.Encoded` in the local,
desktop-sidecar, and release manager build paths. Assert the value comes from
`AGENT_MODEL_PRESET_BASE64` and is not injected into the router binary.

- [ ] **Step 2: Run workflow tests and verify failure**

```bash
make test-workflows
```

Expected: preset linker-symbol assertions fail.

- [ ] **Step 3: Add optional local and desktop linker injection**

Read the environment without inventing a default model:

```bash
agent_model_preset_base64="${AGENT_MODEL_PRESET_BASE64:-}"
manager_metadata="$metadata -X 'github.com/codeasier/mtls-router/internal/manager/preset.Encoded=$agent_model_preset_base64'"
```

Use `manager_metadata` only for `./cmd/mtls-router-manager`. Keep empty local
values valid. In `build-sidecars.sh`, apply the same value to every target.

- [ ] **Step 4: Add release injection and preflight**

Expose `AGENT_MODEL_PRESET_BASE64` from the GitHub Actions expression
`vars.AGENT_MODEL_PRESET_BASE64`
in both release jobs, matching the repository's use of variables for non-secret
deployment configuration. Before matrix builds,
decode it and run a host manager startup/preflight path that invokes
`preset.Load`; a malformed configured value must fail before artifacts are
published. Pass the exact same environment value to standalone manager builds
and desktop sidecar builds.

Do not print the decoded JSON. Treat an unset or empty release variable as a
valid no-preset build, matching the approved design.

- [ ] **Step 5: Run build and workflow tests**

```bash
make test-workflows
AGENT_MODEL_PRESET_BASE64='' ./scripts/build.sh
```

Expected: PASS and both local binaries build. Placeholder certificates may be
created under ignored `secrets/`; do not stage them.

- [ ] **Step 6: Commit build injection**

```bash
git add scripts/build.sh desktop/scripts/build-sidecars.sh .github/workflows/release.yml tests/desktop_workflow_test.sh tests/setup_release_packaging_test.sh
git commit -m "build: inject agent model preset"
```

## Task 10: Update Compatibility Evidence and User Documentation

**Files:**
- Modify: `internal/manager/agent/testdata/compatibility.json`
- Modify: `internal/manager/agent/render_test.go`
- Modify: `README.md`
- Modify: `docs/zh-CN/README.md`
- Modify: `docs/AGENT_MODELS.md`
- Modify: `docs/zh-CN/AGENT_MODELS.md`
- Modify: `docs/DESKTOP.md`
- Modify: `docs/zh-CN/DESKTOP.md`
- Modify: `docs/BUILD.md`
- Modify: `docs/zh-CN/BUILD.md`
- Modify: `docs/TROUBLESHOOTING.md`
- Modify: `docs/zh-CN/TROUBLESHOOTING.md`
- Modify: `docs/CHANGELOG.md`
- Modify: `docs/zh-CN/CHANGELOG.md`

- [ ] **Step 1: Strengthen pinned Claude compatibility evidence**

Inspect the pinned Claude Code artifact recorded in `compatibility.json` and
update its version, retrieval date, checksums, and source only if the tested
artifact changes. Extend the compatibility test to verify the pinned artifact
or checked evidence contains the supported variables and `[1m]` selection
surface:

```text
ANTHROPIC_CUSTOM_MODEL_OPTION_NAME
ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME
ANTHROPIC_DEFAULT_SONNET_MODEL_NAME
ANTHROPIC_DEFAULT_OPUS_MODEL_NAME
[1m]
```

Do not merely change the manifest date. Record reproducible source URL,
revision, integrity, and digest evidence.

- [ ] **Step 2: Update the authoritative Agent contract in both languages**

Document exact canonical examples and rules:

```json
{"model":"claude-sonnet-4-6","name":"Sonnet 4.6","context":"1m"}
```

State that `[1m]` is rendered only at the Claude boundary, base IDs remain
catalog identities, 1M capability is not inferred, and inherited roles inherit
all selection metadata. Add the `preset` protocol response, per-Agent
unavailability, `existing > preset > empty`, import precedence, and sidecar
exclusion.

- [ ] **Step 3: Update setup, desktop, build, and troubleshooting docs in both languages**

Document:

- display-name and Standard/1M desktop controls;
- editable preset defaults after authenticated discovery;
- `AGENT_MODEL_PRESET_BASE64` generation and optional local behavior;
- identical standalone/desktop manager injection;
- manager startup failure for malformed build input;
- nonfatal per-Agent preset catalog mismatch;
- rejection of canonical model IDs containing terminal `[1m]`.

Revise statements saying models are never selected automatically so they
distinguish unsafe inference from an authenticated, validated, visible preset.
The final wording must still prohibit first-model and name-based inference.

- [ ] **Step 4: Add changelog entries**

Add concise unreleased entries for Claude display/context configuration and
validated build-time presets. Keep English and Chinese meaning aligned.

- [ ] **Step 5: Run documentation and compatibility checks**

```bash
go test ./internal/manager/agent -run Compatibility
make test-shell
make test-workflows
```

Expected: PASS.

- [ ] **Step 6: Commit documentation**

```bash
git add internal/manager/agent/testdata/compatibility.json internal/manager/agent/render_test.go README.md docs
git commit -m "docs: document agent presets and Claude context"
```

## Task 11: Run Complete Verification and Review the Integrated Diff

**Files:**
- Modify only files required to fix failures caused by Tasks 1-10.

- [ ] **Step 1: Run Go formatting and tests**

```bash
test -z "$(gofmt -l .)"
go test ./...
go vet ./...
```

Expected: all commands exit 0.

- [ ] **Step 2: Run setup and workflow suites**

```bash
make test-shell
make test-workflows
```

Expected: PASS, including key non-leakage, PowerShell BOM, and release linker
contract tests.

- [ ] **Step 3: Run complete desktop verification**

```bash
make desktop-verify
```

Expected: ESLint, Prettier, TypeScript, Vitest, Vite build, Rust formatting, and
Rust tests all pass.

- [ ] **Step 4: Inspect security and compatibility invariants**

Search the integrated diff and test output to confirm:

```bash
git diff --check
```

Review these invariants manually:

- API keys appear only in `agent.models` and `agent.write` request handling and
  final Agent auth configuration, never in preset/config tokens/logs.
- Preset content is absent from ownership sidecars, journals, and revision
  claims until the user-approved canonical config follows normal preview/write.
- Every selected model is revalidated as a base ID before write.
- No first-model, fuzzy-name, partial-preset, or model-substitution fallback was
  introduced.
- Both manager build paths inject the same linker value.
- `setup.ps1` retains its UTF-8 BOM.

- [ ] **Step 5: Commit verification fixes only when files changed**

If verification required code changes, inspect `git status --short`, stage each
listed fix by its exact path, and commit:

```bash
git status --short
git add path/to/first-fixed-file path/to/second-fixed-file
git commit -m "fix: complete agent preset validation"
```

Replace the example paths with only the files shown by status that were changed
to fix verification failures. If no changes were required, do not create an
empty commit.

- [ ] **Step 6: Report completion**

Report the commit range, exact verification commands, and any residual testing
gap such as unavailable Windows PowerShell or cross-platform packaging on the
current host. Do not claim unrun checks passed.
