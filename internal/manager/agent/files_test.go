package agent

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestPostReplaceErrorReportsReplacement(t *testing.T) {
	inner := errors.New("sync failed")
	err := &postReplaceError{err: inner}
	if !replacementOccurred(err) || !errors.Is(err, inner) {
		t.Fatalf("post-replace error classification failed: %v", err)
	}
	if replacementOccurred(inner) {
		t.Fatal("ordinary error was classified as a completed replacement")
	}
}

func TestCreatePrivateBackupVerifiesReadBack(t *testing.T) {
	dir := t.TempDir()
	source := filepath.Join(dir, "settings.json")
	content := []byte(`{"secret":"canary"}`)
	if err := os.WriteFile(source, content, 0o600); err != nil {
		t.Fatal(err)
	}
	backup, err := createPrivateBackup(source, content, 0o600, "bak")
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(backup)
	if err != nil || string(got) != string(content) {
		t.Fatalf("verified backup = %q, %v", got, err)
	}
}

func TestCreatePrivateBackupStageFailuresRemoveArtifact(t *testing.T) {
	for _, stage := range []backupStage{backupStagePermission, backupStageWrite, backupStageSync, backupStageReopen, backupStageRead, backupStageIdentity, backupStageContent} {
		t.Run(string(stage), func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "settings.json")
			content := []byte(`{"secret":"canary"}`)
			if err := os.WriteFile(source, content, 0o600); err != nil {
				t.Fatal(err)
			}
			_, err := createPrivateBackupWithHook(source, content, 0o600, "bak", func(current backupStage, _ string) error {
				if current == stage {
					return errors.New("injected failure")
				}
				return nil
			})
			if err == nil {
				t.Fatal("backup stage failure succeeded")
			}
			matches, globErr := filepath.Glob(source + ".bak-*")
			if globErr != nil || len(matches) != 0 {
				t.Fatalf("backup artifacts = %v, %v", matches, globErr)
			}
		})
	}
}

func TestCreatePrivateBackupRejectsStoredByteChanges(t *testing.T) {
	tests := []struct {
		name  string
		stage backupStage
	}{
		{name: "identity", stage: backupStageIdentity},
		{name: "content mismatch", stage: backupStageRead},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			dir := t.TempDir()
			source := filepath.Join(dir, "settings.json")
			content := []byte(`{"secret":"canary"}`)
			if err := os.WriteFile(source, content, 0o600); err != nil {
				t.Fatal(err)
			}
			if _, err := createPrivateBackupWithHook(source, content, 0o600, "bak", func(stage backupStage, path string) error {
				if stage != test.stage {
					return nil
				}
				file, err := os.OpenFile(path, os.O_WRONLY|os.O_TRUNC, 0)
				if err != nil {
					return err
				}
				_, writeErr := file.Write([]byte("mismatch"))
				return errors.Join(writeErr, file.Close())
			}); err == nil {
				t.Fatal("changed backup was accepted")
			}
			matches, err := filepath.Glob(source + ".bak-*")
			if err != nil || len(matches) != 0 {
				t.Fatalf("backup artifacts = %v, %v", matches, err)
			}
		})
	}
}
