//go:build unix

package routerenroll

import (
	"bytes"
	"context"
	"crypto/elliptic"
	"encoding/json"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func nativeFixture(t *testing.T) (*Service, NativeStart, string) {
	t.Helper()
	store, issuer, _, now := prepareLedger(t)
	service := NewService(store, nil, issuer)
	service.now = func() time.Time { return now }
	verifier, err := randomProof()
	if err != nil {
		t.Fatal(err)
	}
	request := NativeStart{ProtocolVersion, testCSR(t, elliptic.P256()), "b5c2fb58-d5e8-4dc2-9b04-38209df797d1", "linux_cli", "fixture-native", Challenge(verifier), "S256"}
	return service, request, verifier
}

func TestNativeProofDoesNotConsumeOrCancelAnotherTransaction(t *testing.T) {
	s, request, verifier := nativeFixture(t)
	ctx := context.Background()
	started, code := s.start(ctx, request)
	if code != "" || started.ExpiresIn != 600 || started.Interval != 5 || len(started.ShortCode) != 9 || started.SPKIDigest == "" {
		t.Fatal("bad native start", code)
	}
	state, err := readNative(ctx, s.store.db, started.Transaction)
	if err != nil || state.SPKIDigest != started.SPKIDigest || state.CSRPEM != request.CSRPEM || state.DeviceID != request.DeviceID {
		t.Fatal("binding was not persisted", err)
	}
	other, _ := randomProof()
	bad := NativeProof{started.Transaction, other}
	if _, code := s.poll(ctx, bad); code != InvalidProof {
		t.Fatal("wrong proof polled", code)
	}
	if code := s.cancel(ctx, bad); code != InvalidProof {
		t.Fatal("wrong proof cancelled", code)
	}
	unchanged, err := readNative(ctx, s.store.db, started.Transaction)
	if err != nil || unchanged.Version != state.Version || unchanged.NextPoll != 0 || unchanged.Status != "pending" {
		t.Fatal("bad proof mutated transaction")
	}
	proof := NativeProof{started.Transaction, verifier}
	if result, code := s.poll(ctx, proof); code != "" || result.Status != "pending" {
		t.Fatal(code)
	}
	if _, code := s.poll(ctx, proof); code != SlowDown {
		t.Fatal("poll budget absent", code)
	}
	if code := s.cancel(ctx, proof); code != "" {
		t.Fatal(code)
	}
	if _, code := s.poll(ctx, proof); code != EnrollmentCancelled {
		t.Fatal("cancel not durable", code)
	}
	_, _, journal := databaseFiles(t, s.store)
	log, err := os.ReadFile(filepath.Join(journal, journalName))
	if err != nil || bytes.Contains(log, []byte(verifier)) {
		t.Fatal("verifier persisted in authority", err)
	}
}

func TestPollReturnsOnlyClosedFailureCodes(t *testing.T) {
	s, request, verifier := nativeFixture(t)
	ctx := context.Background()
	started, code := s.start(ctx, request)
	if code != "" {
		t.Fatal(code)
	}
	proof := NativeProof{started.Transaction, verifier}
	for _, test := range []struct{ status, stored, want string }{
		{"failed", ServiceUnavailable, ServiceUnavailable},
		{"failed", ReauthRequired, ReauthRequired},
		{"failed", "oauth-token-canary", ServiceUnavailable},
		{"denied", MembershipRequired, MembershipRequired},
		{"denied", ReauthRequired, ReauthRequired},
		{"denied", "account-canary", EnrollmentDenied},
	} {
		if _, err := s.store.db.ExecContext(ctx, "UPDATE enrollment_transactions SET status=?,failure_code=?,next_poll=0 WHERE id=?", test.status, test.stored, started.Transaction); err != nil {
			t.Fatal(err)
		}
		if _, code := s.poll(ctx, proof); code != test.want || strings.Contains(code, "canary") {
			t.Fatalf("%s/%s returned %s", test.status, test.stored, code)
		}
	}
}

func TestNativeExpiryCapacityAndPolicyEpoch(t *testing.T) {
	s, request, verifier := nativeFixture(t)
	ctx := context.Background()
	started, code := s.start(ctx, request)
	if code != "" {
		t.Fatal(code)
	}
	now := s.now()
	s.now = func() time.Time { return now.Add(EnrollmentTTL) }
	if _, code := s.poll(ctx, NativeProof{started.Transaction, verifier}); code != EnrollmentExpired {
		t.Fatal("expiry boundary", code)
	}
	s.now = func() time.Time { return now }
	started, code = s.start(ctx, request)
	if code != "" {
		t.Fatal(code)
	}
	if err := s.store.transaction(ctx, func(tx *writeTx) error {
		_, err := tx.ExecContext(ctx, "UPDATE organization_policy SET epoch=epoch+1 WHERE singleton=1")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if _, code := s.poll(ctx, NativeProof{started.Transaction, verifier}); code != EnrollmentExpired {
		t.Fatal("old epoch accepted", code)
	}
	err := s.store.transaction(ctx, func(tx *writeTx) error {
		_, err := tx.ExecContext(ctx, `WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?) INSERT INTO enrollment_transactions(id,epoch,device_id,kind,label,csr,csr_digest,spki_digest,challenge,short_code,expires_at,status) SELECT printf('capacity-%d',x),2,'device','linux_cli','label','','','','','short',?,'pending' FROM n`, MaxPendingEnrollments, now.Add(EnrollmentTTL).Unix())
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, code := s.start(ctx, request); code != CapacityReached {
		t.Fatal("capacity bound absent", code)
	}
}

func TestNativeHTTPStrictFieldsAndBodyLimit(t *testing.T) {
	s, request, _ := nativeFixture(t)
	server, err := NewHTTPServer(DefaultListen, s.Handler(), nil)
	if err != nil {
		t.Fatal(err)
	}
	good, _ := json.Marshal(request)
	for _, test := range []struct {
		body   string
		status int
	}{
		{string(good), 201},
		{`{"api_key":"business-secret-canary"}`, 400},
		{`{"private_key":"private-secret-canary"}`, 400},
		{`{"organization":"attacker"}`, 400},
		{`{"csr_pem":"` + strings.Repeat("x", MaxBodyBytes) + `"}`, 400},
	} {
		w := httptest.NewRecorder()
		r := httptest.NewRequest("POST", "/enroll/native/start", strings.NewReader(test.body))
		r.Header.Set("Content-Type", "application/json")
		server.Handler.ServeHTTP(w, r)
		if w.Code != test.status {
			t.Fatalf("HTTP %d, want %d", w.Code, test.status)
		}
		if strings.Contains(w.Body.String(), "secret-canary") {
			t.Fatal("request secret reflected")
		}
	}
}
