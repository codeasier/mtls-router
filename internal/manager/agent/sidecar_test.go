package agent

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"sort"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

func TestSidecarOwnershipReflectsActualMerge(t *testing.T) {
	home := t.TempDir()
	writeFile(t, filepath.Join(home, ".config", "opencode", "opencode.json"), `{"model":"user/default"}`)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), "model_verbosity = \"user\"\n")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode, Codex})

	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	if slices.Contains(state.Agents[OpenCode].OwnedPaths, "model") {
		t.Fatal("preserved user root model was recorded as manager-owned")
	}
	if slices.Contains(state.Agents[Codex].OwnedPaths, "model_verbosity") {
		t.Fatal("absent Codex optional field was recorded as manager-owned")
	}
}

func TestSidecarPreservesMatchingOpenCodeModelOwnershipAcrossCompatibilityWrites(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})
	writeV2Legacy(t, service, []Kind{OpenCode})

	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Contains(state.Agents[OpenCode].OwnedPaths, "model") {
		t.Fatal("unchanged manager-written root model ownership was dropped")
	}
}

func TestSidecarPreservesOpenCodeModelOwnershipAcrossUnrelatedDrift(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})

	root, valid := decodeObject([]byte(readString(t, path)))
	if !valid {
		t.Fatal("first compatibility write produced invalid JSON")
	}
	root["theme"] = json.RawMessage(`"user"`)
	drifted, err := marshalObject(root)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, drifted, 0o600); err != nil {
		t.Fatal(err)
	}

	writeV2Legacy(t, service, []Kind{OpenCode})
	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Contains(state.Agents[OpenCode].OwnedPaths, "model") {
		t.Fatal("unrelated drift dropped manager-owned root model")
	}
	root, _ = decodeObject([]byte(readString(t, path)))
	if jsonString(t, root["theme"]) != "user" {
		t.Fatalf("unrelated drift was not preserved: %s", readString(t, path))
	}
}

func TestSidecarDropsDriftedOpenCodeModelOwnershipOnCompatibilityWrite(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})

	root, valid := decodeObject([]byte(readString(t, path)))
	if !valid {
		t.Fatal("first compatibility write produced invalid JSON")
	}
	root["model"] = json.RawMessage(`"user/default"`)
	drifted, err := marshalObject(root)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, drifted, 0o600); err != nil {
		t.Fatal(err)
	}

	writeV2Legacy(t, service, []Kind{OpenCode})
	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	if slices.Contains(state.Agents[OpenCode].OwnedPaths, "model") {
		t.Fatal("drifted user root model remained manager-owned")
	}
	root, _ = decodeObject([]byte(readString(t, path)))
	if jsonString(t, root["model"]) != "user/default" {
		t.Fatalf("drifted user root model was replaced: %s", readString(t, path))
	}
}

func TestSidecarValidationAcceptsLegacyBroadOwnership(t *testing.T) {
	state := lastAppliedState{
		Version: 1, KeyGeneration: "generation", Agents: map[Kind]lastAppliedAgent{
			OpenCode: {
				ModelConfig: json.RawMessage(`{"default_model":"managed","models":{"managed":{}}}`),
				Files:       []lastAppliedFile{{Role: "config", Path: "/config", RevisionMAC: "revision"}},
				OwnedPaths:  []string{"model", "provider.mtls-router"},
			},
			Codex: {
				ModelConfig: json.RawMessage(`{"model":"managed"}`),
				Files: []lastAppliedFile{
					{Role: "config", Path: "/config.toml", RevisionMAC: "config-revision"},
					{Role: "auth", Path: "/auth.json", RevisionMAC: "auth-revision"},
				},
				OwnedPaths: []string{"auth.OPENAI_API_KEY", "auth.auth_mode", "cli_auth_credentials_store", "model", "model_context_window", "model_provider", "model_providers.mtls-router", "model_verbosity"},
			},
		},
	}
	if err := validateSidecarState(state, "generation"); err != nil {
		t.Fatalf("legacy sidecar rejected: %v", err)
	}
}

