//go:build windows

package lifecycle

import (
	"os/exec"
	"syscall"

	"golang.org/x/sys/windows"
)

func configureDesktopCommand(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{CreationFlags: windows.CREATE_NEW_PROCESS_GROUP, HideWindow: true}
}
