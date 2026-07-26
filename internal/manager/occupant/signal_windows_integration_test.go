//go:build windows

package occupant

import (
	"bufio"
	"bytes"
	"context"
	"fmt"
	"net"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
	"time"
)

const windowsSignalHelperEnv = "MTLS_ROUTER_WINDOWS_SIGNAL_HELPER"

func TestWindowsNativePreflightAndSignalReleaseHelperListener(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(executable, "-test.run=^TestWindowsNativeSignalHelper$")
	cmd.Env = append(os.Environ(), windowsSignalHelperEnv+"=1")
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}

	wait := make(chan error, 1)
	go func() { wait <- cmd.Wait() }()
	exited := false
	t.Cleanup(func() {
		if exited {
			return
		}
		_ = cmd.Process.Kill()
		select {
		case <-wait:
		case <-time.After(5 * time.Second):
			t.Errorf("helper PID %d did not exit during cleanup", cmd.Process.Pid)
		}
	})

	ready := make(chan string, 1)
	readyErr := make(chan error, 1)
	go func() {
		line, err := bufio.NewReader(stdout).ReadString('\n')
		if err != nil {
			readyErr <- fmt.Errorf("read helper readiness: %w", err)
			return
		}
		ready <- strings.TrimSuffix(line, "\n")
	}()

	var address string
	select {
	case address = <-ready:
	case err := <-readyErr:
		t.Fatal(err)
	case err := <-wait:
		exited = true
		t.Fatalf("helper exited before readiness: %v: %s", err, strings.TrimSpace(stderr.String()))
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for helper readiness")
	}
	host, port, err := net.SplitHostPort(address)
	portNumber, portErr := strconv.Atoi(port)
	if err != nil || portErr != nil || host != "127.0.0.1" || portNumber <= 0 || portNumber > 65535 {
		t.Fatalf("invalid helper address %q: %v", address, err)
	}

	probeCtx, cancelProbe := context.WithTimeout(context.Background(), 5*time.Second)
	if err := pollUntil(probeCtx, func() bool {
		connection, err := net.DialTimeout("tcp4", address, 200*time.Millisecond)
		if err != nil {
			return false
		}
		_ = connection.Close()
		return true
	}); err != nil {
		cancelProbe()
		t.Fatalf("helper address %s never accepted a connection: %v", address, err)
	}
	cancelProbe()
	if err := preflightTerminatePIDNative(cmd.Process.Pid); err != nil {
		t.Fatalf("preflight helper PID %d: %v", cmd.Process.Pid, err)
	}
	connection, err := net.DialTimeout("tcp4", address, 200*time.Millisecond)
	if err != nil {
		t.Fatalf("preflight changed helper listener %s: %v", address, err)
	}
	_ = connection.Close()
	select {
	case err := <-wait:
		exited = true
		t.Fatalf("preflight terminated helper PID %d: %v", cmd.Process.Pid, err)
	default:
	}

	if err := signalPIDNative(cmd.Process.Pid); err != nil {
		t.Fatalf("terminate helper PID %d: %v", cmd.Process.Pid, err)
	}
	exitTimer := time.NewTimer(5 * time.Second)
	defer exitTimer.Stop()
	select {
	case <-wait:
		exited = true
	case <-exitTimer.C:
		t.Fatalf("helper PID %d did not exit within timeout", cmd.Process.Pid)
	}

	releaseCtx, cancelRelease := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancelRelease()
	if err := pollUntil(releaseCtx, func() bool {
		connection, err := net.DialTimeout("tcp4", address, 100*time.Millisecond)
		if err != nil {
			return true
		}
		_ = connection.Close()
		return false
	}); err != nil {
		t.Fatalf("helper address %s remained connectable: %v", address, err)
	}
	if err := pollUntil(releaseCtx, func() bool {
		listener, err := net.Listen("tcp4", address)
		if err != nil {
			return false
		}
		_ = listener.Close()
		return true
	}); err != nil {
		t.Fatalf("helper address %s was not released: %v", address, err)
	}
}

func TestWindowsNativeSignalHelper(t *testing.T) {
	if os.Getenv(windowsSignalHelperEnv) != "1" {
		return
	}
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	if _, err := fmt.Fprintln(os.Stdout, listener.Addr().String()); err != nil {
		t.Fatal(err)
	}
	for {
		connection, err := listener.Accept()
		if err != nil {
			t.Fatal(err)
		}
		_ = connection.Close()
	}
}

func pollUntil(ctx context.Context, condition func() bool) error {
	if condition() {
		return nil
	}
	ticker := time.NewTicker(20 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			if condition() {
				return nil
			}
		}
	}
}