func TestSidecarIsCanonicalPrivateAndPreservesUnselectedAgents(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{ClaudeCode, OpenCode})
	path := service.sidecarPath()
	first, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(first), testAPIKey) || strings.Contains(string(first), "model-primary\"") && strings.Contains(string(first), "catalog_token") {
		t.Fatalf("sidecar contains prohibited data: %s", first)
	}
	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Agents) != 2 || state.Agents[ClaudeCode].Files[0].RevisionMAC == "" {
		t.Fatalf("state = %#v", state)
	}
	if runtime.GOOS != "windows" {
		info, _ := os.Stat(path)
		if info.Mode().Perm()&0o077 != 0 {
			t.Fatalf("mode = %04o", info.Mode().Perm())
		}
	}

	writeV2Legacy(t, service, []Kind{ClaudeCode})
	state, _, second, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Agents) != 2 || state.Agents[OpenCode].ModelConfig == nil {
		t.Fatalf("unselected state lost: %#v", state)
	}
	var decoded any
	if json.Unmarshal(second, &decoded) != nil {
		t.Fatal("sidecar is not JSON")
	}
}

func TestClaudeBudgetOwnershipFollowsTypedConfiguration(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"manual-context","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"manual-output"}}`)
	contextWindow, maxOutput := int64(353400), int64(100000)
	input := legacyTestRenderInput()
	service, err := NewService(Options{
		StateDir: filepath.Join(home, "state"), Detector: testServiceDetector(home, nil), LegacyRenderInput: input,
	})
	if err != nil {
		t.Fatal(err)
	}
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	if preview.ManagedConfigDrift {
		t.Fatalf("unconfigured manual budgets reported as collision: %#v", preview.ManagedCollisions)
	}
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	owned := state.Agents[ClaudeCode].OwnedPaths
	want := []string{"env.CLAUDE_CODE_MAX_CONTEXT_TOKENS", "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS"}
	for _, path := range want {
		index := sort.SearchStrings(owned, path)
		if index < len(owned) && owned[index] == path {
			t.Fatalf("unconfigured budget path is owned %q: %#v", path, owned)
		}
	}
	content := readString(t, path)
	if !strings.Contains(content, "manual-context") || !strings.Contains(content, "manual-output") {
		t.Fatalf("manual budgets were not preserved: %s", content)
	}

	input.Config.Claude.ContextWindow = &contextWindow
	input.Config.Claude.MaxOutputTokens = &maxOutput
	preview, err = service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	_, err = service.Write(context.Background(), WriteRequest{
		Agents:                  []Kind{ClaudeCode},
		RevisionToken:           preview.RevisionToken,
		APIKey:                  testAPIKey,
		ApproveManagedOverwrite: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	state, _, _, _, err = service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	owned = state.Agents[ClaudeCode].OwnedPaths
	for _, ownedPath := range want {
		index := sort.SearchStrings(owned, ownedPath)
		if index == len(owned) || owned[index] != ownedPath {
			t.Fatalf("configured budget path is not owned %q: %#v", ownedPath, owned)
		}
	}
	content = readString(t, path)
	if !strings.Contains(content, `"353400"`) || !strings.Contains(content, `"100000"`) {
		t.Fatalf("configured budgets were not rendered: %s", content)
	}

	input.Config.Claude.ContextWindow = nil
	input.Config.Claude.MaxOutputTokens = nil
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	state, _, _, _, err = service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	owned = state.Agents[ClaudeCode].OwnedPaths
	for _, ownedPath := range want {
		index := sort.SearchStrings(owned, ownedPath)
		if index < len(owned) && owned[index] == ownedPath {
			t.Fatalf("omitted budget path remains owned %q: %#v", ownedPath, owned)
		}
	}
	content = readString(t, path)
	if strings.Contains(content, "CLAUDE_CODE_MAX_CONTEXT_TOKENS") || strings.Contains(content, "CLAUDE_CODE_MAX_OUTPUT_TOKENS") {
		t.Fatalf("previously owned omitted budgets survived: %s", content)
	}
}

func TestClaudeFableOwnershipFollowsOptionalConfiguration(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	writeFile(t, path, `{"theme":"dark","env":{"ANTHROPIC_DEFAULT_FABLE_MODEL":"manual","ANTHROPIC_DEFAULT_FABLE_MODEL_NAME":"Manual","UNRELATED":"keep"}}`)
	input := legacyTestRenderInput()
	service, err := NewService(Options{
		StateDir: filepath.Join(home, "state"), Detector: testServiceDetector(home, nil), LegacyRenderInput: input,
	})
	if err != nil {
		t.Fatal(err)
	}
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	if preview.ManagedConfigDrift {
		t.Fatalf("disabled manual Fable reported as collision: %#v", preview.ManagedCollisions)
	}
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	content := readString(t, path)
	root, _ := decodeObject([]byte(content))
	env, _ := decodeObject(root["env"])
	if jsonString(t, env["ANTHROPIC_DEFAULT_FABLE_MODEL"]) != "manual" || jsonString(t, env["UNRELATED"]) != "keep" {
		t.Fatalf("disabled Fable changed manual values: %s", content)
	}

	fableName := "Managed Fable"
	input.Config.Claude.Fable = &modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "model-sonnet", Name: &fableName}}
	preview, err = service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	if !preview.ManagedConfigDrift || len(preview.ManagedCollisions) != 2 || preview.ManagedCollisions[0].Path != "/env/ANTHROPIC_DEFAULT_FABLE_MODEL" || preview.ManagedCollisions[1].Path != "/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME" {
		t.Fatalf("enabled Fable collision = %#v", preview)
	}
	_, err = service.Write(context.Background(), WriteRequest{
		Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey, ApproveManagedOverwrite: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	state, _, _, _, err := service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	owned := strings.Join(state.Agents[ClaudeCode].OwnedPaths, ",")
	if !strings.Contains(owned, "env.ANTHROPIC_DEFAULT_FABLE_MODEL") || !strings.Contains(owned, "env.ANTHROPIC_DEFAULT_FABLE_MODEL_NAME") {
		t.Fatalf("Fable paths not owned: %s", owned)
	}
	content = readString(t, path)
	root, _ = decodeObject([]byte(content))
	env, _ = decodeObject(root["env"])
	if jsonString(t, env["ANTHROPIC_DEFAULT_FABLE_MODEL"]) != "model-sonnet" || jsonString(t, env["ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"]) != fableName || jsonString(t, env["UNRELATED"]) != "keep" {
		t.Fatalf("enabled Fable write = %s", content)
	}

	input.Config.Claude.Fable = &modelconfig.ClaudeRole{Selection: &modelconfig.Model{Model: "model-sonnet"}}
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	content = readString(t, path)
	root, _ = decodeObject([]byte(content))
	env, _ = decodeObject(root["env"])
	if env["ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"] != nil || jsonString(t, env["ANTHROPIC_DEFAULT_FABLE_MODEL"]) != "model-sonnet" {
		t.Fatalf("named-to-nameless Fable left stale name: %s", content)
	}

	input.Config.Claude.Fable = nil
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	state, _, _, _, err = service.readSidecar()
	if err != nil {
		t.Fatal(err)
	}
	owned = strings.Join(state.Agents[ClaudeCode].OwnedPaths, ",")
	content = readString(t, path)
	root, _ = decodeObject([]byte(content))
	env, _ = decodeObject(root["env"])
	if strings.Contains(owned, "FABLE") || env["ANTHROPIC_DEFAULT_FABLE_MODEL"] != nil || env["ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"] != nil || jsonString(t, env["UNRELATED"]) != "keep" || jsonString(t, root["theme"]) != "dark" {
		t.Fatalf("disabled stale cleanup split ownership or data: owned=%s content=%s", owned, content)
	}
}

func TestJournalOrdersManagerStateLastAndRollbackRestoresItFirst(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	originalState := readString(t, service.sidecarPath())
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	seen := false
	service.hooks.afterReplace = func(path string) {
		if path != service.sidecarPath() {
			return
		}
		journal, err := decodeJournal(service.journalPath())
		if err != nil {
			t.Fatal(err)
		}
		if journal.Entries[len(journal.Entries)-1].Scope != scopeManagerState {
			t.Fatalf("journal order = %#v", journal.Entries)
		}
		seen = true
	}
	service.hooks.beforeReplace = func(path string) error {
		if path == service.sidecarPath() {
			return os.ErrPermission
		}
		return nil
	}
	_, err = service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey, ApproveManagedOverwrite: false})
	assertCode(t, err, CodeWriteFailed)
	if got := readString(t, service.sidecarPath()); got != originalState {
		t.Fatal("sidecar changed after rollback")
	}
	if seen {
		t.Fatal("sidecar should not have been replaced")
	}
}

