//go:build !windows

package process

import (
	"os"
	"syscall"
)

func processAlive(pid int) error {
	process, err := os.FindProcess(pid)
	if err != nil {
		return err
	}
	return process.Signal(syscall.Signal(0))
}
