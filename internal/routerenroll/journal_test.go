//go:build unix

package routerenroll

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func databaseFiles(t *testing.T, s *Store) (state, key, journalDir string) {
	t.Helper()
	var path string
	if err := s.db.QueryRow("SELECT file FROM pragma_database_list WHERE name='main'").Scan(&path); err != nil {
		t.Fatal(err)
	}
	state = filepath.Dir(path)
	return state, filepath.Join(filepath.Dir(state), "independent-token-key"), testJournalDirectory(state)
}

func TestOldSnapshotReplaysUnpublishedRevocation(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	ctx := context.Background()
	issued, code := s.issueAuthorized(ctx, issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	state, key, journalDir := databaseFiles(t, s)
	admin, err := OpenAdmin(state, journalDir)
	if err != nil {
		t.Fatal(err)
	}
	backupDir := t.TempDir()
	if err := os.Chmod(backupDir, 0700); err != nil {
		t.Fatal(err)
	}
	snapshot := filepath.Join(backupDir, "before-revocation.sqlite3")
	manifest, err := admin.Backup(ctx, snapshot)
	if err != nil || manifest.Checkpoint.Sequence < 1 || len(manifest.SHA256) != 64 {
		t.Fatal("backup failed", err)
	}
	if _, err := admin.Backup(ctx, snapshot); err == nil {
		t.Fatal("backup overwritten")
	}
	if _, err := admin.Revoke(ctx, RevocationTarget{"serial", issued.Serial}, now); err != nil {
		t.Fatal(err)
	}
	var publications int
	if s.db.QueryRow("SELECT count(*) FROM crl_publications").Scan(&publications) != nil || publications != 0 {
		t.Fatal("test must cover an unpublished revocation")
	}
	_ = admin.Close()
	_ = s.Close()
	old, err := os.ReadFile(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(state, "enroll.sqlite3"), old, 0600); err != nil {
		t.Fatal(err)
	}
	restored, err := OpenStore(state, key, journalDir)
	if err != nil {
		t.Fatal("old projection was not safely caught up", err)
	}
	defer restored.Close()
	if _, code := restored.issueAuthorized(ctx, issuer, request, now); code != ClientCertRejected {
		t.Fatal("restore resurrected revoked result", code)
	}
	var checkpoint []byte
	if restored.db.QueryRow("SELECT value FROM metadata WHERE name='recovery_checkpoint'").Scan(&checkpoint) != nil {
		t.Fatal("checkpoint missing")
	}
	var actual RecoveryCheckpoint
	if json.Unmarshal(checkpoint, &actual) != nil || actual.Sequence <= manifest.Checkpoint.Sequence {
		t.Fatal("backup did not catch up")
	}
	info, err := os.Stat(snapshot)
	if err != nil || info.Mode().Perm() != 0600 {
		t.Fatal("backup permissions")
	}
}

func TestAuthorityCommitBeforeSQLiteCrashRecoversOriginalSerial(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	ctx := context.Background()
	state, key, journalDir := databaseFiles(t, s)
	crash := errors.New("injected-crash")
	s.journal.afterAppend = func() error { return crash }
	if _, code := s.issueAuthorized(ctx, issuer, request, now); code != ServiceUnavailable {
		t.Fatal("crash incorrectly reported success", code)
	}
	var count int
	if s.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 0 {
		t.Fatal("projection unexpectedly committed")
	}
	data, err := os.ReadFile(filepath.Join(journalDir, journalName))
	if err != nil {
		t.Fatal(err)
	}
	lines := bytes.Split(bytes.TrimSpace(data), []byte{'\n'})
	record, err := decodeRecord(lines[len(lines)-1])
	if err != nil {
		t.Fatal(err)
	}
	expectedSerial := ""
	for _, statement := range record.Statements {
		if strings.HasPrefix(statement.SQL, "INSERT INTO certificates(") {
			expectedSerial = statement.Args[1].Text
		}
	}
	if expectedSerial == "" {
		t.Fatal("durable original serial not recorded")
	}
	_ = s.Close()
	restored, err := OpenStore(state, key, journalDir)
	if err != nil {
		t.Fatal(err)
	}
	defer restored.Close()
	result, code := restored.issueAuthorized(ctx, issuer, request, now)
	if code != "" || result.Serial != expectedSerial {
		t.Fatal("crash recovery re-signed", code)
	}
	if restored.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 1 {
		t.Fatal("duplicate serial after recovery")
	}
}

func TestAuthorityOnlyRestoreRetainsEncryptedCredentials(t *testing.T) {
	s, _, _, _ := prepareLedger(t)
	state, key, journalDir := databaseFiles(t, s)
	_ = s.Close()
	if err := os.Remove(filepath.Join(state, "enroll.sqlite3")); err != nil {
		t.Fatal(err)
	}
	restored, err := OpenStore(state, key, journalDir)
	if err != nil {
		t.Fatal("complete authority replay failed", err)
	}
	defer restored.Close()
	var encrypted []byte
	var version, epoch int64
	var user string
	if restored.db.QueryRow("SELECT epoch,user_id,version,ciphertext FROM gitcode_tokens").Scan(&epoch, &user, &version, &encrypted) != nil {
		t.Fatal("token row lost")
	}
	var token GitCodeToken
	if restored.decrypt(encrypted, tokenAAD(epoch, user, version), &token) != nil || token.AccessToken != "gitcode-access-canary" {
		t.Fatal("encrypted credential did not recover")
	}
	data, err := os.ReadFile(filepath.Join(journalDir, journalName))
	if err != nil {
		t.Fatal(err)
	}
	for _, secret := range []string{"gitcode-access-canary", "gitcode-refresh-canary", "BEGIN PRIVATE KEY", "oauth-code-canary", "native-verifier-canary", "business-api-key-canary"} {
		if bytes.Contains(data, []byte(secret)) {
			t.Fatal("journal secret leak", secret)
		}
	}
}

func TestContinuityRejectsMissingMismatchedDamagedOrAheadJournal(t *testing.T) {
	for _, kind := range []string{"missing", "empty", "partial", "corrupt", "ahead", "wrong-store"} {
		t.Run(kind, func(t *testing.T) {
			s, _, _, _ := prepareLedger(t)
			state, key, journalDir := databaseFiles(t, s)
			_ = s.Close()
			path := filepath.Join(journalDir, journalName)
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			switch kind {
			case "missing":
				if os.Remove(path) != nil {
					t.Fatal("remove journal")
				}
			case "empty":
				data = nil
			case "partial":
				data = data[:len(data)-1]
			case "corrupt":
				data[len(data)-5] ^= 1
			case "ahead":
				data = data[:bytes.IndexByte(data, '\n')+1]
			case "wrong-store":
				journalDir = filepath.Join(t.TempDir(), "other-journal")
				if InitJournal(journalDir) != nil {
					t.Fatal("init other")
				}
			}
			if kind != "missing" && kind != "wrong-store" {
				if os.WriteFile(path, data, 0600) != nil {
					t.Fatal("corrupt fixture")
				}
			}
			if restored, err := OpenStore(state, key, journalDir); restored != nil || !errors.Is(err, ErrContinuity) {
				if restored != nil {
					_ = restored.Close()
				}
				t.Fatalf("unsafe recovery accepted: %v", err)
			}
			if kind == "missing" {
				if _, err := os.Stat(path); !os.IsNotExist(err) {
					t.Fatal("missing journal was recreated")
				}
			}
		})
	}
}

func TestSigningLeaseFollowsAuthorityAndAdminCommitsSerialize(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	state, key, journalDir := databaseFiles(t, s)
	if other, err := OpenStore(filepath.Join(t.TempDir(), "other-state"), key, journalDir); other != nil || !errors.Is(err, ErrAlreadyRunning) {
		t.Fatal("changed state path bypassed signing lease", err)
	}
	issued, code := s.issueAuthorized(context.Background(), issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	var group sync.WaitGroup
	for range 8 {
		group.Go(func() {
			admin, err := OpenAdmin(state, journalDir)
			if err != nil {
				t.Error(err)
				return
			}
			defer admin.Close()
			if _, err := admin.Revoke(context.Background(), RevocationTarget{"serial", issued.Serial}, now); err != nil {
				t.Error(err)
			}
		})
	}
	group.Wait()
	if !s.Ready(context.Background()) {
		t.Fatal("parallel admin corrupted journal")
	}
	if _, code := s.issueAuthorized(context.Background(), issuer, request, now); code != ClientCertRejected {
		t.Fatal("parallel revoke lost", code)
	}
}

func TestIndependentJournalConfigurationAndExplicitInit(t *testing.T) {
	state, key := testStoreFiles(t)
	if _, err := OpenStore(state, key, ""); !errors.Is(err, ErrContinuity) {
		t.Fatal("implicit authority accepted", err)
	}
	nested := filepath.Join(state, "backup", "journal")
	if InitJournal(nested) != nil {
		t.Fatal("init nested fixture")
	}
	if _, err := OpenStore(state, key, nested); !errors.Is(err, ErrContinuity) {
		t.Fatal("authority inside state backup accepted", err)
	}
	if InitJournal(testJournalDirectory(state)) == nil {
		t.Fatal("existing authority overwritten")
	}
}

func TestWholeRecordTruncationWithOldSnapshotIsRejected(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	state, key, journalDir := databaseFiles(t, s)
	issued, code := s.issueAuthorized(context.Background(), issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	backup := filepath.Join(t.TempDir(), "old.sqlite3")
	if _, err := s.db.Exec("VACUUM INTO ?", backup); err != nil {
		t.Fatal(err)
	}
	oldLog, err := os.ReadFile(filepath.Join(journalDir, journalName))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := revoke(context.Background(), s, RevocationTarget{"serial", issued.Serial}, now); err != nil {
		t.Fatal(err)
	}
	_ = s.Close()
	oldDB, err := os.ReadFile(backup)
	if err != nil {
		t.Fatal(err)
	}
	if os.WriteFile(filepath.Join(state, "enroll.sqlite3"), oldDB, 0600) != nil || os.WriteFile(filepath.Join(journalDir, journalName), oldLog, 0600) != nil {
		t.Fatal("restore probe")
	}
	// head.json is the independently retained latest high-water mark.
	if value, err := OpenStore(state, key, journalDir); value != nil || !errors.Is(err, ErrContinuity) {
		t.Fatal("whole-record loss plus old DB bypassed continuity", err)
	}
}

func TestAnchoredSchemaTwoMigratesThroughJournal(t *testing.T) {
	s, _, _, _ := prepareLedger(t)
	state, key, journalDir := databaseFiles(t, s)
	if err := s.transaction(context.Background(), func(tx *writeTx) error {
		_, err := tx.ExecContext(context.Background(), "ALTER TABLE enrollment_transactions DROP COLUMN csrf_hash; ALTER TABLE enrollment_transactions DROP COLUMN account_json; ALTER TABLE users DROP COLUMN verification_attempted_at; ALTER TABLE gateway_acknowledgments DROP COLUMN root_digest; PRAGMA user_version=2")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	_ = s.Close()
	reopened, err := OpenStore(state, key, journalDir)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	var version int
	if reopened.db.QueryRow("PRAGMA user_version").Scan(&version) != nil || version != 5 {
		t.Fatal("anchored schema did not migrate")
	}
	rows, err := reopened.db.Query("SELECT csrf_hash,account_json FROM enrollment_transactions LIMIT 0")
	if err != nil {
		t.Fatal("browser columns missing", err)
	}
	_ = rows.Close()
}
