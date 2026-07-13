package state

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"testing"
	"time"
)

func TestReadLegacySetupState(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "setup-state.json")
	legacy := `{"pid":42,"listen_addr":"http://127.0.0.1:19099","binary_path":"/bin/router","log_path":"/tmp/router.log","started_at":"2026-01-01T00:00:00Z","process_started_at":"1234","process_executable":"/bin/router"}`
	for _, tt := range []struct {
		name    string
		content string
	}{
		{"utf8", legacy},
		{"powershell-utf8-bom", "\xef\xbb\xbf" + legacy},
	} {
		t.Run(tt.name, func(t *testing.T) {
			if err := os.WriteFile(path, []byte(tt.content), 0o600); err != nil {
				t.Fatal(err)
			}
			got, err := Read(path)
			if err != nil {
				t.Fatal(err)
			}
			if got.PID != 42 || got.ProcessStartedAt != "1234" || got.ProcessExecutable != "/bin/router" {
				t.Fatalf("legacy state not decoded: %+v", got)
			}
		})
	}
}

func TestDesktopStateRoundTripAndPermissions(t *testing.T) {
	path := filepath.Join(t.TempDir(), "private", "desktop-state.json")
	want := RouterState{
		PID: 10, Owner: "desktop", DesktopSessionID: "session",
		ManagerPID: 11, ManagerProcessStartedAt: "manager-start",
		ManagerProcessExecutable: "/manager", ProcessStartedAt: "router-start",
		ProcessExecutable: "/router", DeploymentID: "prod-a", ManagementProtocolVersion: "1",
	}
	if err := Write(path, want); err != nil {
		t.Fatal(err)
	}
	got, err := Read(path)
	if err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("round trip mismatch: got %+v want %+v", got, want)
	}
	if runtime.GOOS != "windows" {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != 0o600 {
			t.Fatalf("state mode = %o, want 600", info.Mode().Perm())
		}
		dirInfo, err := os.Stat(filepath.Dir(path))
		if err != nil {
			t.Fatal(err)
		}
		if dirInfo.Mode().Perm() != 0o700 {
			t.Fatalf("state directory mode = %o, want 700", dirInfo.Mode().Perm())
		}
	}
}

func TestReadMissingAndCorrupt(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "missing.json")
	if _, err := Read(missing); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("missing error = %v", err)
	}
	for _, content := range []string{"{", `{} {}`} {
		if err := os.WriteFile(missing, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := Read(missing); !errors.Is(err, ErrCorrupt) {
			t.Fatalf("corrupt error = %v", err)
		}
	}
}

func TestPermissionErrors(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX mode bits do not model Windows ACLs")
	}
	dir := t.TempDir()
	path := filepath.Join(dir, "state.json")
	if err := os.WriteFile(path, []byte(`{"pid":1}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, 0); err != nil {
		t.Fatal(err)
	}
	defer os.Chmod(path, 0o600)
	if _, err := Read(path); err == nil {
		t.Fatal("read unexpectedly succeeded without file permission")
	}
	if err := os.Chmod(dir, 0o500); err != nil {
		t.Fatal(err)
	}
	defer os.Chmod(dir, 0o700)
	if err := Write(filepath.Join(dir, "denied", "state.json"), RouterState{PID: 2}); err == nil {
		t.Fatal("write unexpectedly succeeded without directory write permission")
	}
}

func TestConcurrentWritesRemainCompleteJSON(t *testing.T) {
	path := filepath.Join(t.TempDir(), "state.json")
	if err := Write(path, RouterState{PID: 1}); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	errCh := make(chan error, 128)
	for i := 1; i <= 64; i++ {
		wg.Add(1)
		go func(pid int) {
			defer wg.Done()
			if err := Write(path, RouterState{PID: pid, DesktopSessionID: fmt.Sprintf("session-%d", pid)}); err != nil {
				errCh <- err
			}
		}(i)
	}
	for i := 0; i < 64; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for attempt := 0; attempt < 10; attempt++ {
				if _, err := Read(path); err != nil {
					errCh <- err
					return
				}
				time.Sleep(time.Millisecond)
			}
		}()
	}
	wg.Wait()
	close(errCh)
	for err := range errCh {
		t.Error(err)
	}
}

func TestExclusiveOwnershipLock(t *testing.T) {
	path := filepath.Join(t.TempDir(), "desktop-owner.lock")
	first, err := AcquireLock(path)
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	if second, err := AcquireLock(path); !errors.Is(err, ErrLocked) {
		if second != nil {
			second.Close()
		}
		t.Fatalf("second acquire error = %v, want ErrLocked", err)
	}
	if err := first.Close(); err != nil {
		t.Fatal(err)
	}
	third, err := AcquireLock(path)
	if err != nil {
		t.Fatalf("acquire after release: %v", err)
	}
	if err := third.Close(); err != nil {
		t.Fatal(err)
	}
}
