//go:build windows

package lifecycle

import (
	"io"
	"os"
	"testing"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

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

func TestClosingDesktopJobTerminatesRouter(t *testing.T) {
	if os.Getenv("MTLS_ROUTER_LIFECYCLE_HELPER") == "1" {
		select {}
	}

	child, err := launchForegroundCommand(
		os.Args[0],
		[]string{"-test.run=TestClosingDesktopJobTerminatesRouter"},
		append(os.Environ(), "MTLS_ROUTER_LIFECYCLE_HELPER=1"),
		io.Discard,
	)
	if err != nil {
		t.Fatal(err)
	}
	process := child.(commandProcess)
	if err := windows.CloseHandle(process.job); err != nil {
		t.Fatal(err)
	}
	process.job = 0

	done := make(chan error, 1)
	go func() { done <- process.cmd.Wait() }()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		_ = process.cmd.Process.Kill()
		t.Fatal("contained router survived closing its job")
	}
}