func TestCorruptSidecarFailsPreviewAndWriteWithoutArtifacts(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	writeFile(t, service.sidecarPath(), `{"version":1,"version":1}`)
	_, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	assertCode(t, err, CodeModelStateInvalid)
	assertNoBackupFiles(t, home)
	if _, err := os.Stat(service.journalPath()); !os.IsNotExist(err) {
		t.Fatalf("journal exists: %v", err)
	}
}

func TestSidecarCorruptionAndPermissionMatrixFailsClosedWithoutArtifacts(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("this test asserts POSIX mode bits; Windows ACL behavior requires a Windows runtime")
	}
	for _, test := range []struct {
		name    string
		content string
		mode    os.FileMode
	}{
		{name: "empty", content: "", mode: 0o600},
		{name: "malformed", content: `{`, mode: 0o600},
		{name: "duplicate key", content: `{"version":1,"version":1}`, mode: 0o600},
		{name: "unknown field", content: `{"agents":{},"key_generation":"x","unknown":true,"version":1}`, mode: 0o600},
		{name: "oversized", content: strings.Repeat("x", maxSidecarSize+1), mode: 0o600},
		{name: "unsafe permissions", content: `{}`, mode: 0o644},
	} {
		t.Run(test.name, func(t *testing.T) {
			home := t.TempDir()
			service := newTestService(t, filepath.Join(home, "state"), home, nil)
			writeV2Legacy(t, service, []Kind{ClaudeCode})
			if err := os.WriteFile(service.sidecarPath(), []byte(test.content), test.mode); err != nil {
				t.Fatal(err)
			}
			if err := os.Chmod(service.sidecarPath(), test.mode); err != nil {
				t.Fatal(err)
			}
			_, err := service.Preview(context.Background(), []Kind{ClaudeCode})
			assertCode(t, err, CodeModelStateInvalid)
			assertNoBackupFiles(t, home)
			if _, err := os.Stat(service.journalPath()); !os.IsNotExist(err) {
				t.Fatalf("journal exists after rejected sidecar: %v", err)
			}
		})
	}
}

