# OpenCode Model Variants Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add first-class OpenCode variants and Claude numeric token budgets across the canonical manager, desktop interchange/editor, and local v1 preset.

**Architecture:** Extend the canonical model-config types and strict validators first, then carry the new typed fields through rendering, current-file projection, ownership, and desktop round-trip. Preserve legacy `extra.variants` input but reject dual definitions. Keep Claude numeric budgets global to the Claude section, mutually exclusive with selection-level `context: "1m"`, and render them only as Claude Code environment variables.

**Tech Stack:** Go 1.26, JSON Schema draft 2020-12, Rust/Serde, React 19/TypeScript, Vitest, shell preflight tests.

---

## File Map

- `internal/manager/agent/modelconfig/types.go`: canonical Go fields for OpenCode variants and Claude budgets.
- `internal/manager/agent/modelconfig/validate.go`: structural validation, protected-tree checks, dual-definition rejection, and Claude budget relationships.
- `internal/manager/agent/modelconfig/schema.go`: generated interchange schema source.
- `internal/manager/agent/modelconfig/schema/model-config-v1.schema.json`: checked-in generated schema.
- `internal/manager/agent/modelconfig/modelconfig_test.go`: canonical decode, validation, and schema synchronization tests.
- `internal/manager/agent/modelconfig/testdata/jcs-vectors.json`: shared canonical round-trip vector.
- `internal/manager/agent/render_opencode.go`: emit typed top-level variants.
- `internal/manager/agent/render_claude.go`: render and own Claude Code token-budget variables.
- `internal/manager/agent/models.go`: project managed variants and Claude budgets from existing files.
- `internal/manager/agent/render_test.go`: exact renderer and merge behavior tests.
- `internal/manager/agent/models_test.go`: current-file projection and drift/prefill tests.
- `internal/manager/agent/sidecar_test.go`: owned-path persistence assertions.
- `desktop/src-tauri/src/model_config.rs`: Rust interchange types and matching validation.
- `desktop/src/ipc.ts`: TypeScript interchange types.
- `desktop/src/modelConfigVectors.test.ts`: schema-surface assertions.
- `desktop/src/AgentPage.tsx`: typed Claude budget inputs and OpenCode variants JSON editor.
- `desktop/src/AgentPage.test.tsx`: UI initialization/edit/submission round-trip tests.
- `desktop/src/locales/en.ts`: English labels.
- `desktop/src/locales/zh-CN.ts`: Chinese labels.
- `tmp/agent-model-preset-v1.json`: local, ignored target preset; do not stage or commit.
- `tmp/AGENT_MODEL_PRESET_GUIDE.zh-CN.md`: local preset guide; do not stage or commit if ignored.
- `docs/AGENT_MODELS.md`: English canonical field and behavior contract.
- `docs/zh-CN/AGENT_MODELS.md`: Chinese contract parity.

### Task 1: Extend The Canonical Model Config

**Files:**
- Modify: `internal/manager/agent/modelconfig/types.go`
- Modify: `internal/manager/agent/modelconfig/validate.go`
- Test: `internal/manager/agent/modelconfig/modelconfig_test.go`

- [ ] **Step 1: Add failing canonical tests for valid new fields**

Add a test input containing Claude budgets and typed OpenCode variants, then assert the decoded values survive canonical serialization:

