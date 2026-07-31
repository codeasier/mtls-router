package agent

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

func TestCleanupServicePreviewAndWrite(t *testing.T) {
	t.Run("opencode final managed Agent deletes sidecar", func(t *testing.T) {
		home := t.TempDir()
		path := filepath.Join(home, ".config", "opencode", "opencode.json")
		writeFile(t, path, `{"theme":"keep"}`)
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		backupsBeforePreview := backupFileCount(t, home)

		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		if err != nil {
			t.Fatal(err)
		}
		if preview.Agent != OpenCode || len(preview.Files) != 1 || preview.RevisionToken == "" || preview.ManagedConfigDrift {
			t.Fatalf("cleanup preview = %#v", preview)
		}
		if !reflect.DeepEqual(preview.RemovedPaths, []string{"model", "provider.mtls-router"}) {
			t.Fatalf("removed paths = %#v", preview.RemovedPaths)
		}
		if preview.StateChange == nil || preview.StateChange.Operation != OperationDelete || preview.StateBackup == nil {
			t.Fatalf("state effects = change %#v backup %#v", preview.StateChange, preview.StateBackup)
		}
		for _, file := range append(append([]FilePreview{}, preview.Files...), *preview.StateChange, *preview.StateBackup) {
			if len(file.Preserves) != 0 || file.Warning != "" || file.Backup.Warning != "" {
				t.Fatalf("cleanup preview contains manager prose: %#v", file)
			}
		}
		if got := backupFileCount(t, home); got != backupsBeforePreview {
			t.Fatalf("cleanup preview created backup files: before=%d after=%d", backupsBeforePreview, got)
		}

		result, err := service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken})
		if err != nil || len(result.Agents) != 1 || !result.Agents[0].Success {
			t.Fatalf("cleanup write = %#v, %v", result, err)
		}
		if result.StateChange == nil || result.StateChange.Operation != OperationDelete || result.StateBackup == nil || result.StateBackup.Operation != OperationBackup {
			t.Fatalf("cleanup result state effects = change %#v backup %#v", result.StateChange, result.StateBackup)
		}
		if strings.Contains(readString(t, path), "mtls-router") || !strings.Contains(readString(t, path), `"theme":"keep"`) {
			t.Fatalf("cleaned opencode config = %s", readString(t, path))
		}
		if _, err := os.Stat(service.sidecarPath()); !os.IsNotExist(err) {
			t.Fatalf("final sidecar still exists: %v", err)
		}
	})

	t.Run("codex pair preserves another managed Agent", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{ClaudeCode, Codex})

		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: Codex})
		if err != nil {
			t.Fatal(err)
		}
		if len(preview.Files) != 2 || preview.StateChange == nil || preview.StateChange.Operation != OperationReplace {
			t.Fatalf("Codex cleanup preview = %#v", preview)
		}
		result, err := service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: Codex, RevisionToken: preview.RevisionToken})
		if err != nil || !result.Agents[0].Success {
			t.Fatalf("Codex cleanup write = %#v, %v", result, err)
		}
		if result.StateChange == nil || result.StateChange.Operation != OperationReplace {
			t.Fatalf("Codex cleanup state result = %#v", result.StateChange)
		}
		for _, path := range []string{filepath.Join(home, ".codex", "config.toml"), filepath.Join(home, ".codex", "auth.json")} {
			if _, err := os.Stat(path); !os.IsNotExist(err) {
				t.Fatalf("empty managed Codex file still exists at %s: %v", path, err)
			}
		}
		state, _, _, _, err := service.readSidecar()
		if err != nil || len(state.Agents) != 1 || state.Agents[ClaudeCode].ModelConfig == nil {
			t.Fatalf("preserved sidecar = %#v, %v", state, err)
		}
	})
}

func TestCleanupServiceUsesRecordedPaths(t *testing.T) {
	home := t.TempDir()
	recordedDir := filepath.Join(home, "recorded-claude")
	env := map[string]string{"CLAUDE_CONFIG_DIR": recordedDir}
	service := newTestService(t, filepath.Join(home, "state"), home, env)
	writeV2Legacy(t, service, []Kind{ClaudeCode})
	recordedPath := filepath.Join(recordedDir, "settings.json")
	env["CLAUDE_CONFIG_DIR"] = filepath.Join(home, "different-claude")
	differentPath := filepath.Join(env["CLAUDE_CONFIG_DIR"], "settings.json")
	writeFile(t, differentPath, `{"keep":true}`)

	preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	if len(preview.Files) != 1 || preview.Files[0].Path != recordedPath {
		t.Fatalf("cleanup paths = %#v", preview.Files)
	}
	if _, err := service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: ClaudeCode, RevisionToken: preview.RevisionToken}); err != nil {
		t.Fatal(err)
	}
	if readString(t, differentPath) != `{"keep":true}` {
		t.Fatal("cleanup touched a path derived from the current environment")
	}
}