func TestSidecarAndJournalExcludeProhibitedDataAtEveryReplacePoint(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode, OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	prohibited := []string{testAPIKey, "Authorization", "Bearer ", "catalog_token", "raw_response", "rendered_content", "upstream"}
	checked := 0
	service.hooks.afterReplace = func(string) {
		checked++
		for _, path := range []string{service.journalPath(), service.sidecarPath()} {
			content, readErr := os.ReadFile(path)
			if readErr != nil {
				if os.IsNotExist(readErr) {
					continue
				}
				t.Fatal(readErr)
			}
			for _, value := range prohibited {
				if strings.Contains(string(content), value) {
					t.Fatalf("%s contains prohibited value %q: %s", path, value, content)
				}
			}
		}
	}
	if _, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode, OpenCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey}); err != nil {
		t.Fatal(err)
	}
	if checked != 3 {
		t.Fatalf("replace checkpoints = %d, want 3 including sidecar-last", checked)
	}
}

func TestCrashAfterSidecarReplacementRecoversFilesAndStateTogether(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	claudePath := filepath.Join(home, ".claude", "settings.json")
	original := `{"sentinel":"agent-before"}`
	writeFile(t, claudePath, original)
	service := newTestService(t, stateDir, home, nil)
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	previousAgent := readString(t, claudePath)
	previousSidecar := readString(t, service.sidecarPath())
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.afterReplace = func(path string) {
		if path == service.sidecarPath() {
			panic("simulated crash after sidecar-last replacement")
		}
	}
	func() {
		defer func() {
			if recover() == nil {
				t.Fatal("expected crash after sidecar replacement")
			}
		}()
		_, _ = service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey})
	}()
	journal, err := decodeJournal(service.journalPath())
	if err != nil {
		t.Fatal(err)
	}
	if journal.Entries[len(journal.Entries)-1].Scope != scopeManagerState || journal.Entries[len(journal.Entries)-1].Progress != progressReplaced {
		t.Fatalf("crash journal does not record sidecar last: %#v", journal.Entries)
	}
	recovered, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, nil), LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	if recovered.WritesDisabled() || readString(t, claudePath) != previousAgent || readString(t, recovered.sidecarPath()) != previousSidecar {
		t.Fatalf("recovery split Agent/state ownership: disabled=%t agent=%q state=%q", recovered.WritesDisabled(), readString(t, claudePath), readString(t, recovered.sidecarPath()))
	}
}

