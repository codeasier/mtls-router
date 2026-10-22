package routerenroll

import (
	"context"
	"database/sql"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"time"
)

var serialPattern = regexp.MustCompile(`^[0-9a-f]{1,40}$`)

// Admin can only list public metadata or commit revocation intent. It does not
// take the signing-master lease, load signing/token keys, issue certificates or
// expose a public network management API. SQLite serializes its short writes
// against the one signing/publication master.
type Admin struct {
	db      *sql.DB
	journal *journal
}

func OpenAdmin(directory, journalDir string) (*Admin, error) {
	info, err := os.Lstat(directory)
	if err != nil || !info.IsDir() || info.Mode().Perm() != 0700 || !privateOwner(info) {
		return nil, ErrStorage
	}
	path, err := filepath.Abs(filepath.Join(directory, "enroll.sqlite3"))
	if err != nil || privateRegularFile(path) != nil {
		return nil, ErrStorage
	}
	for _, name := range []string{path + "-wal", path + "-shm"} {
		if err := privateRegularFile(name); err != nil && !os.IsNotExist(err) {
			return nil, ErrStorage
		}
	}
	parameters := url.Values{"mode": {"rw"}, "_pragma": {"foreign_keys(1)", "busy_timeout(5000)", "synchronous(FULL)"}, "_txlock": {"immediate"}}
	db, err := sql.Open("sqlite", (&url.URL{Scheme: "file", Path: path, RawQuery: parameters.Encode()}).String())
	if err != nil {
		return nil, ErrStorage
	}
	db.SetMaxOpenConns(1)
	j, err := openJournal(directory, journalDir)
	if err != nil {
		_ = db.Close()
		return nil, err
	}
	if err := j.locked(context.Background(), func() error { return j.recover(context.Background(), db) }); err != nil {
		j.close()
		_ = db.Close()
		return nil, err
	}
	var version, application int
	if db.QueryRow("PRAGMA user_version").Scan(&version) != nil || db.QueryRow("PRAGMA application_id").Scan(&application) != nil || (version < 2 || version > 5) || application != 1129465137 {
		_ = db.Close()
		j.close()
		return nil, ErrStorage
	}
	return &Admin{db, j}, nil
}
func (a *Admin) Close() error {
	defer a.journal.close()
	if a.db.Close() != nil {
		return ErrStorage
	}
	return nil
}
func (a *Admin) List(ctx context.Context, limit int) ([]CertificateSummary, error) {
	var values []CertificateSummary
	err := a.journal.locked(ctx, func() error {
		if err := a.journal.recover(ctx, a.db); err != nil {
			return err
		}
		var err error
		values, err = listCertificates(ctx, a.db, limit)
		return err
	})
	return values, err
}
func (a *Admin) Revoke(ctx context.Context, target RevocationTarget, now time.Time) (RevocationResult, error) {
	return revoke(ctx, a, target, now)
}
func (a *Admin) transaction(ctx context.Context, operation func(*writeTx) error) error {
	return a.journal.transaction(ctx, a.db, operation)
}
