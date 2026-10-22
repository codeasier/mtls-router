//go:build unix

package routerenroll

import (
	"context"
	"crypto/elliptic"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func prepareLedger(t *testing.T) (*Store, *Issuer, authorizedIssue, time.Time) {
	t.Helper()
	cfg := testConfig(t)
	s, err := OpenStore(cfg.StateDir, cfg.TokenKeyFile, cfg.JournalDir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	ctx := context.Background()
	p, err := s.ActivatePolicy(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now().UTC().Truncate(time.Second)
	identity := fixtureIdentity()
	account := Account{ID: identity.UserID, Username: "account-label-canary", Organization: Organization{identity.OrganizationID, cfg.OrgPath}}
	err = s.transaction(ctx, func(tx *writeTx) error {
		if err := bindOrganization(ctx, tx, p.Epoch, account.Organization); err != nil {
			return err
		}
		return s.saveGitCodeToken(ctx, tx, p.Epoch, account, GitCodeToken{AccessToken: "gitcode-access-canary", RefreshToken: "gitcode-refresh-canary"})
	})
	if err != nil {
		t.Fatal(err)
	}
	issuer, _, _ := fixtureIssuer(t, now)
	request := authorizedIssue{OperationID: "67260b59-9e9e-4cca-8db5-67411c8f243c", Epoch: p.Epoch, Account: account, DeviceID: identity.DeviceID, Kind: "linux_cli", Label: "device-label-canary", CSRPEM: testCSR(t, elliptic.P256())}
	return s, issuer, request, now
}

func TestLedgerConcurrentResultRecoveryAndDeviceBinding(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	ctx := context.Background()
	results := make(chan IssuedCertificate, 8)
	var group sync.WaitGroup
	for range 8 {
		group.Go(func() {
			value, code := s.issueAuthorized(ctx, issuer, request, now)
			if code != "" {
				t.Error(code)
			}
			results <- value
		})
	}
	group.Wait()
	close(results)
	serial := ""
	for result := range results {
		if serial == "" {
			serial = result.Serial
		}
		if serial == "" || result.Serial != serial {
			t.Fatal("duplicate signing results")
		}
	}
	var count int
	if s.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 1 {
		t.Fatal("duplicate certificates")
	}
	value, code := s.issueAuthorized(ctx, issuer, request, now.Add(RecoveryWindow-time.Second))
	if code != "" || value.Serial != serial {
		t.Fatal("response recovery changed serial", code)
	}
	if _, code := s.issueAuthorized(ctx, issuer, request, now.Add(RecoveryWindow+time.Second)); code != ReauthRequired {
		t.Fatal("recovery window unbounded", code)
	}
	rebound := request
	rebound.CSRPEM = testCSR(t, elliptic.P256())
	if _, code := s.issueAuthorized(ctx, issuer, rebound, now); code != OperationConflict {
		t.Fatal("operation rebound to new CSR", code)
	}
	other := request
	other.Account.ID = "other-user"
	other.OperationID = "95d467d4-cbab-4ae0-93c6-cc653b0438f5"
	if err := s.transaction(ctx, func(tx *writeTx) error {
		return s.saveGitCodeToken(ctx, tx, request.Epoch, other.Account, GitCodeToken{AccessToken: "other-token-canary"})
	}); err != nil {
		t.Fatal(err)
	}
	if _, code := s.issueAuthorized(ctx, issuer, other, now); code != ClientCertRejected {
		t.Fatal("device rebound across users", code)
	}
}

func TestProtectedAdminRevocationDoesNotAffectOtherDevice(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	ctx := context.Background()
	one, code := s.issueAuthorized(ctx, issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	other := request
	other.DeviceID = "b5c2fb58-d5e8-4dc2-9b04-38209df797d1"
	other.OperationID = "95d467d4-cbab-4ae0-93c6-cc653b0438f5"
	other.CSRPEM = testCSR(t, elliptic.P256())
	two, code := s.issueAuthorized(ctx, issuer, other, now)
	if code != "" {
		t.Fatal(code)
	}
	var databasePath string
	if err := s.db.QueryRow("SELECT file FROM pragma_database_list WHERE name='main'").Scan(&databasePath); err != nil {
		t.Fatal(err)
	}
	admin, err := OpenAdmin(filepath.Dir(databasePath), testJournalDirectory(filepath.Dir(databasePath)))
	if err != nil {
		t.Fatal(err)
	}
	defer admin.Close()
	result, err := admin.Revoke(ctx, RevocationTarget{"device", request.DeviceID}, now)
	if err != nil || result.Matched != 1 || result.Publication != "pending" {
		t.Fatal("false global revocation claim", err)
	}
	if _, code := s.issueAuthorized(ctx, issuer, request, now); code != ClientCertRejected {
		t.Fatal("revoked device result restored", code)
	}
	value, code := s.issueAuthorized(ctx, issuer, other, now)
	if code != "" || value.Serial != two.Serial {
		t.Fatal("other device changed", code)
	}
	list, err := admin.List(ctx, 100)
	if err != nil || len(list) != 2 {
		t.Fatal(err)
	}
	for _, item := range list {
		if item.Serial == one.Serial && !item.RevocationDecided {
			t.Fatal("missing local decision")
		}
		if item.Serial == two.Serial && item.RevocationDecided {
			t.Fatal("other device revoked")
		}
	}
	encoded, _ := json.Marshal(list)
	for _, secret := range []string{"canary", "PRIVATE KEY", "certificate_pem", "access_token", "refresh_token"} {
		if strings.Contains(string(encoded), secret) {
			t.Fatal("admin metadata leaked", secret)
		}
	}
	if _, err := admin.Revoke(ctx, RevocationTarget{"user", "gitcode-secret-canary"}, now); err == nil {
		t.Fatal("unknown target accepted/audited")
	}
	rows, err := s.db.Query("SELECT action,user_id,device_id,serial FROM audit_events")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	for rows.Next() {
		var action, user, device, serial string
		if rows.Scan(&action, &user, &device, &serial) != nil {
			t.Fatal("audit read")
		}
		if strings.Contains(action+user+device+serial, "canary") {
			t.Fatal("secret or label in audit")
		}
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
}

func TestAdminRefusesMissingPublicOrSymlinkState(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "missing")
	if _, err := OpenAdmin(dir, testJournalDirectory(dir)); err == nil {
		t.Fatal("created admin state")
	}
	if _, err := os.Stat(dir); !os.IsNotExist(err) {
		t.Fatal("admin created missing directory")
	}
	dir = t.TempDir()
	if err := os.Chmod(dir, 0755); err != nil {
		t.Fatal(err)
	}
	if _, err := OpenAdmin(dir, testJournalDirectory(dir)); err == nil {
		t.Fatal("public state accepted")
	}
}

func TestSerialAndUserRevocationAndRestart(t *testing.T) {
	s, issuer, request, now := prepareLedger(t)
	ctx := context.Background()
	one, code := s.issueAuthorized(ctx, issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	if _, err := revoke(ctx, s, RevocationTarget{"serial", one.Serial}, now); err != nil {
		t.Fatal(err)
	}
	var disabled int
	if s.db.QueryRow("SELECT disabled FROM devices WHERE id=?", request.DeviceID).Scan(&disabled) != nil || disabled != 0 {
		t.Fatal("serial revoke disabled entire device")
	}
	if _, code := s.issueAuthorized(ctx, issuer, request, now); code != ClientCertRejected {
		t.Fatal("revoked result recovered", code)
	}
	other := request
	other.DeviceID = "b5c2fb58-d5e8-4dc2-9b04-38209df797d1"
	other.OperationID = "95d467d4-cbab-4ae0-93c6-cc653b0438f5"
	other.CSRPEM = testCSR(t, elliptic.P256())
	if _, code := s.issueAuthorized(ctx, issuer, other, now); code != "" {
		t.Fatal(code)
	}
	if _, err := revoke(ctx, s, RevocationTarget{"user", request.Account.ID}, now); err != nil {
		t.Fatal(err)
	}
	if _, code := s.issueAuthorized(ctx, issuer, other, now); code != ClientCertRejected {
		t.Fatal("user revoke omitted device", code)
	}
	var databasePath string
	if err := s.db.QueryRow("SELECT file FROM pragma_database_list WHERE name='main'").Scan(&databasePath); err != nil {
		t.Fatal(err)
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	reopened, err := OpenStore(filepath.Dir(databasePath), filepath.Join(filepath.Dir(filepath.Dir(databasePath)), "independent-token-key"), testJournalDirectory(filepath.Dir(databasePath)))
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	if _, code := reopened.issueAuthorized(ctx, issuer, request, now); code != ClientCertRejected {
		t.Fatal("restart forgot revocation", code)
	}
}
