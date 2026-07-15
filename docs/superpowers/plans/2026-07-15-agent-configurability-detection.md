# Agent Configurability Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow configuration preview and writing for every supported Agent regardless of whether its CLI is visible to the manager process.

**Architecture:** Keep executable lookup as optional `Command` metadata, but decouple the public `Detected` state from lookup success. All three statically supported Agent kinds are detected; existing configuration validity and writability checks remain the safety gates.

**Tech Stack:** Go 1.26, standard library, package-local Go tests

---

## File Structure

- Modify `internal/manager/agent/detect_test.go`: lock down uniform detection with no visible CLIs or configuration files.
- Modify `internal/manager/agent/service_test.go`: prove preview creation works when CLI lookup fails.
- Modify `internal/manager/agent/detect.go`: make supported-Agent detection independent of CLI lookup and remove the Codex-only directory fallback.

### Task 1: Lock Down Supported-Agent Detection

**Files:**
- Modify: `internal/manager/agent/detect_test.go:12-96`

- [ ] **Step 1: Write the failing detector test**

Add a test using `testDetector(home, nil)` and no configuration files. Assert all three states have `Detected == true`, `Command == ""`, `Exists == false`, and `Writable == true`.

```go
func TestDetectTreatsSupportedAgentsAsConfigurableWithoutCLIOrConfig(t *testing.T) {
	home := t.TempDir()
	states := mustDetect(t, testDetector(home, nil))

	for _, state := range states {
		if !state.Detected || state.Command != "" || state.Exists || !state.Writable {
			t.Errorf("unsupported configurable state = %#v", state)
		}
	}
}
```

- [ ] **Step 2: Run the detector test and verify failure**

Run: `go test ./internal/manager/agent -run TestDetectTreatsSupportedAgentsAsConfigurableWithoutCLIOrConfig -count=1`

Expected: FAIL because Claude Code and opencode currently return `Detected: false`.

### Task 2: Prove Preview Is Not CLI-Gated

**Files:**
- Modify: `internal/manager/agent/service_test.go:104-110`

- [ ] **Step 1: Replace the obsolete missing-Agent test**

Replace the `missing Agent` subtest with a preview test that uses no available commands and selects all supported Agents.

```go
	t.Run("missing CLIs and configurations are ready to create", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil, nil)
		preview, err := service.Preview(context.Background(), []Kind{ClaudeCode, OpenCode, Codex})
		if err != nil {
			t.Fatal(err)
		}
		if len(preview.Agents) != 3 {
			t.Fatalf("preview Agents = %#v", preview.Agents)
		}
		for _, agent := range preview.Agents {
			for _, file := range agent.Files {
				if file.Operation != OperationCreate {
					t.Fatalf("preview file = %#v", file)
				}
			}
		}
	})
```

- [ ] **Step 2: Run the service test and verify failure**

Run: `go test ./internal/manager/agent -run 'TestPreviewMissingInvalidConflictAndAlreadyConfigured/missing_CLIs' -count=1`

Expected: FAIL with `Claude Code is not detected` and code `AGENT_NOT_FOUND`.

### Task 3: Decouple Detection from Executable Lookup

**Files:**
- Modify: `internal/manager/agent/detect.go:78-102`

- [ ] **Step 1: Implement the minimal detection change**

Keep lookup results only for command metadata, pass `true` as the detected state for every supported Agent, and remove Codex's home-directory fallback and `<desktop>` sentinel.

```go
	claudeCommand, _ := lookup(lookPath, "claude")
	openCodeCommand, _ := lookup(lookPath, "opencode")
	codexCommand, _ := lookup(lookPath, "codex")

	claudePaths := ClaudePaths(home, getenv("CLAUDE_CONFIG_DIR"))
	openCodeOverride := getenv("OPENCODE_CONFIG")
	openCodePaths := OpenCodePaths(home, openCodeOverride)
	codexPaths := CodexPaths(home, getenv("CODEX_HOME"))

	claude := inspectJSONState(ClaudeCode, "Claude Code", true, claudeCommand, claudePaths, FormatJSON, inspectClaude)
	// Preserve existing opencode format selection.
	openCode := inspectJSONState(OpenCode, "opencode", true, openCodeCommand, openCodePaths, openCodeFormat, inspectOpenCode)
	codex := inspectCodex(true, codexCommand, codexPaths)
```

Update the existing environment-path test to expect Codex detected with an empty command when lookup fails. Replace the obsolete Codex desktop-directory test with assertions already covered by the new uniform test.

- [ ] **Step 2: Format and run focused tests**

Run: `gofmt -w internal/manager/agent/detect.go internal/manager/agent/detect_test.go internal/manager/agent/service_test.go`

Run: `go test ./internal/manager/agent -count=1`

Expected: PASS.

### Task 4: Verify Repository Checks

**Files:**
- Verify only; no planned source changes

- [ ] **Step 1: Run all Go tests**

Run: `go test ./...`

Expected: PASS.

- [ ] **Step 2: Run static analysis**

Run: `go vet ./...`

Expected: PASS.

- [ ] **Step 3: Verify formatting**

Run: `test -z "$(gofmt -l .)"`

Expected: exit status 0 with no output.

- [ ] **Step 4: Run desktop tests because detection drives desktop selection**

Run: `npm test -- --run`

Working directory: `desktop`

Expected: PASS.

- [ ] **Step 5: Inspect final diff**

Run: `git status --short && git diff --check && git diff -- internal/manager/agent/detect.go internal/manager/agent/detect_test.go internal/manager/agent/service_test.go docs/superpowers/specs/2026-07-15-agent-configurability-detection-design.md docs/superpowers/plans/2026-07-15-agent-configurability-detection.md`

Expected: only issue-related files are changed and `git diff --check` reports no errors.
