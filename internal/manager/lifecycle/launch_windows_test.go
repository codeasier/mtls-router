//go:build windows

package lifecycle

import (
	"bytes"
	"io"
	"os"
	"strings"
	"testing"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

func TestLaunchForegroundCommandDrainsImmediateExitStderr(t *testing.T) {
	const (
		helperEnv  = "MTLS_ROUTER_LIFECYCLE_IMMEDIATE_EXIT_HELPER"
		diagnostic = "mtls-router immediate startup failure: distinctive stderr diagnostic is complete"
	)
	if os.Getenv(helperEnv) == "1" {
		_, _ = os.Stderr.WriteString(diagnostic)
		os.Exit(23)
	}

	var output bytes.Buffer
	child, err := launchForegroundCommand(
		os.Args[0],
		[]string{"-test.run=^TestLaunchForegroundCommandDrainsImmediateExitStderr$"},
		append(os.Environ(), helperEnv+"=1"),
		&output,
	)
	if err != nil {
		t.Fatal(err)
	}
	process := child.(*commandProcess)
	if process.cmd.WaitDelay != foregroundWaitDelay {
		t.Fatalf("WaitDelay = %s, want %s", process.cmd.WaitDelay, foregroundWaitDelay)
	}
	if err := process.Wait(); err == nil {
		t.Fatal("helper process unexpectedly succeeded")
	}
	if got := output.String(); !strings.Contains(got, diagnostic) {
		t.Fatalf("output %q does not contain complete diagnostic %q", got, diagnostic)
	}
}

func TestDesktopCreationFlagsStartHiddenSuspendedProcessGroup(t *testing.T) {
	flags := desktopCreationFlags()
	for _, want := range []uint32{windows.CREATE_NEW_PROCESS_GROUP, windows.CREATE_SUSPENDED} {
		if flags&want == 0 {
			t.Fatalf("desktop creation flags %#x missing %#x", flags, want)
		}
	}
}

func TestKillOnCloseJobIsConfigured(t *testing.T) {
	job, err := createKillOnCloseJob()
	if err != nil {
		t.Fatal(err)
	}
	defer windows.CloseHandle(job)

	info := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	if err := windows.QueryInformationJobObject(
		job,
		windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&info)),
		uint32(unsafe.Sizeof(info)),
		nil,
	); err != nil {
		t.Fatal(err)
	}
	if flags := info.BasicLimitInformation.LimitFlags; flags&windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE == 0 {
		t.Fatalf("job limit flags %#x missing JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE", flags)
	}
}

func TestDesktopKillTerminatesRouterJob(t *testing.T) {
	if os.Getenv("MTLS_ROUTER_LIFECYCLE_HELPER") == "1" {
		select {}
	}

	child, err := launchForegroundCommand(
		os.Args[0],
		[]string{"-test.run=TestDesktopKillTerminatesRouterJob"},
		append(os.Environ(), "MTLS_ROUTER_LIFECYCLE_HELPER=1"),
		io.Discard,
	)
	if err != nil {
		t.Fatal(err)
	}
	process := child.(*commandProcess)
	if err := process.Kill(); err != nil {
		t.Fatal(err)
	}

	done := make(chan error, 1)
	go func() { done <- process.Wait() }()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		_ = process.cmd.Process.Kill()
		t.Fatal("contained router survived closing its job")
	}
}