func TestDeleteCrashRecoveryRestoresStateFirst(t *testing.T) {
	for _, crashAfter := range []journalScope{scopeAgent, scopeManagerState} {
		t.Run(string(crashAfter), func(t *testing.T) {
			home := t.TempDir()
			stateDir := filepath.Join(home, "state")
			agentPath := filepath.Join(home, ".config", "opencode", "opencode.json")
			statePath := filepath.Join(home, "managed-state.json")
			agentContent := []byte(`{"provider":{"mtls-router":{}}}`)
			stateContent := []byte(`{"version":1}`)
			writeFile(t, agentPath, string(agentContent))
			writeFile(t, statePath, string(stateContent))
			service := newTestService(t, stateDir, home, nil)
			if err := service.ensureSigner(); err != nil {
				t.Fatal(err)
			}
			agentEntry := deleteJournalEntry(t, service, OpenCode, scopeAgent, agentPath, agentContent)
			stateEntry := deleteJournalEntry(t, service, "", scopeManagerState, statePath, stateContent)
			journal := transactionJournal{Version: 3, KeyGeneration: service.keyGeneration, TransactionID: "delete-crash", Entries: []journalEntry{agentEntry, stateEntry}}
			if err := service.prepareStateDir(); err != nil {
				t.Fatal(err)
			}
			if err := service.writeJournal(journal); err != nil {
				t.Fatal(err)
			}
			if journal.Entries[0].Scope != scopeAgent || journal.Entries[1].Scope != scopeManagerState {
				t.Fatalf("journal order = %#v", journal.Entries)
			}
			for i := range journal.Entries {
				entry := &journal.Entries[i]
				if _, err := service.applyPlannedFile(plannedFile{targetPath: entry.TargetPath, operation: OperationDelete}, nil); err != nil {
					t.Fatal(err)
				}
				entry.Progress = progressReplaced
				if err := service.writeJournal(journal); err != nil {
					t.Fatal(err)
				}
				if entry.Scope == crashAfter {
					break
				}
			}

			var rollbackOrder []string
			service.hooks.beforeRollback = func(path string) error {
				rollbackOrder = append(rollbackOrder, path)
				return nil
			}
			if err := service.recoverLocked(context.Background()); err != nil {
				t.Fatal(err)
			}
			if got := readString(t, agentPath); got != string(agentContent) {
				t.Fatalf("Agent file not restored: %q", got)
			}
			if got := readString(t, statePath); got != string(stateContent) {
				t.Fatalf("manager state not restored: %q", got)
			}
			if crashAfter == scopeManagerState && (len(rollbackOrder) != 2 || rollbackOrder[0] != statePath || rollbackOrder[1] != agentPath) {
				t.Fatalf("rollback order = %#v", rollbackOrder)
			}
		})
	}
}

