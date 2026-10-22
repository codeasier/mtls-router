//go:build unix

package routerenroll

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func keyedCSR(t *testing.T) (*ecdsa.PrivateKey, string) {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{}, key)
	if err != nil {
		t.Fatal(err)
	}
	return key, string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: der}))
}

type renewalFixture struct {
	s       *Service
	key     *ecdsa.PrivateKey
	old     IssuedCertificate
	parsed  *x509.Certificate
	request RenewalRequest
	now     time.Time
	calls   atomic.Int32
	active  atomic.Bool
}

func newRenewalFixture(t *testing.T) *renewalFixture {
	t.Helper()
	store, issuer, initial, now := prepareLedger(t)
	key, csr := keyedCSR(t)
	initial.CSRPEM = csr
	old, code := store.issueAuthorized(context.Background(), issuer, initial, now)
	if code != "" {
		t.Fatal(code)
	}
	f := &renewalFixture{key: key, old: old, now: now.Add(LeafLifetime - time.Hour)}
	f.parsed, _ = parseSingleCertificate([]byte(old.CertificatePEM))
	f.s = NewService(store, nil, issuer)
	f.s.now = func() time.Time { return f.now }
	f.active.Store(true)
	f.s.provider = &GitCode{clientID: "fixture", secret: "secret-canary", now: f.s.now, client: &http.Client{Timeout: time.Second}}
	f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		body := ""
		switch r.URL.Scheme + "://" + r.URL.Host + r.URL.Path {
		case membershipURL + initial.Account.Organization.Path:
			f.calls.Add(1)
			body = fmt.Sprintf(`{"active":%t,"user":{"id":%q},"organization":{"id":%q,"login":%q}}`, f.active.Load(), initial.Account.ID, initial.Account.Organization.ID, initial.Account.Organization.Path)
		case tokenURL:
			body = `{"access_token":"refreshed-access-canary","refresh_token":"rotated-refresh-canary","expires_in":3600,"scope":"read_user read_groups"}`
		case userURL:
			body = fmt.Sprintf(`{"id":%q,"login":"fixture"}`, initial.Account.ID)
		default:
			t.Error("unexpected provider endpoint")
		}
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(body)), Request: r}, nil
	})
	_, nextCSR := keyedCSR(t)
	f.request = RenewalRequest{Version: ProtocolVersion, IssuerID: old.IssuerID, Serial: old.Serial, OperationID: "f6d738a6-e968-4f25-9da2-754bb0ea69e8", CSRPEM: nextCSR}
	return f
}
func signRenewal(t *testing.T, v RenewalRequest, key *ecdsa.PrivateKey, recoverOnly bool, now time.Time) RenewalRequest {
	t.Helper()
	v.Timestamp = now.Unix()
	data, code := RenewalSigningBytes(v, recoverOnly)
	if code != "" {
		t.Fatal(code)
	}
	digest := sha256.Sum256(data)
	signature, err := ecdsa.SignASN1(rand.Reader, key, digest[:])
	if err != nil {
		t.Fatal(err)
	}
	v.Signature = base64.RawURLEncoding.EncodeToString(signature)
	return v
}
func (f *renewalFixture) sign(t *testing.T, recoverOnly bool) RenewalRequest {
	return signRenewal(t, f.request, f.key, recoverOnly, f.now)
}
func (f *renewalFixture) issue(t *testing.T) IssuedCertificate {
	t.Helper()
	v, code := f.s.renew(context.Background(), f.sign(t, false), f.parsed)
	if code != "" {
		t.Fatal(code)
	}
	return v
}

func TestRenewalRecoveryAfterPredecessorExpiryIsReadOnly(t *testing.T) {
	f := newRenewalFixture(t)
	issued := f.issue(t)
	f.now = f.now.Add(2 * time.Hour)
	if !f.now.After(f.parsed.NotAfter) {
		t.Fatal("test did not cross predecessor expiry")
	}
	f.s.issuer = nil
	f.s.provider = nil
	checkpoint, err := readCheckpoint(context.Background(), f.s.store.db)
	if err != nil {
		t.Fatal(err)
	}
	for range 3 {
		v, code := f.s.recoverRenewal(context.Background(), f.sign(t, true))
		if code != "" || v.Serial != issued.Serial || v.CertificatePEM != issued.CertificatePEM {
			t.Fatal("expired predecessor result recovery failed", code)
		}
	}
	after, err := readCheckpoint(context.Background(), f.s.store.db)
	if err != nil || after != checkpoint {
		t.Fatal("recovery performed a durable write")
	}
	if _, code := f.s.renew(context.Background(), f.sign(t, false), f.parsed); code != ClientCertExpired {
		t.Fatal("expired certificate authorized new issuance", code)
	}
	f.now = f.now.Add(RecoveryWindow - 2*time.Hour)
	if _, code := f.s.recoverRenewal(context.Background(), f.sign(t, true)); code != ReauthRequired {
		t.Fatal("24h deadline was extended", code)
	}
	var count int
	if f.s.store.db.QueryRow("SELECT count(*) FROM certificates").Scan(&count) != nil || count != 2 {
		t.Fatal("recovery reissued")
	}
}

func renewalHTTP(t *testing.T, handler http.Handler, path string, request RenewalRequest, certificate string) *httptest.ResponseRecorder {
	t.Helper()
	data, _ := json.Marshal(request)
	r := httptest.NewRequest("POST", EnrollOrigin+path, bytes.NewReader(data))
	r.Header.Set("Content-Type", "application/json")
	if certificate != "" {
		r.Header.Set(GatewayVerifyHeader, "SUCCESS")
		r.Header.Set(GatewayCertificateHeader, url.PathEscape(certificate))
	}
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	return w
}

