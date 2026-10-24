//go:build unix

package routerenroll

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func testStoreFiles(t *testing.T) (string, string) {
	t.Helper()
	directory := t.TempDir()
	key := filepath.Join(directory, "independent-token-key")
	if err := os.WriteFile(key, bytes.Repeat([]byte{42}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	if err := InitJournal(filepath.Join(directory, "independent-journal")); err != nil {
		t.Fatal(err)
	}
	return filepath.Join(directory, "state"), key
}
func testJournalDirectory(state string) string {
	return filepath.Join(filepath.Dir(state), "independent-journal")
}

func TestStoreRestartEncryptionAndSingleWriter(t *testing.T) {
	directory, key := testStoreFiles(t)
	s, err := OpenStore(directory, key, testJournalDirectory(directory))
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if !s.Ready(ctx) {
		t.Fatal("not ready")
	}
	if _, err := OpenStore(directory, key, testJournalDirectory(directory)); !errors.Is(err, ErrAlreadyRunning) {
		t.Fatal("second writer allowed", err)
	}
	secret := map[string]string{"access_token": "gitcode-access-canary", "refresh_token": "gitcode-refresh-canary"}
	sealed, err := s.encrypt(secret, tokenAAD(3, "user-42", 1))
	if err != nil || bytes.Contains(sealed, []byte("canary")) {
		t.Fatal("encryption failed")
	}
	err = s.transaction(ctx, func(tx *writeTx) error {
		_, err := tx.ExecContext(ctx, `INSERT INTO users(epoch,user_id,username) VALUES(3,'user-42','fixture'); INSERT INTO gitcode_tokens(epoch,user_id,version,ciphertext) VALUES(3,'user-42',1,?)`, sealed)
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	s, err = OpenStore(directory, key, testJournalDirectory(directory))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	var restored []byte
	if err := s.db.QueryRowContext(ctx, "SELECT ciphertext FROM gitcode_tokens WHERE user_id='user-42'").Scan(&restored); err != nil {
		t.Fatal(err)
	}
	var plain map[string]string
	if err := s.decrypt(restored, tokenAAD(3, "user-42", 1), &plain); err != nil || plain["access_token"] != secret["access_token"] {
		t.Fatal("restart lost token")
	}
	for _, aad := range []string{tokenAAD(4, "user-42", 1), tokenAAD(3, "user-43", 1), tokenAAD(3, "user-42", 2)} {
		if s.decrypt(restored, aad, &plain) == nil {
			t.Fatal("ciphertext substitution accepted")
		}
	}
	restored[len(restored)-1] ^= 1
	if s.decrypt(restored, tokenAAD(3, "user-42", 1), &plain) == nil {
		t.Fatal("tampering accepted")
	}
	for _, path := range []string{directory, filepath.Join(directory, "enroll.sqlite3"), filepath.Join(directory, "enroll.sqlite3-wal"), filepath.Join(directory, "enroll.sqlite3-shm")} {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		want := os.FileMode(0600)
		if info.IsDir() {
			want = 0700
		}
		if info.Mode().Perm() != want {
			t.Fatalf("permissions %s: %o", path, info.Mode().Perm())
		}
	}
}

func TestStoreRollbackUnsupportedSchemaAndWrongKey(t *testing.T) {
	directory, key := testStoreFiles(t)
	s, err := OpenStore(directory, key, testJournalDirectory(directory))
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	abort := errors.New("test-abort")
	if err := s.transaction(ctx, func(tx *writeTx) error {
		_, err := tx.ExecContext(ctx, "INSERT INTO users(epoch,user_id,username) VALUES(1,'rollback','fixture')")
		if err != nil {
			return err
		}
		return abort
	}); !errors.Is(err, abort) {
		t.Fatal(err)
	}
	var count int
	if err := s.db.QueryRow("SELECT count(*) FROM users").Scan(&count); err != nil || count != 0 {
		t.Fatal("transaction did not roll back")
	}
	_ = s.Close()
	wrongKey := filepath.Join(filepath.Dir(key), "wrong-key")
	if err := os.WriteFile(wrongKey, bytes.Repeat([]byte{99}, 32), 0600); err != nil {
		t.Fatal(err)
	}
	if s, err := OpenStore(directory, wrongKey, testJournalDirectory(directory)); s != nil || !errors.Is(err, ErrStorage) {
		t.Fatal("wrong key accepted", err)
	}
	s, err = OpenStore(directory, key, testJournalDirectory(directory))
	if err != nil {
		t.Fatal("failed open leaked writer lock", err)
	}
	if _, err := s.db.Exec("PRAGMA user_version=999"); err != nil {
		t.Fatal(err)
	}
	_ = s.Close()
	if s, err := OpenStore(directory, key, testJournalDirectory(directory)); s != nil || !errors.Is(err, ErrContinuity) {
		t.Fatal("unknown schema reset/accepted", err)
	}
}

func TestStoreRejectsUnsafeFiles(t *testing.T) {
	directory, key := testStoreFiles(t)
	if err := os.Chmod(key, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err := OpenStore(directory, key, testJournalDirectory(directory)); err == nil {
		t.Fatal("public encryption key accepted")
	}
	if err := os.Chmod(key, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(key, filepath.Join(directory, "enroll.sqlite3")); err != nil {
		t.Fatal(err)
	}
	if _, err := OpenStore(directory, key, testJournalDirectory(directory)); err == nil {
		t.Fatal("symlink database accepted")
	}
}