func TestCleanupServiceRejectsUnmanagedAndInvalidState(t *testing.T) {
	t.Run("not managed", func(t *testing.T) {
		home := t.TempDir()
		stateDir := filepath.Join(home, "state")
		service := &Service{stateDir: stateDir, detector: testServiceDetector(home, nil)}
		_, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		assertCode(t, err, CodeAgentNotManaged)
		assertCleanupStateAbsent(t, stateDir)
	})

	t.Run("write not managed", func(t *testing.T) {
		home := t.TempDir()
		stateDir := filepath.Join(home, "state")
		service := &Service{stateDir: stateDir, detector: testServiceDetector(home, nil)}
		_, err := service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: "invalid-token"})
		assertCode(t, err, CodeAgentNotManaged)
		assertCleanupStateAbsent(t, stateDir)
		if _, statErr := os.Stat(filepath.Join(stateDir, lockFileName)); !os.IsNotExist(statErr) {
			t.Fatalf("unmanaged cleanup write created lock state: %v", statErr)
		}
	})

	t.Run("Agent absent from valid sidecar", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{ClaudeCode})
		_, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		assertCode(t, err, CodeAgentNotManaged)
	})

	t.Run("invalid sidecar", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		if err := os.WriteFile(service.sidecarPath(), []byte(`{}`), 0o600); err != nil {
			t.Fatal(err)
		}
		_, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		assertCode(t, err, CodeModelStateInvalid)
	})

	t.Run("missing signer is not recreated", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		keyPath := filepath.Join(service.stateDir, signingKeyFileName)
		if err := os.Remove(keyPath); err != nil {
			t.Fatal(err)
		}
		_, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		assertCode(t, err, CodeModelStateInvalid)
		if _, statErr := os.Stat(keyPath); !os.IsNotExist(statErr) {
			t.Fatalf("cleanup recreated signer: %v", statErr)
		}
	})
}

func TestCleanupPreviewUsesOnlyExistingCoordinationLock(t *testing.T) {
	t.Run("managed sidecar without lock fails without artifacts", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		if err := os.Remove(filepath.Join(service.stateDir, lockFileName)); err != nil {
			t.Fatal(err)
		}
		entriesBefore := stateDirEntryNames(t, service.stateDir)
		backupsBefore := backupFileCount(t, home)

		_, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		assertCode(t, err, CodeModelStateInvalid)
		if got := stateDirEntryNames(t, service.stateDir); !reflect.DeepEqual(got, entriesBefore) {
			t.Fatalf("cleanup preview changed state artifacts: before=%#v after=%#v", entriesBefore, got)
		}
		if got := backupFileCount(t, home); got != backupsBefore {
			t.Fatalf("cleanup preview created backups: before=%d after=%d", backupsBefore, got)
		}
		if _, statErr := os.Stat(filepath.Join(service.stateDir, lockFileName)); !os.IsNotExist(statErr) {
			t.Fatalf("cleanup preview recreated missing lock: %v", statErr)
		}
	})

	t.Run("managed sidecar opens existing lock", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		entriesBefore := stateDirEntryNames(t, service.stateDir)
		backupsBefore := backupFileCount(t, home)

		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		if err != nil || preview.RevisionToken == "" {
			t.Fatalf("cleanup preview = %#v, %v", preview, err)
		}
		if got := stateDirEntryNames(t, service.stateDir); !reflect.DeepEqual(got, entriesBefore) {
			t.Fatalf("cleanup preview changed state artifacts: before=%#v after=%#v", entriesBefore, got)
		}
		if got := backupFileCount(t, home); got != backupsBefore {
			t.Fatalf("cleanup preview created backups: before=%d after=%d", backupsBefore, got)
		}
	})
}

