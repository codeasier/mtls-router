package routerenroll

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/asn1"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"reflect"
	"strings"
	"testing"
	"time"
)

func fixtureIssuer(t *testing.T, now time.Time) (*Issuer, *fixtureCA, *fixtureCA) {
	t.Helper()
	root := newFixtureCA(t, nil, 1, now)
	ca := newFixtureCA(t, root, 2, now)
	issuer, err := NewIssuer(certificatePEM(ca.cert), privateKeyPEM(t, ca.key), certificatePEM(root.cert), now)
	if err != nil {
		t.Fatal(err)
	}
	return issuer, ca, root
}
func certificatePEM(cert *x509.Certificate) []byte {
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: cert.Raw})
}
func privateKeyPEM(t *testing.T, key *ecdsa.PrivateKey) []byte {
	t.Helper()
	der, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der})
}
func fixtureIdentity() Identity {
	return Identity{ProtocolVersion, "1783195", 3, "650d67fbae6d795139b49b41", "b5c2fb58-d5e8-4dc2-9b04-38209df797d0"}
}

func TestIssuerConstructsRestrictedIdentityAndUniqueSerials(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Second)
	issuer, ca, root := fixtureIssuer(t, now)
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	caPermission, _ := asn1.Marshal(struct {
		CA      bool
		PathLen int
	}{true, 9})
	serverPermission, _ := asn1.Marshal([]asn1.ObjectIdentifier{{1, 3, 6, 1, 5, 5, 7, 3, 1}})
	der, err := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{
		Subject: pkix.Name{CommonName: "administrator", Organization: []string{"attacker"}}, DNSNames: []string{"attacker.invalid"},
		ExtraExtensions: []pkix.Extension{{Id: asn1.ObjectIdentifier{2, 5, 29, 19}, Critical: true, Value: caPermission}, {Id: asn1.ObjectIdentifier{2, 5, 29, 37}, Value: serverPermission}},
	}, key)
	if err != nil {
		t.Fatal(err)
	}
	csr := string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: der}))
	serials := map[string]bool{}
	for range 8 {
		issued, err := issuer.Issue(csr, fixtureIdentity(), now)
		if err != nil {
			t.Fatal(err)
		}
		if serials[issued.Serial] || issued.IssuerID != issuer.id {
			t.Fatal("serial collision or issuer mismatch")
		}
		serials[issued.Serial] = true
		cert, err := parseSingleCertificate([]byte(issued.CertificatePEM))
		if err != nil {
			t.Fatal(err)
		}
		if cert.IsCA || cert.KeyUsage != x509.KeyUsageDigitalSignature || !reflect.DeepEqual(cert.ExtKeyUsage, []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}) || len(cert.DNSNames) != 0 || len(cert.IPAddresses) != 0 || len(cert.EmailAddresses) != 0 || cert.Subject.CommonName != fixtureIdentity().DeviceID || len(cert.Subject.Organization) != 0 {
			t.Fatal("CSR privileges/subject copied")
		}
		if !cert.PublicKey.(*ecdsa.PublicKey).Equal(&key.PublicKey) || issued.SPKIDigest != Digest(cert.RawSubjectPublicKeyInfo) || cert.NotAfter.After(now.Add(LeafLifetime)) || cert.NotBefore.Before(now.Add(-ClockSkew)) {
			t.Fatal("key or lifetime mismatch")
		}
		if len(cert.URIs) != 1 || !strings.HasPrefix(cert.URIs[0].String(), identityURIPrefix) {
			t.Fatal("identity URI missing")
		}
		claims, err := base64.RawURLEncoding.Strict().DecodeString(strings.TrimPrefix(cert.URIs[0].String(), identityURIPrefix))
		var identity Identity
		if err != nil || json.Unmarshal(claims, &identity) != nil || identity != fixtureIdentity() {
			t.Fatal("untrusted identity")
		}
		roots, intermediates := x509.NewCertPool(), x509.NewCertPool()
		roots.AddCert(root.cert)
		intermediates.AddCert(ca.cert)
		if _, err := cert.Verify(x509.VerifyOptions{Roots: roots, Intermediates: intermediates, CurrentTime: now, KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}}); err != nil {
			t.Fatal(err)
		}
		if _, err := cert.Verify(x509.VerifyOptions{Roots: roots, Intermediates: intermediates, CurrentTime: now, KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}); err == nil {
			t.Fatal("client certificate accepted as server")
		}
	}
}

