//go:build unix

package routerenroll

import (
	"context"
	"errors"
	"os"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

func privateOwner(info os.FileInfo) bool {
	stat, ok := info.Sys().(*syscall.Stat_t)
	return ok && stat.Uid == uint32(os.Geteuid())
}

func openCommitLock(path string) (*os.File, error) {
	file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE|unix.O_NOFOLLOW, 0600)
	if err != nil {
		return nil, ErrContinuity
	}
	info, err := file.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || !privateOwner(info) {
		_ = file.Close()
		return nil, ErrContinuity
	}
	return file, nil
}
func lockCommit(ctx context.Context, file *os.File) error {
	deadline := time.NewTimer(5 * time.Second)
	defer deadline.Stop()
	tick := time.NewTicker(5 * time.Millisecond)
	defer tick.Stop()
	for {
		err := unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB)
		if err == nil {
			return nil
		}
		if !errors.Is(err, unix.EWOULDBLOCK) && !errors.Is(err, unix.EAGAIN) {
			return ErrContinuity
		}
		select {
		case <-ctx.Done():
			return ErrContinuity
		case <-deadline.C:
			return ErrContinuity
		case <-tick.C:
		}
	}
}
func unlockCommit(file *os.File) { _ = unix.Flock(int(file.Fd()), unix.LOCK_UN) }

func acquireWriterLock(path string) (*os.File, error) {
	file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE|unix.O_NOFOLLOW, 0600)
	if err != nil {
		return nil, ErrStorage
	}
	info, err := file.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || !privateOwner(info) {
		_ = file.Close()
		return nil, ErrStorage
	}
	if unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB) != nil {
		_ = file.Close()
		return nil, ErrAlreadyRunning
	}
	return file, nil
}