func TestCleanupServiceDriftApproval(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})
	root := readJSONObject(t, path)
	root["theme"] = jsonRaw(`"drifted"`)
	content, err := marshalObject(root)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, content, 0o600); err != nil {
		t.Fatal(err)
	}

	preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
	if err != nil || !preview.ManagedConfigDrift {
		t.Fatalf("drift preview = %#v, %v", preview, err)
	}
	_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken})
	assertCode(t, err, CodeManagedConfigDrift)
	result, err := service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken, ApproveManagedOverwrite: true})
	if err != nil || !result.Agents[0].Success || !strings.Contains(readString(t, path), "drifted") {
		t.Fatalf("approved drift cleanup = %#v, %v, content %s", result, err, readString(t, path))
	}
}

func TestCleanupServiceRejectsStalePreview(t *testing.T) {
	t.Run("Agent file", func(t *testing.T) {
		home := t.TempDir()
		path := filepath.Join(home, ".config", "opencode", "opencode.json")
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, append([]byte(readString(t, path)), '\n'), 0o600); err != nil {
			t.Fatal(err)
		}
		_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken})
		assertCode(t, err, CodePreviewStale)
	})

	t.Run("sidecar", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{OpenCode})
		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(service.sidecarPath(), append([]byte(readString(t, service.sidecarPath())), '\n'), 0o600); err != nil {
			t.Fatal(err)
		}
		_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken})
		assertCode(t, err, CodePreviewStale)
	})
}

func TestCleanupServiceBackupFailureMakesNoMutations(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})
	beforeConfig := readString(t, path)
	beforeState := readString(t, service.sidecarPath())
	preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeBackup = func(string) error { return errors.New("fail") }
	_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken})
	assertCode(t, err, CodeBackupFailed)
	if readString(t, path) != beforeConfig || readString(t, service.sidecarPath()) != beforeState {
		t.Fatal("backup failure mutated cleanup targets")
	}
}

func TestCleanupServiceRevalidatesImmediatelyBeforeBackup(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})
	stateBefore := readString(t, service.sidecarPath())
	preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
	if err != nil {
		t.Fatal(err)
	}
	backupsBefore := backupFileCount(t, home)
	mutated := readString(t, path) + "\n"
	service.hooks.beforeBackup = func(backupSource string) error {
		if backupSource == path {
			return os.WriteFile(path, []byte(mutated), 0o600)
		}
		return nil
	}

	_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: OpenCode, RevisionToken: preview.RevisionToken})
	assertCode(t, err, CodePreviewStale)
	if got := backupFileCount(t, home); got != backupsBefore {
		t.Fatalf("stale cleanup created backup files: before=%d after=%d", backupsBefore, got)
	}
	if readString(t, path) != mutated || readString(t, service.sidecarPath()) != stateBefore {
		t.Fatal("stale cleanup mutated a transaction target")
	}
	if _, statErr := os.Stat(service.journalPath()); !os.IsNotExist(statErr) {
		t.Fatalf("stale cleanup created a journal: %v", statErr)
	}
}

