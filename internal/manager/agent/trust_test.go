package agent

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
)

func TestSigningKeyIsPrivateRandomAndSharedAcrossServices(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	detector := testServiceDetector(home, nil)
	first, err := NewService(Options{StateDir: stateDir, Detector: detector, LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	preview, err := first.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	keyPath := filepath.Join(stateDir, signingKeyFileName)
	key, err := loadSigningKey(keyPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(key.Key, stateDir) || key.Generation == "" {
		t.Fatal("signing key was derived from public state")
	}
	second, err := NewService(Options{StateDir: stateDir, Detector: detector, LegacyRenderInput: legacyTestRenderInput()})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := second.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey}); err != nil {
		t.Fatalf("cross-service token verification failed: %v", err)
	}
}

func TestConcurrentSigningKeyCreationConverges(t *testing.T) {
	home := t.TempDir()
	stateDir := filepath.Join(home, "state")
	detector := testServiceDetector(home, nil)
	services := make([]*Service, 2)
	for i := range services {
		var err error
		services[i], err = NewService(Options{StateDir: stateDir, Detector: detector, LegacyRenderInput: legacyTestRenderInput()})
		if err != nil {
			t.Fatal(err)
		}
	}
	var wg sync.WaitGroup
	errs := make(chan error, 2)
	for _, service := range services {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := service.Preview(context.Background(), []Kind{ClaudeCode})
			errs <- err
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Fatal(err)
		}
	}
	if services[0].keyGeneration == "" || services[0].keyGeneration != services[1].keyGeneration {
		t.Fatalf("services did not converge: %q %q", services[0].keyGeneration, services[1].keyGeneration)
	}
}

func TestSigningKeyCorruptionPermissionsAndLossFailClosed(t *testing.T) {
	t.Run("corruption", func(t *testing.T) {
		home := t.TempDir()
		stateDir := filepath.Join(home, "state")
		service := newTestService(t, stateDir, home, nil)
		if _, err := service.Preview(context.Background(), []Kind{ClaudeCode}); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(stateDir, signingKeyFileName), []byte(`{"version":1}`), 0o600); err != nil {
			t.Fatal(err)
		}
		fresh, err := NewService(Options{StateDir: stateDir, Detector: service.detector, LegacyRenderInput: legacyTestRenderInput()})
		if err != nil {
			t.Fatal(err)
		}
		_, err = fresh.Preview(context.Background(), []Kind{ClaudeCode})
		assertCode(t, err, CodeModelStateInvalid)
	})

	t.Run("loss with owned state", func(t *testing.T) {
		home := t.TempDir()
		stateDir := filepath.Join(home, "state")
		service := newTestService(t, stateDir, home, nil)
		if _, err := service.Preview(context.Background(), []Kind{ClaudeCode}); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(stateDir, sidecarFileName), []byte(`{}`), 0o600); err != nil {
			t.Fatal(err)
		}
		if err := os.Remove(filepath.Join(stateDir, signingKeyFileName)); err != nil {
			t.Fatal(err)
		}
		fresh, err := NewService(Options{StateDir: stateDir, Detector: service.detector, LegacyRenderInput: legacyTestRenderInput()})
		if err != nil {
			t.Fatal(err)
		}
		_, err = fresh.Preview(context.Background(), []Kind{ClaudeCode})
		assertCode(t, err, CodeModelStateInvalid)
	})

	if runtime.GOOS != "windows" {
		t.Run("permissions", func(t *testing.T) {
			home := t.TempDir()
			stateDir := filepath.Join(home, "state")
			service := newTestService(t, stateDir, home, nil)
			if _, err := service.Preview(context.Background(), []Kind{ClaudeCode}); err != nil {
				t.Fatal(err)
			}
			if err := os.Chmod(filepath.Join(stateDir, signingKeyFileName), 0o644); err != nil {
				t.Fatal(err)
			}
			fresh, err := NewService(Options{StateDir: stateDir, Detector: service.detector, LegacyRenderInput: legacyTestRenderInput()})
			if err != nil {
				t.Fatal(err)
			}
			_, err = fresh.Preview(context.Background(), []Kind{ClaudeCode})
			assertCode(t, err, CodeModelStateInvalid)
		})
	}
}

func TestNewJournalUsesKeyedContextSeparatedRevisions(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(home, ".claude", "settings.json")
	original := []byte(`{"sentinel":"secret-bearing"}`)
	writeFile(t, path, string(original))
	service := newTestService(t, filepath.Join(home, "state"), home, nil)
	preview, err := service.Preview(context.Background(), []Kind{ClaudeCode})
	if err != nil {
		t.Fatal(err)
	}
	service.hooks.beforeReplace = func(string) error {
		journal, err := decodeJournal(service.journalPath())
		if err != nil {
			return err
		}
		plain := sha256.Sum256(original)
		entry := journal.Entries[0]
		if journal.Version != 3 || journal.KeyGeneration != service.keyGeneration || entry.PreRevision.Digest == hex.EncodeToString(plain[:]) || entry.BackupRevision.Digest == entry.PreRevision.Digest {
			t.Fatalf("journal revisions are not keyed/context-separated: %#v", journal)
		}
		return nil
	}
	if _, err := service.Write(context.Background(), WriteRequest{Agents: []Kind{ClaudeCode}, RevisionToken: preview.RevisionToken, APIKey: testAPIKey}); err != nil {
		t.Fatal(err)
	}
}
