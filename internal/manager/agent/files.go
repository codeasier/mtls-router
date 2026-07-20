package agent

import (
	"bytes"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"
)

type backupStage string

const (
	backupStagePermission backupStage = "permission"
	backupStageWrite      backupStage = "write"
	backupStageSync       backupStage = "sync"
	backupStageReopen     backupStage = "reopen"
	backupStageRead       backupStage = "read"
	backupStageIdentity   backupStage = "identity"
	backupStageContent    backupStage = "content"
)

type postReplaceError struct{ err error }

func (e *postReplaceError) Error() string { return e.err.Error() }
func (e *postReplaceError) Unwrap() error { return e.err }

func replacementOccurred(err error) bool {
	var replaced *postReplaceError
	return errors.As(err, &replaced)
}

func createPrivateBackup(sourcePath string, content []byte, sourceMode os.FileMode, label string) (string, error) {
	return createPrivateBackupWithHook(sourcePath, content, sourceMode, label, nil)
}

func createPrivateBackupWithHook(sourcePath string, content []byte, sourceMode os.FileMode, label string, hook func(backupStage, string) error) (string, error) {
	dir := filepath.Dir(sourcePath)
	base := filepath.Base(sourcePath)
	for attempts := 0; attempts < 16; attempts++ {
		var random [12]byte
		if _, err := io.ReadFull(rand.Reader, random[:]); err != nil {
			return "", err
		}
		name := fmt.Sprintf("%s.%s-%s-%s", base, label, time.Now().UTC().Format("20060102-150405.000000000"), hex.EncodeToString(random[:]))
		path := filepath.Join(dir, name)
		file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if errors.Is(err, os.ErrExist) {
			continue
		}
		if err != nil {
			return "", err
		}
		ok := false
		cleanup := func() error {
			file.Close()
			if !ok {
				return removeAndSync(path)
			}
			return nil
		}
		fail := func(cause error) (string, error) {
			if cleanupErr := cleanup(); cleanupErr != nil {
				return path, errors.Join(cause, cleanupErr)
			}
			return "", cause
		}
		if err := runBackupHook(hook, backupStagePermission, path); err != nil {
			return fail(err)
		}
		if err := restrictPrivate(path, false); err != nil {
			return fail(err)
		}
		if err := runBackupHook(hook, backupStageWrite, path); err != nil {
			return fail(err)
		}
		if _, err := file.Write(content); err != nil {
			return fail(err)
		}
		if err := runBackupHook(hook, backupStageSync, path); err != nil {
			return fail(err)
		}
		if err := file.Sync(); err != nil {
			return fail(err)
		}
		if err := file.Close(); err != nil {
			return fail(err)
		}
		if err := applyPrivateMode(path, sourceMode); err != nil {
			return fail(err)
		}
		if err := verifyPrivateBackup(path, content, hook); err != nil {
			return fail(err)
		}
		if err := syncDirectory(dir); err != nil {
			return fail(err)
		}
		ok = true
		_ = cleanup()
		return path, nil
	}
	return "", errors.New("could not allocate unique backup path")
}

func verifyPrivateBackup(path string, expected []byte, hook func(backupStage, string) error) error {
	before, err := os.Lstat(path)
	if err != nil || isFinalComponentLink(path, before) || !before.Mode().IsRegular() {
		return errors.New("backup identity is unsafe")
	}
	if err := runBackupHook(hook, backupStageReopen, path); err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()
	after, err := file.Stat()
	if err != nil || !after.Mode().IsRegular() || !os.SameFile(before, after) {
		return errors.New("backup identity changed")
	}
	if err := runBackupHook(hook, backupStageIdentity, path); err != nil {
		return err
	}
	pathAfter, err := os.Lstat(path)
	if err != nil || isFinalComponentLink(path, pathAfter) || !pathAfter.Mode().IsRegular() || !os.SameFile(after, pathAfter) {
		return errors.New("backup path identity changed")
	}
	if err := runBackupHook(hook, backupStageRead, path); err != nil {
		return err
	}
	content, err := io.ReadAll(io.LimitReader(file, maxConfigSize+1))
	if err != nil || len(content) > maxConfigSize {
		return errors.New("backup read-back failed")
	}
	if err := runBackupHook(hook, backupStageContent, path); err != nil {
		return err
	}
	if !bytes.Equal(content, expected) {
		return errors.New("backup content mismatch")
	}
	return nil
}

func runBackupHook(hook func(backupStage, string) error, stage backupStage, path string) error {
	if hook == nil {
		return nil
	}
	return hook(stage, path)
}

func writeAtomic(path string, content []byte, mode os.FileMode, private bool) (err error) {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, "."+filepath.Base(path)+"-*")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()
	defer func() {
		tmp.Close()
		os.Remove(tmpPath)
	}()
	// Keep secret-bearing temporary contents user-private until replacement.
	if err := restrictPrivate(tmpPath, false); err != nil {
		return err
	}
	if _, err := tmp.Write(content); err != nil {
		return err
	}
	if err := tmp.Sync(); err != nil {
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if !private {
		if err := applyTargetPermissions(tmpPath, path, mode); err != nil {
			return err
		}
	}
	if err := replaceAtomic(tmpPath, path); err != nil {
		return err
	}
	if err := syncDirectory(dir); err != nil {
		return &postReplaceError{err: err}
	}
	return nil
}