func TestCleanupServiceCleansEarlierCodexBackupsBeforeJournal(t *testing.T) {
	t.Run("later file stale", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{Codex})
		configPath := filepath.Join(home, ".codex", "config.toml")
		authPath := filepath.Join(home, ".codex", "auth.json")
		configBefore := readString(t, configPath)
		authBefore := readString(t, authPath)
		stateBefore := readString(t, service.sidecarPath())
		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: Codex})
		if err != nil {
			t.Fatal(err)
		}
		backupsBefore := backupFileCount(t, home)
		mutatedAuth := authBefore + "\n"
		service.hooks.beforeBackup = func(backupSource string) error {
			if backupSource == authPath {
				return os.WriteFile(authPath, []byte(mutatedAuth), 0o600)
			}
			return nil
		}

		_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: Codex, RevisionToken: preview.RevisionToken})
		assertCode(t, err, CodePreviewStale)
		assertCleanupAttemptLeftNoArtifacts(t, service, home, backupsBefore)
		if readString(t, configPath) != configBefore || readString(t, authPath) != mutatedAuth || readString(t, service.sidecarPath()) != stateBefore {
			t.Fatal("later-file stale cleanup mutated a transaction target")
		}
	})

	t.Run("cancellation after first backup", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{Codex})
		configPath := filepath.Join(home, ".codex", "config.toml")
		authPath := filepath.Join(home, ".codex", "auth.json")
		configBefore := readString(t, configPath)
		authBefore := readString(t, authPath)
		stateBefore := readString(t, service.sidecarPath())
		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: Codex})
		if err != nil {
			t.Fatal(err)
		}
		backupsBefore := backupFileCount(t, home)
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()
		cancelled := false
		service.hooks.backupStage = func(stage backupStage, backupPath string) error {
			if !cancelled && stage == backupStageContent && strings.HasPrefix(filepath.Base(backupPath), filepath.Base(configPath)+".bak-") {
				cancelled = true
				cancel()
			}
			return nil
		}

		_, err = service.CleanupWrite(ctx, CleanupWriteRequest{Agent: Codex, RevisionToken: preview.RevisionToken})
		assertCode(t, err, CodeOperationTimeout)
		if !cancelled {
			t.Fatal("test did not cancel after the first Codex backup")
		}
		assertCleanupAttemptLeftNoArtifacts(t, service, home, backupsBefore)
		if readString(t, configPath) != configBefore || readString(t, authPath) != authBefore || readString(t, service.sidecarPath()) != stateBefore {
			t.Fatal("cancelled cleanup mutated a transaction target")
		}
	})

	t.Run("earlier file changes during later backup", func(t *testing.T) {
		home := t.TempDir()
		service := newTestService(t, filepath.Join(home, "state"), home, nil)
		writeV2Legacy(t, service, []Kind{Codex})
		configPath := filepath.Join(home, ".codex", "config.toml")
		authPath := filepath.Join(home, ".codex", "auth.json")
		configBefore := readString(t, configPath)
		authBefore := readString(t, authPath)
		stateBefore := readString(t, service.sidecarPath())
		preview, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: Codex})
		if err != nil {
			t.Fatal(err)
		}
		backupsBefore := backupFileCount(t, home)
		mutatedConfig := configBefore + "\n"
		service.hooks.beforeBackup = func(backupSource string) error {
			if backupSource == authPath {
				return os.WriteFile(configPath, []byte(mutatedConfig), 0o600)
			}
			return nil
		}

		_, err = service.CleanupWrite(context.Background(), CleanupWriteRequest{Agent: Codex, RevisionToken: preview.RevisionToken})
		assertCode(t, err, CodePreviewStale)
		assertCleanupAttemptLeftNoArtifacts(t, service, home, backupsBefore)
		if readString(t, configPath) != mutatedConfig || readString(t, authPath) != authBefore || readString(t, service.sidecarPath()) != stateBefore {
			t.Fatal("cross-file stale cleanup mutated a transaction target")
		}
	})
}

func TestCleanupServiceRejectsUnsafeRecordedPath(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".config", "opencode", "opencode.json")
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	writeV2Legacy(t, service, []Kind{OpenCode})
	target := filepath.Join(home, "unmanaged.json")
	writeFile(t, target, `{"canary":"keep"}`)
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, path); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}

	_, err := service.CleanupPreview(context.Background(), CleanupPreviewRequest{Agent: OpenCode})
	assertCode(t, err, CodeConfigInvalid)
	if readString(t, target) != `{"canary":"keep"}` {
		t.Fatal("unsafe cleanup path modified its symlink target")
	}
}

func jsonRaw(value string) json.RawMessage { return json.RawMessage(value) }

