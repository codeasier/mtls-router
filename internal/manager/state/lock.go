package state

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

var ErrLocked = errors.New("desktop ownership is already locked")

// Lock is an exclusively held desktop ownership lock.
type Lock struct {
	file *os.File
}

// AcquireLock attempts to acquire path without waiting.
func AcquireLock(path string) (*Lock, error) {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("create lock directory: %w", err)
	}
	if err := restrictPath(dir, true); err != nil {
		return nil, fmt.Errorf("restrict lock directory: %w", err)
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, fmt.Errorf("open ownership lock: %w", err)
	}
	if err := restrictPath(path, false); err != nil {
		f.Close()
		return nil, fmt.Errorf("restrict ownership lock: %w", err)
	}
	if err := lockFile(f); err != nil {
		f.Close()
		if isLockConflict(err) {
			return nil, ErrLocked
		}
		return nil, fmt.Errorf("acquire ownership lock: %w", err)
	}
	return &Lock{file: f}, nil
}

// Close releases the ownership lock.
func (l *Lock) Close() error {
	if l == nil || l.file == nil {
		return nil
	}
	f := l.file
	l.file = nil
	unlockErr := unlockFile(f)
	closeErr := f.Close()
	return errors.Join(unlockErr, closeErr)
}
