package routerenroll

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"

	_ "modernc.org/sqlite"
)

var (
	ErrStorage        = errors.New("enroll_storage_unavailable")
	ErrAlreadyRunning = errors.New("enroll_writer_already_running")
)

// Store is a single signing/publication writer, backed by durable SQLite state.
// Public certificates and encrypted GitCode credentials are separate records.
type Store struct {
	db         *sql.DB
	aead       cipher.AEAD
	lock       *os.File
	journal    *journal
	masterLock *os.File
}

// OpenStore requires an independently provisioned 32-byte private encryption
// key. It never generates/replaces keys, silently resets state or starts a second
// writer. The control-plane service is supported on Unix service hosts.
func OpenStore(directory, keyFile, journalDir string) (_ *Store, result error) {
	key, err := readPrivateFile(keyFile, 32)
	if err != nil || len(key) != 32 {
		return nil, ErrStorage
	}
	defer clear(key)
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, ErrStorage
	}
	aead, err := cipher.NewGCM(block)
	if err != nil || privateDirectory(directory) != nil {
		return nil, ErrStorage
	}
	lock, err := acquireWriterLock(filepath.Join(directory, "writer.lock"))
	if err != nil {
		return nil, err
	}
	defer func() {
		if result != nil {
			_ = lock.Close()
		}
	}()
	journal, err := openJournal(directory, journalDir)
	if err != nil {
		return nil, err
	}
	defer func() {
		if result != nil {
			journal.close()
		}
	}()
	master, err := acquireWriterLock(filepath.Join(journalDir, "signer.lock"))
	if err != nil {
		return nil, err
	}
	defer func() {
		if result != nil {
			_ = master.Close()
		}
	}()
	path, err := filepath.Abs(filepath.Join(directory, "enroll.sqlite3"))
	if err != nil {
		return nil, ErrStorage
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_RDWR, 0600)
	if err == nil {
		_ = file.Close()
	} else if !errors.Is(err, os.ErrExist) {
		return nil, ErrStorage
	}
	for _, name := range []string{path, path + "-wal", path + "-shm"} {
		if err := privateRegularFile(name); err != nil && !(name != path && errors.Is(err, os.ErrNotExist)) {
			return nil, ErrStorage
		}
	}
	parameters := url.Values{"_pragma": {"foreign_keys(1)", "busy_timeout(5000)", "synchronous(FULL)"}, "_txlock": {"immediate"}}
	dsn := (&url.URL{Scheme: "file", Path: path, RawQuery: parameters.Encode()}).String()
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, ErrStorage
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	store := &Store{db: db, aead: aead, lock: lock, journal: journal, masterLock: master}
	if err := store.migrate(); err != nil {
		_ = db.Close()
		return nil, err
	}
	return store, nil
}

func (s *Store) Close() error {
	dbErr := s.db.Close()
	lockErr := s.lock.Close()
	_ = s.masterLock.Close()
	s.journal.close()
	if dbErr != nil || lockErr != nil {
		return ErrStorage
	}
	return nil
}
func (s *Store) Ready(ctx context.Context) bool {
	return s.journal.locked(ctx, func() error { return s.journal.recover(ctx, s.db) }) == nil
}