func backupFileCount(t *testing.T, root string) int {
	t.Helper()
	count := 0
	if err := filepath.WalkDir(root, func(_ string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() && strings.Contains(entry.Name(), ".bak-") {
			count++
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	return count
}

func assertCleanupStateAbsent(t *testing.T, stateDir string) {
	t.Helper()
	if _, err := os.Stat(stateDir); !os.IsNotExist(err) {
		t.Fatalf("unmanaged cleanup created state directory: %v", err)
	}
	for _, name := range []string{lockFileName, signingKeyFileName, journalFileName, sidecarFileName} {
		if _, err := os.Stat(filepath.Join(stateDir, name)); !os.IsNotExist(err) {
			t.Fatalf("unmanaged cleanup created %s: %v", name, err)
		}
	}
}

func assertCleanupAttemptLeftNoArtifacts(t *testing.T, service *Service, home string, backupsBefore int) {
	t.Helper()
	if got := backupFileCount(t, home); got != backupsBefore {
		t.Fatalf("cleanup attempt retained backups: before=%d after=%d", backupsBefore, got)
	}
	if _, err := os.Stat(service.journalPath()); !os.IsNotExist(err) {
		t.Fatalf("cleanup attempt created a journal: %v", err)
	}
}

func stateDirEntryNames(t *testing.T, stateDir string) []string {
	t.Helper()
	entries, err := os.ReadDir(stateDir)
	if err != nil {
		t.Fatal(err)
	}
	names := make([]string, len(entries))
	for i, entry := range entries {
		names[i] = entry.Name()
	}
	return names
}

func TestCleanupTransformsPreserveUnmanagedConfiguration(t *testing.T) {
	t.Run("claude", func(t *testing.T) {
		root, _ := decodeObject([]byte(`{"env":{"ANTHROPIC_BASE_URL":"managed","USER_FLAG":"keep"},"theme":"keep"}`))
		got, err := cleanupClaude(root, []string{"env.ANTHROPIC_BASE_URL"})
		if err != nil || got.Delete || string(got.Content) != "{\"env\":{\"USER_FLAG\":\"keep\"},\"theme\":\"keep\"}\n" {
			t.Fatalf("cleanupClaude() = %#v, %v", got, err)
		}
		if !reflect.DeepEqual(got.RemovedPaths, []string{"env.ANTHROPIC_BASE_URL"}) {
			t.Fatalf("removed paths = %#v", got.RemovedPaths)
		}
	})

	t.Run("opencode preserves user model", func(t *testing.T) {
		root, _ := decodeObject([]byte(`{"model":"user/default","provider":{"mtls-router":{"name":"CodeasierRouter"},"other":{"keep":true}}}`))
		got, err := cleanupOpenCode(root, true)
		if err != nil || got.Delete || bytes.Contains(got.Content, []byte("mtls-router")) || !bytes.Contains(got.Content, []byte("user/default")) {
			t.Fatalf("cleanupOpenCode() = %s, delete=%t, err=%v", got.Content, got.Delete, err)
		}
		if !reflect.DeepEqual(got.RemovedPaths, []string{"provider.mtls-router"}) {
			t.Fatalf("removed paths = %#v", got.RemovedPaths)
		}
	})

	t.Run("codex narrows old broad ownership", func(t *testing.T) {
		content := []byte("model_provider = \"mtls-router\"\nmodel = \"managed\"\nmodel_verbosity = \"high\"\n[model_providers.mtls-router]\nname = \"CodeasierRouter\"\n")
		got, err := cleanupCodexConfig(content, &modelconfig.CodexConfig{Model: "managed"})
		if err != nil || got.Delete || !bytes.Contains(got.Content, []byte("model_verbosity")) {
			t.Fatalf("cleanupCodexConfig() = %s, delete=%t, err=%v", got.Content, got.Delete, err)
		}
	})

	t.Run("codex auth preserves metadata", func(t *testing.T) {
		root, _ := decodeObject([]byte(`{"auth_mode":"apikey","OPENAI_API_KEY":"managed","accepted_metadata":{"keep":true}}`))
		got, err := cleanupCodexAuth(root)
		if err != nil || got.Delete || bytes.Contains(got.Content, []byte("managed")) || !bytes.Contains(got.Content, []byte("accepted_metadata")) {
			t.Fatalf("cleanupCodexAuth() = %s, delete=%t, err=%v", got.Content, got.Delete, err)
		}
	})
}

func TestCleanupTransformsDeleteSemanticallyEmptyFiles(t *testing.T) {
	t.Run("claude", func(t *testing.T) {
		root, _ := decodeObject([]byte(`{"env":{"ANTHROPIC_BASE_URL":"managed"}}`))
		got, err := cleanupClaude(root, []string{"env.ANTHROPIC_BASE_URL"})
		if err != nil || !got.Delete || got.Content != nil {
			t.Fatalf("cleanupClaude() = %#v, %v", got, err)
		}
	})

	t.Run("opencode", func(t *testing.T) {
		root, _ := decodeObject([]byte(`{"model":"mtls-router/managed","provider":{"mtls-router":{"name":"CodeasierRouter"}}}`))
		got, err := cleanupOpenCode(root, true)
		if err != nil || !got.Delete || got.Content != nil || !reflect.DeepEqual(got.RemovedPaths, []string{"model", "provider.mtls-router"}) {
			t.Fatalf("cleanupOpenCode() = %#v, %v", got, err)
		}
	})

	t.Run("codex config", func(t *testing.T) {
		content := []byte("model_provider = \"mtls-router\"\nmodel = \"managed\"\ncli_auth_credentials_store = \"file\"\n[model_providers.mtls-router]\nbase_url = \"http://127.0.0.1/v1\"\n")
		got, err := cleanupCodexConfig(content, &modelconfig.CodexConfig{Model: "managed"})
		if err != nil || !got.Delete || got.Content != nil {
			t.Fatalf("cleanupCodexConfig() = %#v, %v", got, err)
		}
	})

	t.Run("codex auth", func(t *testing.T) {
		root, _ := decodeObject([]byte(`{"auth_mode":"apikey","OPENAI_API_KEY":"managed"}`))
		got, err := cleanupCodexAuth(root)
		if err != nil || !got.Delete || got.Content != nil {
			t.Fatalf("cleanupCodexAuth() = %#v, %v", got, err)
		}
	})
}

func TestCleanupTransformsUseEffectiveOwnership(t *testing.T) {
	t.Run("opencode exact managed model prefix", func(t *testing.T) {
		for _, test := range []struct {
			name      string
			model     string
			ownsModel bool
			removed   bool
		}{
			{name: "owned exact prefix", model: "mtls-router/model", ownsModel: true, removed: true},
			{name: "unowned exact prefix", model: "mtls-router/model", removed: false},
			{name: "similar prefix", model: "mtls-routerish/model", ownsModel: true, removed: false},
		} {
			t.Run(test.name, func(t *testing.T) {
				root, _ := decodeObject([]byte(`{"model":"` + test.model + `","keep":true}`))
				got, err := cleanupOpenCode(root, test.ownsModel)
				if err != nil {
					t.Fatal(err)
				}
				if bytes.Contains(got.Content, []byte(`"model"`)) == test.removed {
					t.Fatalf("cleanupOpenCode() = %s, removed=%t", got.Content, test.removed)
				}
			})
		}
	})

	t.Run("codex saved optional fields", func(t *testing.T) {
		verbosity := "high"
		content := []byte("model_provider = \"mtls-router\"\nmodel = \"managed\"\ncli_auth_credentials_store = \"file\"\nmodel_verbosity = \"high\"\nmodel_reasoning_summary = \"user\"\nmodel_auto_compact_token_limit_scope = \"body_after_prefix\"\n[model_providers.mtls-router]\nbase_url = \"managed\"\n")
		got, err := cleanupCodexConfig(content, &modelconfig.CodexConfig{
			Model: "managed", Verbosity: &verbosity,
			Extra: map[string]any{"model_auto_compact_token_limit_scope": "body_after_prefix"},
		})
		if err != nil || got.Delete || bytes.Contains(got.Content, []byte("model_verbosity")) || bytes.Contains(got.Content, []byte("model_auto_compact_token_limit_scope")) || !bytes.Contains(got.Content, []byte("model_reasoning_summary")) {
			t.Fatalf("cleanupCodexConfig() = %s, delete=%t, err=%v", got.Content, got.Delete, err)
		}
	})
}

func TestCleanupTransformsRejectMalformedManagedContainers(t *testing.T) {
	for _, test := range []struct {
		name string
		run  func() error
	}{
		{name: "claude env", run: func() error {
			root, _ := decodeObject([]byte(`{"env":[]}`))
			_, err := cleanupClaude(root, []string{"env.ANTHROPIC_BASE_URL"})
			return err
		}},
		{name: "claude null env", run: func() error {
			root, _ := decodeObject([]byte(`{"env":null}`))
			_, err := cleanupClaude(root, []string{"env.ANTHROPIC_BASE_URL"})
			return err
		}},
		{name: "opencode provider root", run: func() error {
			root, _ := decodeObject([]byte(`{"provider":[]}`))
			_, err := cleanupOpenCode(root, false)
			return err
		}},
		{name: "opencode null provider root", run: func() error {
			root, _ := decodeObject([]byte(`{"provider":null}`))
			_, err := cleanupOpenCode(root, false)
			return err
		}},
		{name: "opencode managed provider", run: func() error {
			root, _ := decodeObject([]byte(`{"provider":{"mtls-router":"invalid"}}`))
			_, err := cleanupOpenCode(root, false)
			return err
		}},
		{name: "opencode null managed provider", run: func() error {
			root, _ := decodeObject([]byte(`{"provider":{"mtls-router":null}}`))
			_, err := cleanupOpenCode(root, false)
			return err
		}},
		{name: "codex providers", run: func() error {
			_, err := cleanupCodexConfig([]byte(`model_providers = "invalid"`), &modelconfig.CodexConfig{Model: "managed"})
			return err
		}},
		{name: "codex managed provider", run: func() error {
			_, err := cleanupCodexConfig([]byte(`model_providers.mtls-router = "invalid"`), &modelconfig.CodexConfig{Model: "managed"})
			return err
		}},
	} {
		t.Run(test.name, func(t *testing.T) {
			if err := test.run(); CodeOf(err) != CodeConfigInvalid {
				t.Fatalf("error = %v", err)
			}
		})
	}
}
