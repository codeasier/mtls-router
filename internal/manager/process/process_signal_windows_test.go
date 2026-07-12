//go:build windows

package process

import "golang.org/x/sys/windows"

func processAlive(pid int) error {
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if err != nil {
		return err
	}
	return windows.CloseHandle(handle)
}
