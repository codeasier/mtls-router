//go:build linux

package process

import (
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"syscall"
)

func inspect(pid int) (string, string, error) {
	stat, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if errors.Is(err, os.ErrNotExist) {
		return "", "", ErrNotFound
	}
	if err != nil {
		return "", "", err
	}
	closeParen := strings.LastIndex(string(stat), ") ")
	if closeParen < 0 {
		return "", "", errors.New("invalid proc stat")
	}
	fields := strings.Fields(string(stat[closeParen+2:]))
	if len(fields) < 20 {
		return "", "", errors.New("incomplete proc stat")
	}
	startedAt := fields[19]
	if _, err := strconv.ParseUint(startedAt, 10, 64); err != nil {
		return "", "", fmt.Errorf("invalid proc start identity: %w", err)
	}
	executable, err := os.Readlink(fmt.Sprintf("/proc/%d/exe", pid))
	if errors.Is(err, os.ErrNotExist) {
		return "", "", ErrNotFound
	}
	return startedAt, executable, err
}

func sameStartIdentity(expected, live string) bool {
	return expected == live
}

func signalProcess(pid int, signal os.Signal) error {
	process, err := os.FindProcess(pid)
	if err != nil {
		return err
	}
	if err := process.Signal(signal); errors.Is(err, os.ErrProcessDone) || errors.Is(err, syscall.ESRCH) {
		return ErrNotFound
	} else {
		return err
	}
}
