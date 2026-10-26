//go:build unix

package routerenroll

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestMembershipRefreshEncryptionAndTransientFailure(t *testing.T) {
	f := newRenewalFixture(t)
	base := f.s.provider.client.Transport
	var refreshes atomic.Int32
	var offline atomic.Bool
	offline.Store(true)
	f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		if r.URL.Path == "/oauth/token" {
			refreshes.Add(1)
		}
		if strings.Contains(r.URL.Path, "memberships") && offline.Load() {
			return nil, errors.New("provider-secret-canary")
		}
		return base.RoundTrip(r)
	})
	ctx := context.Background()
	if _, code := f.s.verifyMember(ctx, f.old.Identity.PolicyEpoch, f.old.Identity.UserID); code != ServiceUnavailable {
		t.Fatal(code)
	}
	var verified, version int64
	var status string
	var ciphertext []byte
	if f.s.store.db.QueryRow(`SELECT u.verified_at,u.verification_status,t.version,t.ciphertext FROM users u JOIN gitcode_tokens t ON u.epoch=t.epoch AND u.user_id=t.user_id`).Scan(&verified, &status, &version, &ciphertext) != nil || verified != 0 || status != "unavailable" || version != 2 {
		t.Fatal("unverified response marked active or rotating token lost")
	}
	if strings.Contains(string(ciphertext), "canary") {
		t.Fatal("token persisted in plaintext")
	}
	var token GitCodeToken
	if f.s.store.decrypt(ciphertext, tokenAAD(f.old.Identity.PolicyEpoch, f.old.Identity.UserID, version), &token) != nil || token.RefreshToken != "rotated-refresh-canary" {
		t.Fatal("refresh rotation did not persist")
	}
	var revoked int
	if f.s.store.db.QueryRow("SELECT count(*) FROM revocations").Scan(&revoked) != nil || revoked != 0 {
		t.Fatal("outage misclassified as membership loss")
	}
	offline.Store(false)
	if _, code := f.s.verifyMember(ctx, f.old.Identity.PolicyEpoch, f.old.Identity.UserID); code != "" {
		t.Fatal(code)
	}
	if refreshes.Load() != 1 {
		t.Fatal("reused old rotating refresh token")
	}
}

func TestHiddenMembershipDoesNotRevokeExistingCertificates(t *testing.T) {
	f := newRenewalFixture(t)
	f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		status, body := 200, ""
		switch {
		case strings.Contains(r.URL.Path, "memberships"):
			status, body = 404, `{"login":"account-canary"}`
		case strings.HasSuffix(r.URL.Path, "/oauth/token"):
			body = `{"access_token":"refreshed-access-canary","refresh_token":"rotated-refresh-canary","expires_in":3600,"scope":"read_user read_groups"}`
		case strings.HasSuffix(r.URL.Path, "/user"):
			body = fmt.Sprintf(`{"id":%q,"login":"fixture"}`, f.old.Identity.UserID)
		default:
			t.Errorf("unexpected provider target %s", r.URL.Path)
		}
		return &http.Response{StatusCode: status, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(body)), Request: r}, nil
	})
	if _, code := f.s.verifyMember(context.Background(), f.old.Identity.PolicyEpoch, f.old.Identity.UserID); code != ServiceUnavailable {
		t.Fatal(code)
	}
	var status string
	var revoked int
	if f.s.store.db.QueryRow("SELECT verification_status FROM users").Scan(&status) != nil || status == "nonmember" || f.s.store.db.QueryRow("SELECT count(*) FROM revocations").Scan(&revoked) != nil || revoked != 0 {
		t.Fatal("404 revoked a member", status, revoked)
	}
}