```go
func TestClaudeBudgetsAndOpenCodeVariants(t *testing.T) {
	input := []byte(`{"version":1,"claude":{"primary":{"model":"m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":353400,"max_output_tokens":100000},"opencode":{"default_model":"m","models":{"m":{"variants":{"medium":{"reasoningEffort":"medium"},"custom":{"reasoningSummary":"auto"}}}}}}`)
	config, err := Decode(input, []Agent{Claude, OpenCode}, []string{"m"})
	if err != nil {
		t.Fatal(err)
	}
	if config.Claude.ContextWindow == nil || *config.Claude.ContextWindow != 353400 || config.Claude.MaxOutputTokens == nil || *config.Claude.MaxOutputTokens != 100000 {
		t.Fatalf("Claude budgets = %#v", config.Claude)
	}
	variants := config.OpenCode.Models["m"].Variants
	if len(variants) != 2 || variants["medium"]["reasoningEffort"] != "medium" {
		t.Fatalf("variants = %#v", variants)
	}
}
```

- [ ] **Step 2: Add failing validation-table cases**

Extend the invalid-input table with exact cases for:

```go
{name: "Claude zero context", input: `{"version":1,"claude":{"primary":{"model":"m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":0}}`, rule: "positive_integer"},
{name: "Claude output equals context", input: `{"version":1,"claude":{"primary":{"model":"m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":100,"max_output_tokens":100}}`, rule: "integer_relationship"},
{name: "Claude numeric and 1m", input: `{"version":1,"claude":{"primary":{"model":"m","context":"1m"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":100}}`, rule: "context_conflict"},
{name: "variant is not object", input: `{"version":1,"opencode":{"default_model":"m","models":{"m":{"variants":{"high":true}}}}}`, rule: "object"},
{name: "protected variant key", input: `{"version":1,"opencode":{"default_model":"m","models":{"m":{"variants":{"high":{"api_key":"secret"}}}}}}`, rule: "protected_path"},
{name: "dual variants", input: `{"version":1,"opencode":{"default_model":"m","models":{"m":{"variants":{"high":{"reasoningEffort":"high"}},"extra":{"variants":{"low":{"reasoningEffort":"low"}}}}}}}`, rule: "field_conflict"},
```

Also add valid cases proving `context_window` and `max_output_tokens` may each appear alone, and a role-level `context: "1m"` conflicts with numeric `context_window` even when primary uses standard context.

- [ ] **Step 3: Run focused tests and verify they fail**

Run:

```bash
go test ./internal/manager/agent/modelconfig -run 'TestClaudeBudgetsAndOpenCodeVariants|Test.*Invalid' -count=1
```

Expected: FAIL because the new fields are rejected as unknown or absent from the Go types.

- [ ] **Step 4: Add the canonical Go fields**

Add fields with these exact JSON names:

```go
type ClaudeConfig struct {
	Primary         Model             `json:"primary"`
	Haiku           ClaudeRole        `json:"haiku"`
	Sonnet          ClaudeRole        `json:"sonnet"`
	Opus            ClaudeRole        `json:"opus"`
	ContextWindow   *int64            `json:"context_window,omitempty"`
	MaxOutputTokens *int64            `json:"max_output_tokens,omitempty"`
	Extra           map[string]string `json:"extra,omitempty"`
}

type OpenCodeModelConfig struct {
	// existing fields remain unchanged
	Options  map[string]any            `json:"options,omitempty"`
	Variants map[string]map[string]any `json:"variants,omitempty"`
	Extra    map[string]any            `json:"extra,omitempty"`
}
```

- [ ] **Step 5: Implement Claude budget validation**

Allow `context_window` and `max_output_tokens` in `parseClaude`. Parse each with `positive`, reject `max_output_tokens >= context_window` when both exist, and reject any selection context when `context_window` exists:

```go
if c.ContextWindow != nil {
	if c.Primary.Context != nil {
		return nil, invalid("/claude/primary/context", "context_conflict")
	}
	for key, role := range map[string]ClaudeRole{"haiku": c.Haiku, "sonnet": c.Sonnet, "opus": c.Opus} {
		if role.Selection != nil && role.Selection.Context != nil {
			return nil, invalid("/claude/"+key+"/context", "context_conflict")
		}
	}
}
```

Use `positive_integer` for invalid scalar budget values and `integer_relationship` for the cross-field comparison.

- [ ] **Step 6: Implement typed variants validation**

Allow `variants` in `parseOpenCodeModel`. Require a JSON object, reject empty/control/over-128-byte variant names, require each variant value to be an object, and call `validateTree` on each option object. Convert every object with `mapFromObject`.

Before assigning `Extra`, detect a `variants` key when typed `Variants` is non-nil:

```go
if m.Variants != nil {
	if _, duplicate := v["variants"]; duplicate {
		return nil, invalid(path+"/variants", "field_conflict")
	}
}
```

Retain `variants` in the `extra` top-level allowlist for backward compatibility.

- [ ] **Step 7: Run and pass canonical tests**

Run:

```bash
go test ./internal/manager/agent/modelconfig -count=1
```

Expected: PASS.

### Task 2: Update JSON Schema And Shared Interchange

**Files:**
- Modify: `internal/manager/agent/modelconfig/schema.go`
- Modify: `internal/manager/agent/modelconfig/schema/model-config-v1.schema.json`
- Modify: `internal/manager/agent/modelconfig/testdata/jcs-vectors.json`
- Modify: `desktop/src-tauri/src/model_config.rs`
- Modify: `desktop/src/ipc.ts`
- Modify: `desktop/src/modelConfigVectors.test.ts`
- Test: `internal/manager/agent/modelconfig/modelconfig_test.go`
- Test: `desktop/src-tauri/src/model_config.rs`

- [ ] **Step 1: Add failing schema-surface assertions**

Update `desktop/src/modelConfigVectors.test.ts` so `openCodeModel` expects `variants`, and the Claude property list expects both budget fields:

```ts
expect(Object.keys(schema.$defs.openCodeModel.properties ?? {}).sort()).toContain(
  "variants",
);
expect(
  Object.keys(schema.properties.claude.properties ?? {}).sort(),
).toEqual([
  "context_window",
  "extra",
  "haiku",
  "max_output_tokens",
  "opus",
  "primary",
  "sonnet",
]);
```

- [ ] **Step 2: Add Rust validation tests before fields**

In the existing `#[cfg(test)]` module in `model_config.rs`, add tests that deserialize and validate:

```rust
r#"{"version":1,"claude":{"primary":{"model":"m1"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true},"context_window":353400,"max_output_tokens":100000},"opencode":{"default_model":"m1","models":{"m1":{"variants":{"medium":{"reasoningEffort":"medium"}}}}}}"#
```

Add rejection assertions for equal budgets, numeric context plus `"1m"`, unsafe nested variant keys, and simultaneous typed/extra variants.

- [ ] **Step 3: Run the cross-language tests and verify failure**

Run:

```bash
go test ./internal/manager/agent/modelconfig -run TestGeneratedSchemaMatchesCheckedIn -count=1
npm test -- --run src/modelConfigVectors.test.ts
cargo test --manifest-path desktop/src-tauri/Cargo.toml model_config --locked
```

Expected: FAIL because schema and Rust/TypeScript surfaces do not expose the new fields.

- [ ] **Step 4: Update schema source**

In `schema.go`:

- add Claude `context_window` and `max_output_tokens` properties using the existing positive safe `integer` definition;
- add OpenCode model `variants` as an object with `propertyNames` requiring at least one character and `additionalProperties` referencing an object extension definition;
- keep recursive/protected-key validation documented as manager-authoritative.

Use a separate object schema for a variant value rather than the current unconstrained `extension` wrapper:

```go
variantOptions := map[string]any{
	"type": "object",
	"description": "Recursively bounded and protected-key validated by the manager",
}
variants := map[string]any{
	"type":                 "object",
	"propertyNames":        map[string]any{"minLength": 1, "maxLength": 128},
	"additionalProperties": variantOptions,
}
```

- [ ] **Step 5: Regenerate the checked-in schema deterministically**

Use a temporary Go test or small temporary Go program outside the repository to call `GenerateSchema`, then replace `internal/manager/agent/modelconfig/schema/model-config-v1.schema.json` with its exact output. Remove the temporary program afterward.

Run:

```bash
go test ./internal/manager/agent/modelconfig -run TestGeneratedSchemaMatchesCheckedIn -count=1
```

Expected: PASS, not a manually edited near-match.

- [ ] **Step 6: Update Rust and TypeScript interchange types**

Add these fields:

```rust
pub struct ClaudeConfig {
    // existing selections
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    // existing extra
}

pub struct OpenCodeModel {
    // existing fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, Map<String, Value>>>,
    // existing extra
}
```

```ts
export interface ClaudeModelConfig {
  // existing selections
  context_window?: number;
  max_output_tokens?: number;
  extra?: Record<string, string>;
}

export interface OpenCodeModelConfig {
  // existing fields
  variants?: Record<string, JsonObject>;
  extra?: JsonObject;
}
```

- [ ] **Step 7: Mirror manager validation in Rust**

Validate safe positive integers, `max_output_tokens < context_window`, numeric context versus every explicit `OneMillion` selection, variant names/objects, protected trees, and typed/extra dual definitions. Reuse `validate_extension` for each variant options map instead of creating a second recursive validator.

- [ ] **Step 8: Extend a shared JCS vector**

Add the new Claude budgets and OpenCode variants to one valid entry in `jcs-vectors.json`, then update its canonical JCS string exactly. This proves Go, Rust/Serde, and TypeScript preserve the fields rather than merely accepting them.

- [ ] **Step 9: Run shared interchange tests**

Run:

```bash
go test ./internal/manager/agent/modelconfig -count=1
npm test -- --run src/modelConfigVectors.test.ts
cargo test --manifest-path desktop/src-tauri/Cargo.toml model_config --locked
```

Expected: PASS.

### Task 3: Render And Re-Extract Managed Fields

**Files:**
- Modify: `internal/manager/agent/render_opencode.go`
- Modify: `internal/manager/agent/render_claude.go`
- Modify: `internal/manager/agent/models.go`
- Test: `internal/manager/agent/render_test.go`
- Test: `internal/manager/agent/models_test.go`
- Test: `internal/manager/agent/sidecar_test.go`

- [ ] **Step 1: Add failing exact-render tests**

Create an OpenCode test model with typed variants and assert rendered output has:

```json
"variants": {
  "medium": { "reasoningEffort": "medium" }
}
```

at `provider.mtls-router.models.<id>.variants`.

Create a Claude config with both budgets and assert exact string values:

```go
"CLAUDE_CODE_MAX_CONTEXT_TOKENS": "353400",
"CLAUDE_CODE_MAX_OUTPUT_TOKENS":  "100000",
```

Also verify omitted typed fields remove stale owned values during `mergeClaude`.

- [ ] **Step 2: Add failing current-file projection tests**

In `models_test.go`, supply managed Claude JSON containing both budget variables and OpenCode JSON containing `options`, typed `variants`, and legacy-safe fields. Assert `DiscoverModels` returns budgets and variants in canonical existing config.

Add malformed numeric-string cases such as `"35e4"`, `"0"`, and values over `MaxSafeInteger`; projection should reject the section rather than repair it.

- [ ] **Step 3: Add failing sidecar ownership assertion**

Assert a Claude sidecar created from configured budgets includes:

```text
env.CLAUDE_CODE_MAX_CONTEXT_TOKENS
env.CLAUDE_CODE_MAX_OUTPUT_TOKENS
```

in sorted owned paths.

- [ ] **Step 4: Run focused agent tests and verify failure**

Run:

```bash
go test ./internal/manager/agent -run 'Test.*(Claude|OpenCode|Sidecar|Models)' -count=1
```

Expected: FAIL because rendering and current projection omit the new fields.

- [ ] **Step 5: Render typed OpenCode variants**

In `openCodeProvider`, add typed variants before the `DeepMerge` call:

```go
if config.Variants != nil {
	entry["variants"] = config.Variants
}
```

The validator guarantees typed and legacy `extra.variants` cannot coexist, so no renderer precedence logic is needed.

- [ ] **Step 6: Render and own Claude numeric budgets**

Append both environment names to `claudeFixedEnvKeys`. In `claudeManagedEnv`, format configured integers in base 10:

```go
if config.ContextWindow != nil {
	result["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] = strconv.FormatInt(*config.ContextWindow, 10)
}
if config.MaxOutputTokens != nil {
	result["CLAUDE_CODE_MAX_OUTPUT_TOKENS"] = strconv.FormatInt(*config.MaxOutputTokens, 10)
}
```

The existing merge and sidecar loops then remove stale values and record owned paths without separate special cases.

- [ ] **Step 7: Project Claude budgets from existing settings**

In `currentClaude`, parse optional raw JSON strings with a helper that accepts only canonical decimal positive integers:

```go
func rawPositiveIntString(raw json.RawMessage) (*int64, bool) {
	value, present := rawString(raw)
	if !present {
		return nil, true
	}
	parsed, err := strconv.ParseInt(value, 10, 64)
	if err != nil || parsed <= 0 || parsed > modelconfig.MaxSafeInteger || strconv.FormatInt(parsed, 10) != value {
		return nil, false
	}
	return &parsed, true
}
```

Assign the two optional values to `ClaudeConfig`, marshal a temporary
`{"version":1,"claude":...}` document, and call `modelconfig.DecodeStructural`
on it before returning the section. Return the decoded Claude section so budget
relationships and `[1m]` conflicts use the same authoritative validator as
requests and presets.

- [ ] **Step 8: Preserve OpenCode variants and options during projection**

Extend the `currentOpenCode` typed projection key list to include:

```go
"options", "variants"
```

Do not project arbitrary `extra`, provider identity, headers, URLs, or credentials.

- [ ] **Step 9: Run agent tests**

Run:

```bash
go test ./internal/manager/agent -count=1
```

Expected: PASS.

### Task 4: Expose The Fields In The Desktop Editor

**Files:**
- Modify: `desktop/src/AgentPage.tsx`
- Modify: `desktop/src/AgentPage.test.tsx`
- Modify: `desktop/src/locales/en.ts`
- Modify: `desktop/src/locales/zh-CN.ts`

- [ ] **Step 1: Add failing UI round-trip test**

Initialize discovery with a preset containing:

```ts
claude: {
  primary: { model: "model-a" },
  haiku: { inherit_primary: true },
  sonnet: { inherit_primary: true },
  opus: { inherit_primary: true },
  context_window: 353400,
  max_output_tokens: 100000,
},
opencode: {
  default_model: "model-a",
  models: {
    "model-a": {
      variants: { medium: { reasoningEffort: "medium" } },
    },
  },
},
```

Assert the two Claude number inputs display their values and the variants editor displays `reasoningEffort`. Edit all three and assert the preview API receives the updated typed fields.

- [ ] **Step 2: Run the UI test and verify failure**

Run:

```bash
npm test -- --run src/AgentPage.test.tsx
```

Expected: FAIL because no controls render the new fields.

- [ ] **Step 3: Add translated labels**

Add matching locale keys:

```ts
"agents.claudeContextWindow": "Claude context window",
"agents.claudeMaxOutputTokens": "Claude max output tokens",
"agents.variantsJson": "Variants JSON",
```

```ts
"agents.claudeContextWindow": "Claude 上下文窗口",
"agents.claudeMaxOutputTokens": "Claude 最大输出 Token",
"agents.variantsJson": "Variants JSON",
```

- [ ] **Step 4: Add Claude typed number controls**

Render two `OptionalNumber` controls after the primary selection. Update only the selected field while preserving roles and extras:

```tsx
<OptionalNumber
  label={t("agents.claudeContextWindow")}
  value={config.claude.context_window}
  onChange={(context_window) =>
    setConfig({
      ...config,
      claude: { ...config.claude!, context_window },
    })
  }
/>
```

Add the equivalent control for `max_output_tokens`.

- [ ] **Step 5: Add typed variants JSON editor**

In `OpenCodeSettings`, add an `ObjectField` before model `extra`:

```tsx
<ObjectField
  label={t("agents.variantsJson")}
  value={settings.variants}
  onChange={(value) =>
    update({
      ...settings,
      variants: value as Record<string, JsonObject> | undefined,
    })
  }
/>
```

Keep `extra` separately editable for legacy imports; manager/Rust validation rejects dual `variants` definitions.

- [ ] **Step 6: Run desktop frontend checks**

Run:

```bash
npm test -- --run src/AgentPage.test.tsx src/modelConfigVectors.test.ts
npm run typecheck
npm run static:check
```

Expected: PASS.

### Task 5: Update The Local V1 Preset And Guide

**Files:**
- Modify locally: `tmp/agent-model-preset-v1.json`
- Modify locally: `tmp/AGENT_MODEL_PRESET_GUIDE.zh-CN.md`

- [ ] **Step 1: Update Claude and Codex defaults**

Change Claude Haiku to:

```json
"haiku": {
  "model": "gpt-5.6-luna",
  "name": "GPT 5.6 Luna"
}
```

Add to the Claude section:

```json
"context_window": 353400,
"max_output_tokens": 100000
```

Change Codex `reasoning_effort` from `max` to `medium`.

- [ ] **Step 2: Set every OpenCode default to medium**

For all five OpenCode models, set:

```json
"options": {
  "reasoningEffort": "medium"
}
```

- [ ] **Step 3: Add the six-level GPT 5.4/5.5 variants**

Add this exact top-level model field to both models:

```json
"variants": {
  "none": { "reasoningEffort": "none" },
  "minimal": { "reasoningEffort": "minimal" },
  "low": { "reasoningEffort": "low" },
  "medium": { "reasoningEffort": "medium" },
  "high": { "reasoningEffort": "high" },
  "xhigh": { "reasoningEffort": "xhigh" }
}
```

- [ ] **Step 4: Add the seven-level GPT 5.6 variants**

Add the same six levels plus:

```json
"max": { "reasoningEffort": "max" }
```

to Sol, Terra, and Luna.

- [ ] **Step 5: Update the Chinese guide**

Revise all affected tables and examples so the guide states:

- Claude Haiku uses Luna;
- Claude renders both numeric budget variables;
- numeric context override is effective for unknown custom model names on Claude Code 2.1.193+, but older versions remain usable and may ignore it;
- Fable independently needs Claude Code 2.1.170+;
- OpenCode contains six/seven explicit variants and all five defaults are `medium`;
- Codex reasoning effort is `medium`;
- variants are product metadata, not a blind copy of the currently explicit local provider variants.

Remove the obsolete claim that canonical OpenCode rejects variants and the obsolete claim that Claude has no numeric context configuration.

- [ ] **Step 6: Validate the local JSON and matrix exactly**

Run:

```bash
jq -e '
  .claude.haiku.model == "gpt-5.6-luna" and
  .claude.context_window == 353400 and
  .claude.max_output_tokens == 100000 and
  .codex.reasoning_effort == "medium" and
  ([.opencode.models[].options.reasoningEffort] | all(. == "medium")) and
  ([.opencode.models["gpt-5.4", "gpt-5.5"].variants | keys | length] | all(. == 6)) and
  ([.opencode.models["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"].variants | keys | length] | all(. == 7))
' tmp/agent-model-preset-v1.json >/dev/null
```

Expected: exit 0.

- [ ] **Step 7: Run strict preset preflight and classify the result**

Run:

```bash
AGENT_MODEL_PRESET_BASE64="$(openssl base64 -A < tmp/agent-model-preset-v1.json)" ./scripts/preflight-agent-model-preset.sh
```

Expected before Fable implementation: sanitized failure caused by the still-unknown Claude `fable` field. It must not fail first on OpenCode `variants`, Claude budgets, or the changed model metadata. Do not remove `fable` to obtain a false pass.

### Task 6: Update Public Canonical Documentation

**Files:**
- Modify: `docs/AGENT_MODELS.md`
- Modify: `docs/zh-CN/AGENT_MODELS.md`

- [ ] **Step 1: Document OpenCode variants in English**

State that each model may contain typed top-level `variants`, mapping variant names to bounded, protected provider option objects. Document legacy `extra.variants` compatibility and rejection when both locations appear.

- [ ] **Step 2: Document Claude numeric budgets in English**

Add the exact canonical fields and rendered environment names. Explain positive safe integers, the output/context relationship, the `[1m]` conflict, 2.1.193 effectiveness without a hard minimum, and that these values change Claude Code budgeting rather than upstream capability.

- [ ] **Step 3: Mirror the contract in Chinese**

Update the corresponding sections in `docs/zh-CN/AGENT_MODELS.md` with the same field names, constraints, compatibility behavior, and version nuance. Preserve terminology already used in the file.

- [ ] **Step 4: Check documentation parity and formatting**

Run:

```bash
git diff --check -- docs/AGENT_MODELS.md docs/zh-CN/AGENT_MODELS.md
```

Expected: no output.

### Task 7: Full Verification

**Files:**
- Verify all modified tracked files.
- Verify local ignored preset files separately.

- [ ] **Step 1: Format tracked source**

Run:

```bash
gofmt -w internal/manager/agent/modelconfig/types.go internal/manager/agent/modelconfig/validate.go internal/manager/agent/modelconfig/schema.go internal/manager/agent/modelconfig/modelconfig_test.go internal/manager/agent/render_opencode.go internal/manager/agent/render_claude.go internal/manager/agent/models.go internal/manager/agent/render_test.go internal/manager/agent/models_test.go internal/manager/agent/sidecar_test.go
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all
npm run format -- --write src/AgentPage.tsx src/AgentPage.test.tsx src/ipc.ts src/modelConfigVectors.test.ts src/locales/en.ts src/locales/zh-CN.ts
```

Run the npm commands from `desktop/`.

- [ ] **Step 2: Run all Go validation**

Run:

```bash
go test ./...
go vet ./...
test -z "$(gofmt -l .)"
```

Expected: all commands exit 0.

- [ ] **Step 3: Run desktop validation**

Run from `desktop/`:

```bash
npm run verify
```

Expected: ESLint, Prettier, TypeScript, Vitest, Vite build, Rust formatting, and Rust tests all pass.

- [ ] **Step 4: Run workflow tests**

Run:

```bash
make test-workflows
```

Expected: all workflow scripts pass.

- [ ] **Step 5: Inspect final diff without staging ignored preset files**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only intended tracked source/docs changes appear. `tmp/agent-model-preset-v1.json` and its local guide may be absent from status because `tmp/` is ignored; verify them directly with the Task 5 `jq` command instead of forcing them into Git.

- [ ] **Step 6: Record the remaining blocker accurately**

The implementation is complete when manager and desktop tests pass, the local preset passes JSON/matrix checks, and strict preflight reaches only the independent Fable schema blocker. Do not claim the complete preset is injectable until Fable support is implemented and strict preflight exits 0.
