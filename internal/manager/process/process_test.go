package process

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"
)

func TestInspectAndValidateCurrentProcess(t *testing.T) {
	identity, err := Inspect(os.Getpid())
	if err != nil {
		t.Fatal(err)
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	status, err := Validate(identity, executable)
	if err != nil {
		t.Fatal(err)
	}
	if status != StatusGenuine {
		t.Fatalf("status = %q, want genuine", status)
	}
}

func TestValidateRejectsIncompleteAndMismatchedIdentity(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	identity, err := Inspect(os.Getpid())
	if err != nil {
		t.Fatal(err)
	}
	tests := []Identity{
		{PID: identity.PID},
		{PID: identity.PID, StartedAt: identity.StartedAt + "-reused", Executable: identity.Executable},
		{PID: identity.PID, StartedAt: identity.StartedAt, Executable: filepath.Join(t.TempDir(), "other")},
	}
	for _, expected := range tests {
		status, _ := Validate(expected, executable)
		if status != StatusStale {
			t.Fatalf("Validate(%+v) = %q, want stale", expected, status)
		}
	}
}

func TestValidateMissingProcess(t *testing.T) {
	status, err := Validate(Identity{PID: 1<<30 - 1, StartedAt: "missing", Executable: "/missing"}, "/missing")
	if err != nil {
		t.Fatal(err)
	}
	if status != StatusAbsent {
		t.Fatalf("status = %q, want absent", status)
	}
}

func TestSignalNeverTrustsPIDAlone(t *testing.T) {
	cmd := helperProcess(t)
	identity, err := Inspect(cmd.Process.Pid)
	if err != nil {
		cmd.Process.Kill()
		t.Fatal(err)
	}
	forged := identity
	forged.StartedAt += "-reused"
	if err := Signal(forged, identity.Executable, os.Kill); !errors.Is(err, ErrIdentityMismatch) {
		cmd.Process.Kill()
		t.Fatalf("forged signal error = %v", err)
	}
	if err := processAlive(cmd.Process.Pid); err != nil {
		cmd.Process.Kill()
		t.Fatalf("forged signal stopped process: %v", err)
	}
	if err := Signal(identity, identity.Executable, os.Kill); err != nil {
		cmd.Process.Kill()
		t.Fatal(err)
	}
	_ = cmd.Wait()
}

func helperProcess(t *testing.T) *exec.Cmd {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=TestProcessHelper", "--", strconv.FormatInt(time.Now().UnixNano(), 10))
	cmd.Env = append(os.Environ(), "MTLS_ROUTER_PROCESS_HELPER=1")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if cmd.Process != nil {
			_ = cmd.Process.Kill()
		}
	})
	return cmd
}

func TestProcessHelper(t *testing.T) {
	if os.Getenv("MTLS_ROUTER_PROCESS_HELPER") != "1" {
		return
	}
	for {
		time.Sleep(time.Second)
	}
}
