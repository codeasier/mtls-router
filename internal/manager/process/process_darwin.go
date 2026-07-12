//go:build darwin

package process

import (
	"encoding/binary"
	"errors"
	"fmt"
	"os"
	"strings"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

func inspect(pid int) (string, string, error) {
	info, err := unix.SysctlKinfoProc("kern.proc.pid", pid)
	if errors.Is(err, syscall.ESRCH) || info == nil || info.Proc.P_pid == 0 {
		return "", "", ErrNotFound
	}
	if err != nil {
		return "", "", fmt.Errorf("read process start identity: %w", err)
	}
	args, err := unix.SysctlRaw("kern.procargs2", pid)
	if errors.Is(err, syscall.ESRCH) {
		return "", "", ErrNotFound
	}
	if err != nil {
		return "", "", fmt.Errorf("read process executable: %w", err)
	}
	if len(args) <= 4 || binary.LittleEndian.Uint32(args[:4]) == 0 {
		return "", "", errors.New("empty process executable path")
	}
	executable, _, _ := strings.Cut(string(args[4:]), "\x00")
	if executable == "" {
		return "", "", errors.New("empty process executable path")
	}
	start := time.Unix(info.Proc.P_starttime.Sec, int64(info.Proc.P_starttime.Usec)*1000)
	return start.UTC().Format(time.RFC3339Nano), executable, nil
}

func sameStartIdentity(expected, live string) bool {
	liveTime, err := time.Parse(time.RFC3339Nano, live)
	if err != nil {
		return false
	}
	if expectedTime, err := time.Parse(time.RFC3339Nano, expected); err == nil {
		return expectedTime.Equal(liveTime)
	}
	legacy, err := time.ParseInLocation("Mon Jan 2 15:04:05 2006", expected, time.Local)
	return err == nil && legacy.Unix() == liveTime.Unix()
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