func TestIssuerRejectsUnrestrictedOrMismatchedAuthority(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Second)
	_, ca, root := fixtureIssuer(t, now)
	for _, change := range []struct {
		name  string
		apply func(*x509.Certificate)
	}{
		{"path-len", func(c *x509.Certificate) { c.MaxPathLen = 1; c.MaxPathLenZero = false }},
		{"server-authority", func(c *x509.Certificate) { c.ExtKeyUsage = append(c.ExtKeyUsage, x509.ExtKeyUsageServerAuth) }},
		{"unconstrained-purpose", func(c *x509.Certificate) { c.ExtKeyUsage = nil }},
		{"expired", func(c *x509.Certificate) { c.NotBefore = now.Add(-48 * time.Hour); c.NotAfter = now.Add(-time.Hour) }},
		{"wrong-usage", func(c *x509.Certificate) { c.KeyUsage |= x509.KeyUsageDigitalSignature }},
	} {
		t.Run(change.name, func(t *testing.T) {
			template := *ca.cert
			change.apply(&template)
			der, err := x509.CreateCertificate(rand.Reader, &template, root.cert, &ca.key.PublicKey, root.key)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := NewIssuer(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), privateKeyPEM(t, ca.key), certificatePEM(root.cert), now); err == nil {
				t.Fatal("unrestricted authority accepted")
			}
		})
	}
	wrong, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := NewIssuer(certificatePEM(ca.cert), privateKeyPEM(t, wrong), certificatePEM(root.cert), now); err == nil {
		t.Fatal("mismatched key accepted")
	}
	issuer, _, _ := fixtureIssuer(t, now)
	if _, err := issuer.Issue(testCSR(t, elliptic.P384()), fixtureIdentity(), now); err == nil {
		t.Fatal("unsupported CSR accepted")
	}
	csr := testCSR(t, elliptic.P256())
	block, _ := pem.Decode([]byte(csr))
	block.Bytes[len(block.Bytes)-1] ^= 1
	if _, err := issuer.Issue(string(pem.EncodeToMemory(block)), fixtureIdentity(), now); err == nil {
		t.Fatal("invalid CSR signature accepted")
	}
}

func TestLeafCannotOutliveIssuerOrRoot(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Second)
	_, ca, root := fixtureIssuer(t, now)
	for _, duration := range []time.Duration{48 * time.Hour, LeafLifetime / 2} {
		template := *ca.cert
		template.NotAfter = now.Add(duration)
		der, err := x509.CreateCertificate(rand.Reader, &template, root.cert, &ca.key.PublicKey, root.key)
		if err != nil {
			t.Fatal(err)
		}
		issuer, err := NewIssuer(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), privateKeyPEM(t, ca.key), certificatePEM(root.cert), now)
		if err != nil {
			t.Fatal(err)
		}
		issued, err := issuer.Issue(testCSR(t, elliptic.P256()), fixtureIdentity(), now)
		if err != nil || issued.NotAfter != now.Add(duration).Unix() {
			t.Fatal("leaf outlived issuer", err)
		}
	}
}

// This PKI boundary is relevant to task 4.1: a public certificate result may be
// recoverable for 24 hours, but an mTLS-only recovery path cannot be reached with
// a predecessor that expires sooner. No renewal implementation is claimed here.
func TestMTLSRecoveryBoundaryForNearExpiryPredecessor(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Second)
	issuedAt := now.Add(-LeafLifetime + time.Hour)
	issuer, ca, root := fixtureIssuer(t, issuedAt)
	old, err := issuer.Issue(testCSR(t, elliptic.P256()), fixtureIdentity(), issuedAt)
	if err != nil {
		t.Fatal(err)
	}
	newResult, err := issuer.Issue(testCSR(t, elliptic.P256()), fixtureIdentity(), now)
	if err != nil {
		t.Fatal(err)
	}
	oldCert, err := parseSingleCertificate([]byte(old.CertificatePEM))
	if err != nil {
		t.Fatal(err)
	}
	newCert, err := parseSingleCertificate([]byte(newResult.CertificatePEM))
	if err != nil {
		t.Fatal(err)
	}
	roots, intermediates := x509.NewCertPool(), x509.NewCertPool()
	roots.AddCert(root.cert)
	intermediates.AddCert(ca.cert)
	options := x509.VerifyOptions{Roots: roots, Intermediates: intermediates, CurrentTime: now.Add(2 * time.Hour), KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}}
	if _, err := oldCert.Verify(options); err == nil {
		t.Fatal("expired predecessor unexpectedly valid for mTLS")
	}
	if _, err := newCert.Verify(options); err != nil {
		t.Fatal("replacement is not valid during the recovery window", err)
	}
	t.Log("predecessor expires in 1h; at +2h replacement is valid, but predecessor cannot establish mTLS during the promised 24h result-recovery window")
}