func TestRenewalOriginPurposeAndGatewayBoundary(t *testing.T) {
	f := newRenewalFixture(t)
	ctx := context.Background()
	if w := renewalHTTP(t, f.s.Handler(), RenewPath, f.sign(t, false), f.old.CertificatePEM); w.Code == 200 {
		t.Fatal("public header forgery signed")
	}
	if w := renewalHTTP(t, f.s.GatewayHandler(), RenewPath, f.sign(t, false), ""); w.Code != 403 {
		t.Fatal("missing gateway accepted", w.Code)
	}
	if w := renewalHTTP(t, f.s.GatewayHandler(), RenewPath, f.sign(t, true), f.old.CertificatePEM); w.Code != 400 {
		t.Fatal("recovery signature authorized issuance", w.Code)
	}
	if _, code := f.s.recoverRenewal(ctx, f.sign(t, true)); code != OperationConflict {
		t.Fatal("uncommitted operation recovered", code)
	}
	wrong, _ := keyedCSR(t)
	if _, code := f.s.renew(ctx, signRenewal(t, f.request, wrong, false, f.now), f.parsed); code != InvalidProof {
		t.Fatal("wrong private key accepted", code)
	}
	if _, code := f.s.renew(ctx, signRenewal(t, f.request, f.key, false, f.now.Add(-ClockSkew-time.Second)), f.parsed); code != InvalidProof {
		t.Fatal("stale proof accepted", code)
	}
	if w := renewalHTTP(t, f.s.GatewayHandler(), RenewPath, f.sign(t, false), f.old.CertificatePEM); w.Code != 200 {
		t.Fatal("verified gateway signing failed", w.Code, w.Body.String())
	}
	if w := renewalHTTP(t, f.s.Handler(), RenewRecoverPath, f.sign(t, true), ""); w.Code != 200 {
		t.Fatal("public recovery failed", w.Code)
	}
	changed := f.sign(t, true)
	_, changed.CSRPEM = keyedCSR(t)
	if _, code := f.s.recoverRenewal(ctx, changed); code != InvalidProof {
		t.Fatal("changed CSR accepted", code)
	}
}

func TestConcurrentRenewalHasOneSuccessorAndCannotChain(t *testing.T) {
	f := newRenewalFixture(t)
	v := f.sign(t, false)
	serials := make(chan string, 8)
	var group sync.WaitGroup
	for range 8 {
		group.Go(func() {
			result, code := f.s.renew(context.Background(), v, f.parsed)
			if code != "" && code != OperationConflict {
				t.Error(code)
			}
			if code == "" {
				serials <- result.Serial
			}
		})
	}
	group.Wait()
	close(serials)
	serial := ""
	for v := range serials {
		if serial == "" {
			serial = v
		}
		if v != serial {
			t.Fatal("multiple successors")
		}
	}
	if serial == "" {
		t.Fatal("no committed successor")
	}
	different := f.request
	different.OperationID = "67260b59-9e9e-4cca-8db5-67411c8f243f"
	if _, code := f.s.renew(context.Background(), signRenewal(t, different, f.key, false, f.now), f.parsed); code != OperationConflict {
		t.Fatal("second successor accepted", code)
	}
	var count int
	if f.s.store.db.QueryRow("SELECT count(*) FROM renewal_operations").Scan(&count) != nil || count != 1 {
		t.Fatal("operation not unique")
	}
	f.now = f.now.Add(RecoveryWindow)
	if err := f.s.retirePredecessors(context.Background()); err != nil {
		t.Fatal(err)
	}
	var reason string
	if f.s.store.db.QueryRow("SELECT reason FROM revocations WHERE serial=?", f.old.Serial).Scan(&reason) != nil || reason != "handover" {
		t.Fatal("predecessor not retired")
	}
}

func TestRevocationWinsRenewalAndRecovery(t *testing.T) {
	for _, kind := range []string{"before_commit", "predecessor", "successor", "device", "user", "policy"} {
		t.Run(kind, func(t *testing.T) {
			f := newRenewalFixture(t)
			ctx := context.Background()
			if kind == "before_commit" {
				base := f.s.provider.client.Transport
				f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
					if strings.Contains(r.URL.Path, "memberships") {
						_, err := revoke(ctx, f.s.store, RevocationTarget{"device", f.old.Identity.DeviceID}, f.now)
						if err != nil {
							t.Error(err)
						}
					}
					return base.RoundTrip(r)
				})
				if _, code := f.s.renew(ctx, f.sign(t, false), f.parsed); code != ClientCertRejected {
					t.Fatal("late check revived revoked device", code)
				}
				return
			}
			next := f.issue(t)
			target := RevocationTarget{"serial", f.old.Serial}
			switch kind {
			case "successor":
				target.ID = next.Serial
			case "device":
				target = RevocationTarget{kind, f.old.Identity.DeviceID}
			case "user":
				target = RevocationTarget{kind, f.old.Identity.UserID}
			case "policy":
				cfg := testConfig(t)
				cfg.OrgPath = "other-org"
				if _, err := f.s.store.ActivatePolicy(ctx, cfg); err != nil {
					t.Fatal(err)
				}
			}
			if kind != "policy" {
				if _, err := revoke(ctx, f.s.store, target, f.now); err != nil {
					t.Fatal(err)
				}
			}
			if _, code := f.s.recoverRenewal(ctx, f.sign(t, true)); code != ClientCertRejected {
				t.Fatal("revoked result restored", code)
			}
		})
	}
}
