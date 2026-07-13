// Package process retrieves and validates process identity before signaling.
package process

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

var (
	ErrNotFound         = errors.New("process not found")
	ErrIdentityMismatch = errors.New("process identity mismatch")
)

// Identity is the minimum process identity needed to defend against PID reuse.
type Identity struct {
	PID        int    `json:"pid"`
	StartedAt  string `json:"started_at"`
	Executable string `json:"executable"`
}

// Status classifies recorded identity against the current process table.
type Status string

const (
	StatusGenuine Status = "genuine"
	StatusAbsent  Status = "absent"
	StatusStale   Status = "stale"
)

// Inspect returns the current start and executable identity for pid.
func Inspect(pid int) (Identity, error) {
	if pid <= 0 {
		return Identity{}, ErrNotFound
	}
	startedAt, executable, err := inspect(pid)
	if err != nil {
		return Identity{}, err
	}
	executable, err = NormalizeExecutable(executable)
	if err != nil {
		return Identity{}, fmt.Errorf("normalize process executable: %w", err)
	}
	return Identity{PID: pid, StartedAt: startedAt, Executable: executable}, nil
}

// NormalizeExecutable normalizes an executable identity. Linux's procfs suffix
// for a replaced running image is intentionally removed.
func NormalizeExecutable(path string) (string, error) {
	path = strings.TrimSpace(path)
	if runtime.GOOS == "linux" {
		path = strings.TrimSuffix(path, " (deleted)")
	}
	if runtime.GOOS == "windows" {
		if strings.HasPrefix(path, `\\?\UNC\`) {
			path = `\\` + strings.TrimPrefix(path, `\\?\UNC\`)
		} else {
			path = strings.TrimPrefix(path, `\\?\`)
		}
	}
	if path == "" {
		return "", errors.New("empty executable path")
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	dir, base := filepath.Split(filepath.Clean(abs))
	if resolved, err := filepath.EvalSymlinks(dir); err == nil {
		abs = filepath.Join(resolved, base)
	}
	return filepath.Clean(abs), nil
}

// Validate checks PID, process start identity, recorded executable identity,
// and expected binary path. Incomplete or inaccessible identity is stale.
func Validate(expected Identity, binaryPath string) (Status, error) {
	if expected.PID <= 0 || expected.StartedAt == "" || expected.Executable == "" || binaryPath == "" {
		return StatusStale, nil
	}
	live, err := Inspect(expected.PID)
	if errors.Is(err, ErrNotFound) {
		return StatusAbsent, nil
	}
	if err != nil {
		return StatusStale, err
	}
	storedExecutable, err := NormalizeExecutable(expected.Executable)
	if err != nil {
		return StatusStale, err
	}
	storedBinary, err := NormalizeExecutable(binaryPath)
	if err != nil {
		return StatusStale, err
	}
	if !sameStartIdentity(expected.StartedAt, live.StartedAt) || !sameExecutable(storedExecutable, live.Executable) || !sameExecutable(storedExecutable, storedBinary) {
		return StatusStale, nil
	}
	return StatusGenuine, nil
}

func sameExecutable(left, right string) bool {
	if runtime.GOOS == "windows" {
		return strings.EqualFold(left, right)
	}
	return left == right
}

// Signal validates complete identity immediately before signaling. Callers do
// not receive a PID-only signaling API from this package.
func Signal(expected Identity, binaryPath string, signal os.Signal) error {
	status, err := Validate(expected, binaryPath)
	if err != nil {
		return err
	}
	if status != StatusGenuine {
		return fmt.Errorf("%w: %s", ErrIdentityMismatch, status)
	}
	return signalProcess(expected.PID, signal)
}
