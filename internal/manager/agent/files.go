package agent

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"
)

type postReplaceError struct{ err error }

func (e *postReplaceError) Error() string { return e.err.Error() }
func (e *postReplaceError) Unwrap() error { return e.err }

func replacementOccurred(err error) bool {
	var replaced *postReplaceError
	return errors.As(err, &replaced)
}

func createPrivateBackup(sourcePath string, content []byte, sourceMode os.FileMode, label string) (string, error) {
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
		defer func() {
			if !ok {
				file.Close()
				os.Remove(path)
			}
		}()
		if err := restrictPrivate(path, false); err != nil {
			return "", err
		}
		if _, err := file.Write(content); err != nil {
			return "", err
		}
		if err := file.Sync(); err != nil {
			return "", err
		}
		if err := file.Close(); err != nil {
			return "", err
		}
		if err := applyPrivateMode(path, sourceMode); err != nil {
			return "", err
		}
		if err := syncDirectory(dir); err != nil {
			return "", err
		}
		ok = true
		return path, nil
	}
	return "", errors.New("could not allocate unique backup path")
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
