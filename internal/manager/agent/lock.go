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