func TestStartupRecoversDeleteAfterAgentAndSidecar(t *testing.T) {
	for _, crashAfter := range []journalScope{scopeAgent, scopeManagerState} {
		t.Run(string(crashAfter), func(t *testing.T) {
			home := t.TempDir()
			stateDir := filepath.Join(home, "state")
			agentPath := filepath.Join(home, ".config", "opencode", "opencode.json")
			agentContent := []byte(`{"provider":{"mtls-router":{}}}`)
			stateContent := []byte(`{"version":1}`)
			writeFile(t, agentPath, string(agentContent))
			service := newTestService(t, stateDir, home, nil)
			if err := service.ensureSigner(); err != nil {
				t.Fatal(err)
			}
			writeFile(t, service.sidecarPath(), string(stateContent))
			journal := transactionJournal{Version: 3, KeyGeneration: service.keyGeneration, TransactionID: "startup-delete", Entries: []journalEntry{
				deleteJournalEntry(t, service, OpenCode, scopeAgent, agentPath, agentContent),
				deleteJournalEntry(t, service, "", scopeManagerState, service.sidecarPath(), stateContent),
			}}
			if err := service.writeJournal(journal); err != nil {
				t.Fatal(err)
			}
			for i := range journal.Entries {
				if _, err := service.applyPlannedFile(plannedFile{targetPath: journal.Entries[i].TargetPath, operation: OperationDelete}, nil); err != nil {
					t.Fatal(err)
				}
				journal.Entries[i].Progress = progressReplaced
				if err := service.writeJournal(journal); err != nil {
					t.Fatal(err)
				}
				if journal.Entries[i].Scope == crashAfter {
					break
				}
			}

			recovered, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, nil), LegacyRenderInput: legacyTestRenderInput()})
			if err != nil {
				t.Fatal(err)
			}
			if recovered.WritesDisabled() || readString(t, agentPath) != string(agentContent) || readString(t, recovered.sidecarPath()) != string(stateContent) {
				t.Fatalf("startup delete recovery split state: disabled=%t agent=%q state=%q", recovered.WritesDisabled(), readString(t, agentPath), readString(t, recovered.sidecarPath()))
			}
			if _, err := os.Stat(recovered.journalPath()); !os.IsNotExist(err) {
				t.Fatalf("journal retained after startup recovery: %v", err)
			}
		})
	}
}

func deleteJournalEntry(t *testing.T, service *Service, kind Kind, scope journalScope, path string, content []byte) journalEntry {
	t.Helper()
	preRevision, _, mode, err := service.readKeyedRevision(path, revisionContextJournal)
	if err != nil {
		t.Fatal(err)
	}
	backupPath, err := createPrivateBackup(path, content, mode, "bak")
	if err != nil {
		t.Fatal(err)
	}
	backupRevision, err := service.keyedRevisionForContent(content, mode, revisionContextBackup, backupPath)
	if err != nil {
		t.Fatal(err)
	}
	return journalEntry{
		Scope: scope, Agent: kind, TargetPath: path, Operation: OperationDelete,
		PreRevision: preRevision, BackupPath: backupPath, BackupRevision: backupRevision,
		RestoreFrom: path, TargetMode: uint32(mode.Perm()), Progress: progressPending,
	}
}

func writeV2Legacy(t *testing.T, service *Service, selected []Kind) {
	t.Helper()
	preview, err := service.Preview(context.Background(), selected)
	if err != nil {
		t.Fatal(err)
	}
	_, err = service.Write(context.Background(), WriteRequest{Agents: selected, RevisionToken: preview.RevisionToken, APIKey: testAPIKey, ApproveManagedOverwrite: preview.ManagedConfigDrift, ApproveCodexAuthChange: preview.RequiresCodexAuthApproval})
	if err != nil {
		t.Fatal(err)
	}
}
