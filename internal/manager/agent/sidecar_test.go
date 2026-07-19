package agent

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"testing"
)

func TestSidecarIsCanonicalPrivateAndPreservesUnselectedAgents(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
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
		StateDir: filepath.Join(home, "state"), Detector: testServiceDetector(home, map[string]bool{"claude": true}, nil), LegacyRenderInput: input,
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

func TestJournalOrdersManagerStateLastAndRollbackRestoresItFirst(t *testing.T) {
	home := t.TempDir()
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
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
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
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
			service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true}, nil)
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
	service := newTestService(t, filepath.Join(home, "state"), home, map[string]bool{"claude": true, "opencode": true}, nil)
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
	service := newTestService(t, stateDir, home, map[string]bool{"claude": true}, nil)
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
	recovered, err := NewService(Options{StateDir: stateDir, Detector: testServiceDetector(home, map[string]bool{"claude": true}, nil), LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	if recovered.WritesDisabled() || readString(t, claudePath) != previousAgent || readString(t, recovered.sidecarPath()) != previousSidecar {
		t.Fatalf("recovery split Agent/state ownership: disabled=%t agent=%q state=%q", recovered.WritesDisabled(), readString(t, claudePath), readString(t, recovered.sidecarPath()))
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
