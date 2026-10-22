//go:build unix

package routerenroll

import (
	"context"
	"crypto/rand"
	"crypto/x509"
	"math/big"
	"testing"
	"time"
)

func fixtureRootCRL(t *testing.T, root *fixtureCA, number int64, now time.Time, revoked ...*x509.Certificate) []byte {
	t.Helper()
	var entries []x509.RevocationListEntry
	for _, v := range revoked {
		entries = append(entries, x509.RevocationListEntry{SerialNumber: v.SerialNumber, RevocationTime: now})
	}
	der, err := x509.CreateRevocationList(rand.Reader, &x509.RevocationList{Number: big.NewInt(number), ThisUpdate: now, NextUpdate: now.Add(30 * 24 * time.Hour), RevokedCertificateEntries: entries}, root.cert, root.key)
	if err != nil {
		t.Fatal(err)
	}
	return der
}

func TestCRLMonotonicPublicationAndRefresh(t *testing.T) {
	store, _, initial, now := prepareLedger(t)
	issuer, _, root := fixtureIssuer(t, now)
	ctx := context.Background()
	issued, code := store.issueAuthorized(ctx, issuer, initial, now)
	if code != "" {
		t.Fatal(code)
	}
	rootDER := fixtureRootCRL(t, root, 1, now)
	first, err := store.BuildCRL(ctx, issuer, rootDER, now)
	if err != nil || first.Number != 1 {
		t.Fatal(err)
	}
	same, err := store.BuildCRL(ctx, issuer, rootDER, now.Add(time.Minute))
	if err != nil || same.Digest != first.Digest {
		t.Fatal("unnecessary republish", err)
	}
	if _, err := revoke(ctx, store, RevocationTarget{"serial", issued.Serial}, now.Add(time.Minute)); err != nil {
		t.Fatal(err)
	}
	second, err := store.BuildCRL(ctx, issuer, rootDER, now.Add(time.Minute))
	if err != nil || second.Number != 2 {
		t.Fatal("revocation did not immediately advance CRL", err)
	}
	parsed, err := ValidateCRL(second.DER, issuer.cert, first.Number, first.Digest, now.Add(time.Minute))
	if err != nil || len(parsed.RevokedCertificateEntries) != 1 || parsed.RevokedCertificateEntries[0].SerialNumber.Text(16) != issued.Serial || parsed.NextUpdate.Sub(parsed.ThisUpdate) > CRLLifetime {
		t.Fatal("invalid generated CRL", err)
	}
	third, err := store.BuildCRL(ctx, issuer, rootDER, now.Add(time.Hour+time.Minute))
	if err != nil || third.Number != 3 {
		t.Fatal("hourly refresh failed", err)
	}
	if _, err := ValidateCRL(first.DER, issuer.cert, second.Number, second.Digest, now); err == nil {
		t.Fatal("rollback accepted")
	}
	if _, err := ValidateCRL(second.DER, issuer.cert, second.Number, "different-digest", now.Add(time.Minute)); err == nil {
		t.Fatal("same number with different body accepted")
	}
	if _, err := ValidateCRL(second.DER, issuer.cert, 0, "", now.Add(CRLLifetime+time.Hour)); err == nil {
		t.Fatal("expired CRL accepted")
	}
	corrupt := append([]byte(nil), second.DER...)
	corrupt[len(corrupt)-1] ^= 1
	if _, err := ValidateCRL(corrupt, issuer.cert, 0, "", now); err == nil {
		t.Fatal("wrong signature accepted")
	}
}

func TestRootCRLRevocationAndRollbackFailClosed(t *testing.T) {
	store, _, _, now := prepareLedger(t)
	issuer, ca, root := fixtureIssuer(t, now)
	ctx := context.Background()
	if _, err := store.BuildCRL(ctx, issuer, fixtureRootCRL(t, root, 3, now), now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.BuildCRL(ctx, issuer, fixtureRootCRL(t, root, 2, now), now); err == nil {
		t.Fatal("offline-root CRL rolled back")
	}
	if _, err := store.BuildCRL(ctx, issuer, fixtureRootCRL(t, root, 4, now, ca.cert), now); err == nil {
		t.Fatal("revoked issuing CA published")
	}
	wrong := newFixtureCA(t, nil, 99, now)
	if _, err := store.BuildCRL(ctx, issuer, fixtureRootCRL(t, wrong, 5, now), now); err == nil {
		t.Fatal("untrusted root CRL accepted")
	}
}
