package agent

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"
)

func TestTransactionLockHelperProcess(t *testing.T) {
	if os.Getenv("MTLS_AGENT_LOCK_HELPER") == "" {
		return
	}
	stateDir := os.Getenv("MTLS_AGENT_LOCK_STATE")
	ready := os.Getenv("MTLS_AGENT_LOCK_READY")
	ctx := context.Background()
	if timeout, _ := strconv.Atoi(os.Getenv("MTLS_AGENT_LOCK_CONTEXT_MS")); timeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, time.Duration(timeout)*time.Millisecond)
		defer cancel()
	}
	lock, err := acquireTransactionLock(ctx, stateDir)
	if err != nil {
		if CodeOf(err) == CodeOperationBusy {
			os.Exit(5)
		}
		os.Exit(2)
	}
	if err := os.WriteFile(ready, []byte("ready"), 0o600); err != nil {
		os.Exit(3)
	}
	if os.Getenv("MTLS_AGENT_LOCK_CRASH") != "" {
		os.Exit(0)
	}
	hold, _ := strconv.Atoi(os.Getenv("MTLS_AGENT_LOCK_HOLD_MS"))
	if hold == 0 {
		hold = 750
	}
	time.Sleep(time.Duration(hold) * time.Millisecond)
	if err := lock.release(); err != nil {
		os.Exit(4)
	}
	os.Exit(0)
}

func TestTransactionLockSerializesProcessesAndReleases(t *testing.T) {
	stateDir := filepath.Join(t.TempDir(), "agent-transactions")
	ready := filepath.Join(t.TempDir(), "ready")
	cmd := lockHelperCommand(t, stateDir, ready, false)
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	waitForFile(t, ready)

	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	started := time.Now()
	_, err := acquireTransactionLock(cancelled, stateDir)
	assertCode(t, err, CodeOperationBusy)
	if time.Since(started) > time.Second {
		t.Fatal("cancelled lock acquisition was not bounded by its context")
	}
	if err := cmd.Wait(); err != nil {
		t.Fatal(err)
	}
	lock, err := acquireTransactionLock(context.Background(), stateDir)
	if err != nil {
		t.Fatalf("lock was not released by other process: %v", err)
	}
	if err := lock.release(); err != nil {
		t.Fatal(err)
	}
}

func TestTransactionLockReleasedWhenOwnerProcessExits(t *testing.T) {
	stateDir := filepath.Join(t.TempDir(), "agent-transactions")
	ready := filepath.Join(t.TempDir(), "ready")
	cmd := lockHelperCommand(t, stateDir, ready, true)
	if err := cmd.Run(); err != nil {
		t.Fatal(err)
	}
	lock, err := acquireTransactionLock(context.Background(), stateDir)
	if err != nil {
		t.Fatalf("OS did not release crashed process lock: %v", err)
	}
	_ = lock.release()
}

func TestTransactionLockCrossProcessContentionMatrix(t *testing.T) {
	stateDir := filepath.Join(t.TempDir(), "agent-transactions")
	ready := filepath.Join(t.TempDir(), "ready")
	holder := lockHelperCommand(t, stateDir, ready, false)
	holder.Env = append(holder.Env, "MTLS_AGENT_LOCK_HOLD_MS=1200")
	if err := holder.Start(); err != nil {
		t.Fatal(err)
	}
	waitForFile(t, ready)
	contender := lockHelperCommand(t, stateDir, filepath.Join(t.TempDir(), "contender-ready"), false)
	contender.Env = append(contender.Env, "MTLS_AGENT_LOCK_CONTEXT_MS=150")
	if err := contender.Run(); err == nil {
		t.Fatal("second one-shot process unexpectedly acquired held transaction lock")
	} else if exitErr, ok := err.(*exec.ExitError); !ok || exitErr.ExitCode() != 5 {
		t.Fatalf("second one-shot process error = %v, want busy exit", err)
	}

	for _, test := range []struct {
		name    string
		ctx     func() (context.Context, context.CancelFunc)
		maxWait time.Duration
	}{
		{name: "desktop versus CLI cancellation", ctx: func() (context.Context, context.CancelFunc) { return context.WithCancel(context.Background()) }, maxWait: time.Second},
		{name: "two one-shot writers deadline", ctx: func() (context.Context, context.CancelFunc) {
			return context.WithTimeout(context.Background(), 150*time.Millisecond)
		}, maxWait: time.Second},
	} {
		t.Run(test.name, func(t *testing.T) {
			ctx, cancel := test.ctx()
			if test.name == "desktop versus CLI cancellation" {
				cancel()
			} else {
				defer cancel()
			}
			started := time.Now()
			lock, err := acquireTransactionLock(ctx, stateDir)
			if lock != nil {
				_ = lock.release()
				t.Fatal("contender acquired held transaction lock")
			}
			assertCode(t, err, CodeOperationBusy)
			if elapsed := time.Since(started); elapsed > test.maxWait {
				t.Fatalf("contention returned after %s, want <= %s", elapsed, test.maxWait)
			}
		})
	}
	if err := holder.Wait(); err != nil {
		t.Fatal(err)
	}

	first, err := acquireTransactionLock(context.Background(), stateDir)
	if err != nil {
		t.Fatal(err)
	}
	if err := first.release(); err != nil {
		t.Fatal(err)
	}
	second, err := acquireTransactionLock(context.Background(), stateDir)
	if err != nil {
		t.Fatalf("lock was not reusable after release: %v", err)
	}
	if err := second.release(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(stateDir, lockFileName)); err != nil && !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("lock path became unusable: %v", err)
	}
}

func TestTransactionLockTimeoutIsFiveSeconds(t *testing.T) {
	if lockTimeout != 5*time.Second {
		t.Fatalf("lock timeout = %s, want 5s", lockTimeout)
	}
}

func TestTransactionLockEnforcesFiveSecondAcquisitionTimeout(t *testing.T) {
	stateDir := filepath.Join(t.TempDir(), "agent-transactions")
	ready := filepath.Join(t.TempDir(), "ready")
	holder := lockHelperCommand(t, stateDir, ready, false)
	holder.Env = append(holder.Env, "MTLS_AGENT_LOCK_HOLD_MS=6000")
	if err := holder.Start(); err != nil {
		t.Fatal(err)
	}
	waitForFile(t, ready)
	started := time.Now()
	lock, err := acquireTransactionLock(context.Background(), stateDir)
	if lock != nil {
		_ = lock.release()
		t.Fatal("acquired lock before holder released it")
	}
	assertCode(t, err, CodeOperationBusy)
	elapsed := time.Since(started)
	if elapsed < 4900*time.Millisecond || elapsed > 5700*time.Millisecond {
		t.Fatalf("lock timeout = %s, want approximately 5s", elapsed)
	}
	if err := holder.Wait(); err != nil {
		t.Fatal(err)
	}
}

func lockHelperCommand(t *testing.T, stateDir, ready string, crash bool) *exec.Cmd {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=^TestTransactionLockHelperProcess$")
	cmd.Env = append(os.Environ(), "MTLS_AGENT_LOCK_HELPER=1", "MTLS_AGENT_LOCK_STATE="+stateDir, "MTLS_AGENT_LOCK_READY="+ready)
	if crash {
		cmd.Env = append(cmd.Env, "MTLS_AGENT_LOCK_CRASH=1")
	}
	return cmd
}

func waitForFile(t *testing.T, path string) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if _, err := os.Stat(path); err == nil {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("helper process did not acquire lock")
}
