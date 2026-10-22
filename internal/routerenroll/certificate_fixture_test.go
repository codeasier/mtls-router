package routerenroll

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"testing"
	"time"
)

type certificateFixture struct {
	LeafDays  int `json:"leaf_days"`
	Skew      int `json:"clock_skew_seconds"`
	RenewDays int `json:"renew_before_days"`
	Handover  int `json:"handover_seconds"`
	Cases     []struct {
		Name     string `json:"name"`
		Trust    string `json:"trust"`
		EKU      string `json:"eku"`
		Expired  bool   `json:"expired"`
		Revoked  bool   `json:"revoked"`
		Accepted bool   `json:"accepted"`
	} `json:"validation_cases"`
	Issuance []json.RawMessage `json:"issuance_cases"`
	Renewal  []json.RawMessage `json:"renewal_cases"`
}

// Test keys exist only in memory; the shared fixture specifies public profiles
// and expected outcomes rather than committing reusable private PEM material.
type fixtureCA struct {
	cert *x509.Certificate
	key  *ecdsa.PrivateKey
}

func newFixtureCA(t *testing.T, parent *fixtureCA, serial int64, now time.Time) *fixtureCA {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	template := &x509.Certificate{
		SerialNumber: big.NewInt(serial), Subject: pkix.Name{CommonName: fmt.Sprintf("synthetic-test-ca-%d", serial)},
		NotBefore: now.Add(-time.Hour), NotAfter: now.Add(60 * 24 * time.Hour),
		IsCA: true, BasicConstraintsValid: true, MaxPathLen: 0, MaxPathLenZero: true,
		KeyUsage: x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
	}
	if parent == nil {
		parent = &fixtureCA{template, key}
		template.MaxPathLen, template.MaxPathLenZero = 1, false
	} else {
		template.ExtKeyUsage = []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}
	}
	der, err := x509.CreateCertificate(rand.Reader, template, parent.cert, &key.PublicKey, parent.key)
	if err != nil {
		t.Fatal(err)
	}
	cert, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	return &fixtureCA{cert, key}
}

func TestPersonalCertificateFixtures(t *testing.T) {
	data, err := os.ReadFile("../../testdata/router-mtls-v1/certificates.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture certificateFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.LeafDays != 30 || fixture.Skew != 300 || fixture.RenewDays != 7 || fixture.Handover != 86400 || len(fixture.Issuance) != 5 || len(fixture.Renewal) != 5 {
		t.Fatal("certificate lifecycle fixture drift")
	}
	now := time.Date(2026, 10, 21, 12, 0, 0, 0, time.UTC)
	root := newFixtureCA(t, nil, 1, now)
	issuer := newFixtureCA(t, root, 2, now)
	roots, intermediates := x509.NewCertPool(), x509.NewCertPool()
	roots.AddCert(root.cert)
	intermediates.AddCert(issuer.cert)
	for i, test := range fixture.Cases {
		t.Run(test.Name, func(t *testing.T) {
			ca := issuer
			if test.Trust != "personal" {
				ca = newFixtureCA(t, nil, int64(100+i), now)
			}
			key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
			if err != nil {
				t.Fatal(err)
			}
			eku := x509.ExtKeyUsageClientAuth
			if test.EKU == "server_auth" {
				eku = x509.ExtKeyUsageServerAuth
			}
			leaf := &x509.Certificate{
				SerialNumber: big.NewInt(int64(10 + i)), NotBefore: now.Add(-5 * time.Minute), NotAfter: now.Add(time.Duration(fixture.LeafDays) * 24 * time.Hour),
				BasicConstraintsValid: true, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{eku},
			}
			if test.Expired {
				leaf.NotBefore, leaf.NotAfter = now.Add(-48*time.Hour), now.Add(-time.Hour)
			}
			der, err := x509.CreateCertificate(rand.Reader, leaf, ca.cert, &key.PublicKey, ca.key)
			if err != nil {
				t.Fatal(err)
			}
			cert, err := x509.ParseCertificate(der)
			if err != nil {
				t.Fatal(err)
			}
			_, verifyErr := cert.Verify(x509.VerifyOptions{Roots: roots, Intermediates: intermediates, CurrentTime: now, KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}})
			crlTemplate := &x509.RevocationList{Number: big.NewInt(1), ThisUpdate: now.Add(-time.Minute), NextUpdate: now.Add(24 * time.Hour)}
			if test.Revoked {
				crlTemplate.RevokedCertificateEntries = []x509.RevocationListEntry{{SerialNumber: cert.SerialNumber, RevocationTime: now.Add(-time.Second)}}
			}
			crlDER, err := x509.CreateRevocationList(rand.Reader, crlTemplate, ca.cert, ca.key)
			if err != nil {
				t.Fatal(err)
			}
			crl, err := x509.ParseRevocationList(crlDER)
			if err != nil || crl.CheckSignatureFrom(ca.cert) != nil {
				t.Fatal("invalid CRL fixture")
			}
			revoked := false
			for _, entry := range crl.RevokedCertificateEntries {
				revoked = revoked || entry.SerialNumber.Cmp(cert.SerialNumber) == 0
			}
			if accepted := verifyErr == nil && !revoked; accepted != test.Accepted {
				t.Fatalf("accepted=%v, want %v", accepted, test.Accepted)
			}
		})
	}
}
