package routerenroll

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
	"path/filepath"
)

type BackupSummary struct {
	Checkpoint RecoveryCheckpoint `json:"checkpoint"`
	SHA256     string             `json:"sha256"`
}

// Backup holds the shared commit gate across catch-up, VACUUM and manifest
// generation. Only an exclusive, private output is allowed; existing backups
// are not replaced. The latest independent journal is NOT part of this backup.
func (a *Admin) Backup(ctx context.Context, path string) (BackupSummary, error) {
	var result BackupSummary
	err := a.journal.locked(ctx, func() error {
		if err := a.journal.recover(ctx, a.db); err != nil {
			return err
		}
		dir := filepath.Dir(path)
		info, err := os.Lstat(dir)
		if err != nil || !info.IsDir() || info.Mode().Perm() != 0700 || !privateOwner(info) {
			return ErrStorage
		}
		file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_RDWR, 0600)
		if err != nil {
			return ErrStorage
		}
		if file.Close() != nil {
			return ErrStorage
		}
		complete := false
		defer func() {
			if !complete {
				_ = os.Remove(path)
			}
		}()
		if _, err := a.db.ExecContext(ctx, "VACUUM INTO ?", path); err != nil {
			return ErrStorage
		}
		if privateRegularFile(path) != nil {
			return ErrStorage
		}
		file, err = os.OpenFile(path, os.O_RDWR, 0)
		if err != nil {
			return ErrStorage
		}
		defer file.Close()
		if file.Sync() != nil || syncDirectory(dir) != nil {
			return ErrStorage
		}
		hash := sha256.New()
		if _, err := io.Copy(hash, file); err != nil {
			return ErrStorage
		}
		result = BackupSummary{a.journal.head, hex.EncodeToString(hash.Sum(nil))}
		complete = true
		return nil
	})
	return result, err
}
