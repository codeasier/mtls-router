//go:build windows

package process

import (
	"errors"
	"os"
	"syscall"
	"time"

	"golang.org/x/sys/windows"
)

var generateConsoleCtrlEvent = windows.NewLazySystemDLL("kernel32.dll").NewProc("GenerateConsoleCtrlEvent")

func inspect(pid int) (string, string, error) {
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
		return "", "", ErrNotFound
	}
	if err != nil {
		return "", "", err
	}
	defer windows.CloseHandle(handle)

	var creation, exit, kernel, user windows.Filetime
	if err := windows.GetProcessTimes(handle, &creation, &exit, &kernel, &user); err != nil {
		return "", "", err
	}
	buf := make([]uint16, 32768)
	size := uint32(len(buf))
	if err := windows.QueryFullProcessImageName(handle, 0, &buf[0], &size); err != nil {
		return "", "", err
	}
	startedAt := time.Unix(0, creation.Nanoseconds()).UTC().Format(time.RFC3339Nano)
	return startedAt, windows.UTF16ToString(buf[:size]), nil
}

func sameStartIdentity(expected, live string) bool {
	expectedTime, expectedErr := time.Parse(time.RFC3339Nano, expected)
	liveTime, liveErr := time.Parse(time.RFC3339Nano, live)
	return expectedErr == nil && liveErr == nil && expectedTime.Equal(liveTime)
}

func signalProcess(pid int, signal os.Signal) error {
	if signal == os.Interrupt {
		// CTRL_BREAK_EVENT can target a CREATE_NEW_PROCESS_GROUP child by PID.
		result, _, callErr := generateConsoleCtrlEvent.Call(1, uintptr(pid))
		if result == 0 {
			return callErr
		}
		return nil
	}
	if signal != os.Kill {
		return syscall.EWINDOWS
	}
	handle, err := windows.OpenProcess(windows.PROCESS_TERMINATE, false, uint32(pid))
	if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
		return ErrNotFound
	}
	if err != nil {
		return err
	}
	defer windows.CloseHandle(handle)
	return windows.TerminateProcess(handle, 1)
}
