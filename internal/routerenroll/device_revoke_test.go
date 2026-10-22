//go:build unix

package routerenroll

import (
	"context"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"testing"
)

func signedDeviceRevoke(t *testing.T, f *renewalFixture, key *ecdsa.PrivateKey) DeviceRevokeRequest {
	t.Helper()
	v := DeviceRevokeRequest{Version: ProtocolVersion, IssuerID: f.old.IssuerID, Serial: f.old.Serial, Timestamp: f.now.Unix()}
	data, code := DeviceRevokeSigningBytes(v)
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

func TestSignedDeviceLogoutIsScopedIdempotentAndCancelsLateRenewal(t *testing.T) {
	f := newRenewalFixture(t)
	ctx := context.Background()
	f.issue(t)
	other := authorizedIssue{OperationID: "67260b59-9e9e-4cca-8db5-67411c8f243d", Epoch: f.old.Identity.PolicyEpoch, Account: Account{ID: f.old.Identity.UserID, Organization: Organization{f.old.Identity.OrganizationID, "msinsight-org"}}, DeviceID: "b5c2fb58-d5e8-4dc2-9b04-38209df797d1", Kind: "linux_cli", Label: "other"}
	_, other.CSRPEM = keyedCSR(t)
	if _, code := f.s.store.issueAuthorized(ctx, f.s.issuer, other, f.now); code != "" {
		t.Fatal(code)
	}
	wrong, _ := keyedCSR(t)
	if _, code := f.s.revokeDevice(ctx, signedDeviceRevoke(t, f, wrong)); code != InvalidProof {
		t.Fatal("wrong key revoked", code)
	}
	for range 2 {
		result, code := f.s.revokeDevice(ctx, signedDeviceRevoke(t, f, f.key))
		if code != "" || result.Status != "revocation_pending" {
			t.Fatal("false remote revocation claim", code)
		}
	}
	if _, code := f.s.recoverRenewal(ctx, f.sign(t, true)); code != ClientCertRejected {
		t.Fatal("logout resurrected by late renewal", code)
	}
	var disabled, count int
	if f.s.store.db.QueryRow("SELECT disabled FROM devices WHERE id=?", other.DeviceID).Scan(&disabled) != nil || disabled != 0 {
		t.Fatal("other device disabled")
	}
	if f.s.store.db.QueryRow("SELECT count(*) FROM revocations").Scan(&count) != nil || count != 2 {
		t.Fatal("device certificate set not revoked")
	}
}
