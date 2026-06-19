//go:build darwin || linux

package background

import (
	"os/exec"
	"syscall"
)

func Start(exePath string, args []string, logPath string) (int, error) {
	logFile, err := OpenLogFile(logPath)
	if err != nil {
		return 0, err
	}
	defer logFile.Close()

	cmd := exec.Command(exePath, args...)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	cmd.Stdin = nil
	cmd.Stdout = logFile
	cmd.Stderr = logFile
	if err := cmd.Start(); err != nil {
		return 0, err
	}
	return cmd.Process.Pid, nil
}