const schema = `
CREATE TABLE metadata (name TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE organization_policy (singleton INTEGER PRIMARY KEY CHECK(singleton=1), path TEXT NOT NULL, expected_id TEXT NOT NULL, bound_id TEXT NOT NULL, fingerprint TEXT NOT NULL, epoch INTEGER NOT NULL CHECK(epoch>0));
CREATE TABLE users (epoch INTEGER NOT NULL, user_id TEXT NOT NULL, username TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1, verified_at INTEGER NOT NULL DEFAULT 0, verification_status TEXT NOT NULL DEFAULT 'unknown', verification_attempted_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(epoch,user_id));
CREATE TABLE gitcode_tokens (epoch INTEGER NOT NULL, user_id TEXT NOT NULL, version INTEGER NOT NULL CHECK(version>0), ciphertext BLOB NOT NULL, PRIMARY KEY(epoch,user_id), FOREIGN KEY(epoch,user_id) REFERENCES users(epoch,user_id));
CREATE TABLE devices (id TEXT PRIMARY KEY, epoch INTEGER NOT NULL, user_id TEXT NOT NULL, kind TEXT NOT NULL, label TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1, disabled INTEGER NOT NULL DEFAULT 0 CHECK(disabled IN (0,1)), FOREIGN KEY(epoch,user_id) REFERENCES users(epoch,user_id));
CREATE TABLE enrollment_transactions (id TEXT PRIMARY KEY, epoch INTEGER NOT NULL, device_id TEXT NOT NULL, kind TEXT NOT NULL, label TEXT NOT NULL, csr BLOB NOT NULL, csr_digest TEXT NOT NULL, spki_digest TEXT NOT NULL, challenge TEXT NOT NULL, short_code TEXT NOT NULL, expires_at INTEGER NOT NULL, next_poll INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL, browser_hash TEXT NOT NULL DEFAULT '', csrf_hash TEXT NOT NULL DEFAULT '', state_hash TEXT UNIQUE, state_used INTEGER NOT NULL DEFAULT 0, user_id TEXT NOT NULL DEFAULT '', account_json TEXT NOT NULL DEFAULT '', failure_code TEXT NOT NULL DEFAULT '', version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE issuance_requests (id TEXT PRIMARY KEY, transaction_id TEXT UNIQUE REFERENCES enrollment_transactions(id), device_id TEXT NOT NULL REFERENCES devices(id), csr_digest TEXT NOT NULL, spki_digest TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1, committed_at INTEGER NOT NULL, recovery_until INTEGER NOT NULL);
CREATE TABLE certificates (issuer_id TEXT NOT NULL, serial TEXT NOT NULL UNIQUE, request_id TEXT NOT NULL UNIQUE REFERENCES issuance_requests(id), device_id TEXT NOT NULL REFERENCES devices(id), epoch INTEGER NOT NULL, organization_id TEXT NOT NULL, user_id TEXT NOT NULL, csr_digest TEXT NOT NULL, spki_digest TEXT NOT NULL, certificate_pem TEXT NOT NULL, chain_pem TEXT NOT NULL, not_before INTEGER NOT NULL, not_after INTEGER NOT NULL, version INTEGER NOT NULL DEFAULT 1, PRIMARY KEY(issuer_id,serial), FOREIGN KEY(epoch,user_id) REFERENCES users(epoch,user_id));
CREATE TABLE renewal_operations (id TEXT PRIMARY KEY, issuer_id TEXT NOT NULL, predecessor_serial TEXT NOT NULL, csr_digest TEXT NOT NULL, spki_digest TEXT NOT NULL, request_id TEXT NOT NULL UNIQUE REFERENCES issuance_requests(id), handover_until INTEGER NOT NULL, version INTEGER NOT NULL DEFAULT 1, UNIQUE(issuer_id,predecessor_serial), FOREIGN KEY(issuer_id,predecessor_serial) REFERENCES certificates(issuer_id,serial));
CREATE TABLE revocations (issuer_id TEXT NOT NULL, serial TEXT NOT NULL, decided_at INTEGER NOT NULL, reason TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1, PRIMARY KEY(issuer_id,serial), FOREIGN KEY(issuer_id,serial) REFERENCES certificates(issuer_id,serial));
CREATE TABLE crl_publications (issuer_id TEXT NOT NULL, number INTEGER NOT NULL CHECK(number>0), digest TEXT NOT NULL, crl_der BLOB NOT NULL, this_update INTEGER NOT NULL, next_update INTEGER NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY(issuer_id,number));
CREATE TABLE gateway_acknowledgments (gateway_id TEXT NOT NULL, issuer_id TEXT NOT NULL, number INTEGER NOT NULL, digest TEXT NOT NULL, root_digest TEXT NOT NULL DEFAULT '', loaded_at INTEGER NOT NULL, PRIMARY KEY(gateway_id,issuer_id), FOREIGN KEY(issuer_id,number) REFERENCES crl_publications(issuer_id,number));
CREATE TABLE audit_events (id INTEGER PRIMARY KEY, created_at INTEGER NOT NULL, action TEXT NOT NULL, epoch INTEGER NOT NULL, user_id TEXT NOT NULL DEFAULT '', device_id TEXT NOT NULL DEFAULT '', serial TEXT NOT NULL DEFAULT '');
CREATE INDEX enrollment_expiry ON enrollment_transactions(epoch,status,expires_at);
CREATE INDEX devices_user ON devices(epoch,user_id);
CREATE INDEX certificates_user ON certificates(epoch,user_id,not_after);
CREATE INDEX certificates_device ON certificates(device_id,not_after);
CREATE INDEX revocations_decision ON revocations(decided_at);
PRAGMA application_id=1129465137;
PRAGMA user_version=5;
`

