//go:build unix

package routerenroll

import (
	"context"
	"errors"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"
)

func approveNative(t *testing.T, f *browserFlow) {
	t.Helper()
	f.authorize(t)
	page := f.callback()
	if page.Code != 200 {
		t.Fatal(page.Code)
	}
	w := f.post("/enroll/native/approve", url.Values{"transaction": {f.started.Transaction}, "csrf": {extractCSRF(t, page.Body.String())}, "decision": {"approve"}}, f.cookie, EnrollOrigin)
	if w.Code != 200 {
		t.Fatal("approval failed", w.Code)
	}
}
func TestNativeIssuanceResponseRecoveryAndSignedCancel(t *testing.T) {
	f := newBrowserFlow(t)
	approveNative(t, f)
	ctx := context.Background()
	result, code := f.s.pollResult(ctx, f.proof)
	if code != "" || result.Status != "issued" || result.Certificate == nil || result.Certificate.SPKIDigest != f.started.SPKIDigest {
		t.Fatal("bound issuance failed", code)
	}
	first := *result.Certificate
	if strings.Contains(first.CertificatePEM+first.ChainPEM, "PRIVATE KEY") {
		t.Fatal("private key in native response")
	}
	now := f.s.now()
	f.s.now = func() time.Time { return now.Add(PollInterval) }
	f.s.provider.client.Transport = providerTransport(func(*http.Request) (*http.Response, error) { return nil, errors.New("provider-offline-canary") })
	result, code = f.s.pollResult(ctx, f.proof)
	if code != "" || result.Certificate == nil || result.Certificate.Serial != first.Serial || result.Certificate.CertificatePEM != first.CertificatePEM {
		t.Fatal("recovery requires provider or re-signed", code)
	}
	state, code := f.s.cancelResult(ctx, f.proof)
	if code != "" || state.Status != "revocation_pending" {
		t.Fatal("committed issuance falsely cancelled as unissued", code)
	}
	var revoked int
	if f.s.store.db.QueryRow("SELECT count(*) FROM revocations WHERE serial=?", first.Serial).Scan(&revoked) != nil || revoked != 1 {
		t.Fatal("signed cancellation did not persist revocation")
	}
	f.s.now = func() time.Time { return now.Add(2 * PollInterval) }
	if _, code := f.s.pollResult(ctx, f.proof); code != ClientCertRejected {
		t.Fatal("late signed result revived cancelled identity", code)
	}
}

func TestConcurrentNativeClaimProducesOneCertificate(t *testing.T) {
	f := newBrowserFlow(t)
	approveNative(t, f)
	var group sync.WaitGroup
	serials := make(chan string, 8)
	for range 8 {
		group.Go(func() {
			result, code := f.s.pollResult(context.Background(), f.proof)
			if code != "" && code != SlowDown {
				t.Error(code)
			}
			if result.Certificate != nil {
				serials <- result.Certificate.Serial
			}
		})
	}
	group.Wait()
	close(serials)
	serial := ""
	for value := range serials {
		if serial == "" {
			serial = value
		}
		if value != serial {
			t.Fatal("multiple issuance results")
		}
	}
	var count int
	if serial == "" || f.s.store.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 1 {
		t.Fatal("concurrent claim did not commit exactly one result")
	}
}

func TestCancellationDuringFreshMemberCheckWinsBeforeSigning(t *testing.T) {
	f := newBrowserFlow(t)
	approveNative(t, f)
	base := f.s.provider.client.Transport
	f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		if strings.HasPrefix(r.URL.Path, "/api/v5/user/memberships/") {
			if code := f.s.cancel(context.Background(), f.proof); code != "" {
				t.Error(code)
			}
		}
		return base.RoundTrip(r)
	})
	if _, code := f.s.pollResult(context.Background(), f.proof); code != "CERT_OPERATION_CONFLICT" {
		t.Fatal("late membership response issued after cancellation", code)
	}
	var count int
	if f.s.store.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 0 {
		t.Fatal("cancel-before-commit issued certificate")
	}
}

func TestNativeResultRecoveryExpiresWithoutReissuing(t *testing.T) {
	f := newBrowserFlow(t)
	approveNative(t, f)
	if _, code := f.s.pollResult(context.Background(), f.proof); code != "" {
		t.Fatal(code)
	}
	now := f.s.now()
	f.s.now = func() time.Time { return now.Add(RecoveryWindow + time.Second) }
	if _, code := f.s.pollResult(context.Background(), f.proof); code != ReauthRequired {
		t.Fatal("result recovery not bounded", code)
	}
	var count int
	if f.s.store.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 1 {
		t.Fatal("expired recovery issued again")
	}
}