func TestBackgroundMembershipRecheckAndConfirmedLoss(t *testing.T) {
	f := newRenewalFixture(t)
	ctx := context.Background()
	if err := f.s.RecheckMembers(ctx); err != nil {
		t.Fatal(err)
	}
	if f.calls.Load() != 1 {
		t.Fatal("active user not checked")
	}
	if err := f.s.RecheckMembers(ctx); err != nil || f.calls.Load() != 1 {
		t.Fatal("background check busy-looped")
	}
	f.now = f.now.Add(MembershipInterval)
	f.active.Store(false)
	if err := f.s.RecheckMembers(ctx); err != nil || f.calls.Load() != 2 {
		t.Fatal("300s target not applied")
	}
	var status string
	var disabled, revoked int
	if f.s.store.db.QueryRow("SELECT verification_status FROM users").Scan(&status) != nil || status != "nonmember" {
		t.Fatal("confirmed loss not recorded")
	}
	if f.s.store.db.QueryRow("SELECT disabled FROM devices").Scan(&disabled) != nil || disabled != 1 || f.s.store.db.QueryRow("SELECT count(*) FROM revocations").Scan(&revoked) != nil || revoked != 1 {
		t.Fatal("confirmed loss did not revoke device")
	}
}

func TestBrowserMembershipLossUsesAccountVersionAndRevokesExistingDevices(t *testing.T) {
	for _, newer := range []bool{false, true} {
		t.Run(map[bool]string{false: "current", true: "stale"}[newer], func(t *testing.T) {
			f := newRenewalFixture(t)
			ctx := context.Background()
			epoch, user := f.old.Identity.PolicyEpoch, f.old.Identity.UserID
			checked, err := f.s.memberVersion(ctx, epoch, user)
			if err != nil {
				t.Fatal(err)
			}
			if newer {
				if err := f.s.store.transaction(ctx, func(tx *writeTx) error {
					_, err := tx.ExecContext(ctx, "UPDATE users SET version=version+1,verification_status='active' WHERE epoch=? AND user_id=?", epoch, user)
					return err
				}); err != nil {
					t.Fatal(err)
				}
			}
			if err := f.s.recordMembershipLoss(ctx, epoch, user, checked); err != nil {
				t.Fatal(err)
			}
			var count int
			if f.s.store.db.QueryRow("SELECT count(*) FROM revocations").Scan(&count) != nil {
				t.Fatal("query")
			}
			if newer && count != 0 || !newer && count != 1 {
				t.Fatal("stale browser result overwrote account state", count)
			}
		})
	}
}

func TestMembershipSingleFlightAndRevocationCAS(t *testing.T) {
	f := newRenewalFixture(t)
	ctx := context.Background()
	entered := make(chan struct{})
	release := make(chan struct{})
	base := f.s.provider.client.Transport
	f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		if strings.Contains(r.URL.Path, "memberships") {
			close(entered)
			<-release
		}
		return base.RoundTrip(r)
	})
	done := make(chan string, 1)
	go func() {
		_, code := f.s.verifyMember(ctx, f.old.Identity.PolicyEpoch, f.old.Identity.UserID)
		done <- code
	}()
	select {
	case <-entered:
	case <-time.After(5 * time.Second):
		t.Fatal("provider not reached")
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if _, code := f.s.verifyMember(cancelled, f.old.Identity.PolicyEpoch, f.old.Identity.UserID); code != ServiceUnavailable {
		t.Fatal("waiter cancellation ignored", code)
	}
	if _, err := revoke(ctx, f.s.store, RevocationTarget{"user", f.old.Identity.UserID}, f.now); err != nil {
		t.Fatal(err)
	}
	close(release)
	select {
	case code := <-done:
		if code != OperationConflict {
			t.Fatal("late check overwrote revoke", code)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("check hung")
	}
	if f.calls.Load() != 1 {
		t.Fatal("overlapping check was not coalesced")
	}
	var status string
	if f.s.store.db.QueryRow("SELECT verification_status FROM users").Scan(&status) != nil || status != "revoked" {
		t.Fatal("revoked user reactivated")
	}
}
