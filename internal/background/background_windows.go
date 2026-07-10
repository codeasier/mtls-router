//go:build windows

package background

import (
	"os"
	"os/exec"
	"syscall"

	"golang.org/x/sys/windows"
)

func windowsSysProcAttr() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{
		CreationFlags: windows.CREATE_NEW_PROCESS_GROUP | windows.DETACHED_PROCESS,
		HideWindow:    true,
	}
}

func Start(exePath string, args []string, logPath string) (int, error) {
	logFile, err := OpenLogFile(logPath)
	if err != nil {
		return 0, err
	}
	defer logFile.Close()

	cmd := exec.Command(exePath, args...)
	cmd.Env = ChildEnv(os.Environ())
	cmd.SysProcAttr = windowsSysProcAttr()
	cmd.Stdin = nil
	cmd.Stdout = logFile
	cmd.Stderr = logFile
	if err := cmd.Start(); err != nil {
		return 0, err
	}
	pid := cmd.Process.Pid
	_ = cmd.Process.Release()
	return pid, nil
}