func (s *Store) migrate() error {
	ctx := context.Background()
	if _, err := s.db.ExecContext(ctx, "PRAGMA journal_mode=WAL"); err != nil {
		return ErrStorage
	}
	return s.transaction(ctx, func(tx *writeTx) error {
		var version, application int
		if tx.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version) != nil || tx.QueryRowContext(ctx, "PRAGMA application_id").Scan(&application) != nil {
			return ErrStorage
		}
		if version == 0 && application == 0 {
			if _, err := tx.ExecContext(ctx, schema); err != nil {
				return ErrStorage
			}
			sealed, err := s.encrypt("enroll-v1", "enroll:key-check:v1")
			if err != nil {
				return err
			}
			_, err = tx.ExecContext(ctx, "INSERT INTO metadata(name,value) VALUES('encryption_check',?)", sealed)
			return err
		}
		if version == 2 && application == 1129465137 {
			if _, err := tx.ExecContext(ctx, "ALTER TABLE enrollment_transactions ADD COLUMN csrf_hash TEXT NOT NULL DEFAULT ''; ALTER TABLE enrollment_transactions ADD COLUMN account_json TEXT NOT NULL DEFAULT ''; PRAGMA user_version=3"); err != nil {
				return ErrStorage
			}
			version = 3
		}
		if version == 3 && application == 1129465137 {
			if _, err := tx.ExecContext(ctx, "ALTER TABLE users ADD COLUMN verification_attempted_at INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=4"); err != nil {
				return ErrStorage
			}
			version = 4
		}
		if version == 4 && application == 1129465137 {
			if _, err := tx.ExecContext(ctx, "ALTER TABLE gateway_acknowledgments ADD COLUMN root_digest TEXT NOT NULL DEFAULT ''; PRAGMA user_version=5"); err != nil {
				return ErrStorage
			}
			version = 5
		}
		if version != 5 || application != 1129465137 {
			return ErrStorage
		}
		var encrypted []byte
		if tx.QueryRowContext(ctx, "SELECT value FROM metadata WHERE name='encryption_check'").Scan(&encrypted) != nil {
			return ErrStorage
		}
		var check string
		if s.decrypt(encrypted, "enroll:key-check:v1", &check) != nil || check != "enroll-v1" {
			return ErrStorage
		}
		return nil
	})
}

func (s *Store) transaction(ctx context.Context, operation func(*writeTx) error) error {
	return s.journal.transaction(ctx, s.db, operation)
}

func (s *Store) encrypt(value any, associated string) ([]byte, error) {
	plain, err := json.Marshal(value)
	if err != nil {
		return nil, ErrStorage
	}
	defer clear(plain)
	nonce := make([]byte, s.aead.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return nil, ErrStorage
	}
	return s.aead.Seal(nonce, nonce, plain, []byte(associated)), nil
}
func (s *Store) decrypt(encrypted []byte, associated string, target any) error {
	n := s.aead.NonceSize()
	if len(encrypted) < n {
		return ErrStorage
	}
	plain, err := s.aead.Open(nil, encrypted[:n], encrypted[n:], []byte(associated))
	if err != nil {
		return ErrStorage
	}
	defer clear(plain)
	if json.Unmarshal(plain, target) != nil {
		return ErrStorage
	}
	return nil
}

func tokenAAD(epoch int64, userID string, version int64) string {
	return fmt.Sprintf("gitcode:v1:%d:%s:%d", epoch, userID, version)
}

func privateDirectory(path string) error {
	if err := os.MkdirAll(path, 0700); err != nil {
		return ErrStorage
	}
	info, err := os.Lstat(path)
	if err != nil || !info.IsDir() || !privateOwner(info) {
		return ErrStorage
	}
	if os.Chmod(path, 0700) != nil {
		return ErrStorage
	}
	return nil
}
func privateRegularFile(path string) error {
	info, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || !privateOwner(info) {
		return ErrStorage
	}
	return nil
}
func readPrivateFile(path string, limit int64) ([]byte, error) {
	if privateRegularFile(path) != nil {
		return nil, ErrStorage
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, ErrStorage
	}
	defer func() { _ = file.Close() }()
	value, err := io.ReadAll(io.LimitReader(file, limit+1))
	if err != nil || int64(len(value)) > limit || len(value) == 0 {
		clear(value)
		return nil, ErrStorage
	}
	return value, nil
}
