package agent

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"time"
)

const (
	lockFileName = "agent-operation.lock"
	lockTimeout  = 5 * time.Second
)

type transactionLock struct {
	file *os.File
}

func acquireTransactionLock(ctx context.Context, stateDir string) (*transactionLock, error) {
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		return nil, err
	}
	if err := restrictPrivate(stateDir, true); err != nil {
		return nil, err
	}
	path := filepath.Join(stateDir, lockFileName)
	file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o600)
	if err != nil {
		return nil, err
	}
	if err := restrictPrivate(path, false); err != nil {
		file.Close()
		return nil, err
	}
	return acquireOpenedTransactionLock(ctx, file)
}

// acquireExistingTransactionLock opens existing coordination state without
// creating directories, files, or changing permissions.
func acquireExistingTransactionLock(ctx context.Context, stateDir string) (*transactionLock, error) {
	dirInfo, err := os.Lstat(stateDir)
	if err != nil || isFinalComponentLink(stateDir, dirInfo) || !dirInfo.IsDir() || !privatePermissionsOK(stateDir, true, dirInfo.Mode()) {
		return nil, errors.New("invalid transaction state directory")
	}
	path := filepath.Join(stateDir, lockFileName)
	pathInfo, err := os.Lstat(path)
	if err != nil || isFinalComponentLink(path, pathInfo) || !pathInfo.Mode().IsRegular() || !privatePermissionsOK(path, false, pathInfo.Mode()) {
		return nil, errors.New("invalid transaction lock")
	}
	file, err := os.OpenFile(path, os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	openedInfo, err := file.Stat()
	if err != nil || !openedInfo.Mode().IsRegular() || !os.SameFile(pathInfo, openedInfo) {
		file.Close()
		return nil, errors.New("transaction lock identity changed")
	}
	pathAfter, err := os.Lstat(path)
	if err != nil || isFinalComponentLink(path, pathAfter) || !os.SameFile(openedInfo, pathAfter) || !privatePermissionsOK(path, false, pathAfter.Mode()) {
		file.Close()
		return nil, errors.New("transaction lock path identity changed")
	}
	return acquireOpenedTransactionLock(ctx, file)
}

func acquireOpenedTransactionLock(ctx context.Context, file *os.File) (*transactionLock, error) {
	deadline := time.Now().Add(lockTimeout)
	for {
		locked, err := tryLockFile(file)
		if err != nil {
			file.Close()
			return nil, err
		}
		if locked {
			return &transactionLock{file: file}, nil
		}
		if ctx != nil {
			select {
			case <-ctx.Done():
				file.Close()
				return nil, operationError(CodeOperationBusy, "Another Agent operation is in progress")
			default:
			}
		}
		if !time.Now().Before(deadline) {
			file.Close()
			return nil, operationError(CodeOperationBusy, "Another Agent operation is in progress")
		}
		time.Sleep(25 * time.Millisecond)
	}
}

func (l *transactionLock) release() error {
	if l == nil || l.file == nil {
		return nil
	}
	err := unlockFile(l.file)
	return errors.Join(err, l.file.Close())
}
