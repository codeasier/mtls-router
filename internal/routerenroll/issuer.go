package routerenroll

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"math/big"
	"net/url"
	"os"
	"sync"
	"time"
)

const (
	LeafLifetime        = 30 * 24 * time.Hour
	RenewBefore         = 7 * 24 * time.Hour
	ClockSkew           = 5 * time.Minute
	MaxCertificateBytes = 16 * 1024
	identityURIPrefix   = "urn:codeasier:router:identity:v1:"
)

var ErrIssuer = errors.New("enroll_issuer_invalid")

type Issuer struct {
	cert, root   *x509.Certificate
	key          *ecdsa.PrivateKey
	id, chain    string
	rootMu       sync.RWMutex
	rootCRL      *x509.RevocationList
	rootRequired bool
}
type IssuedCertificate struct {
	IssuerID       string   `json:"issuer_id"`
	Serial         string   `json:"serial"`
	CertificatePEM string   `json:"certificate_pem"`
	ChainPEM       string   `json:"chain_pem"`
	Identity       Identity `json:"identity"`
	NotBefore      int64    `json:"not_before"`
	NotAfter       int64    `json:"not_after"`
	SPKIDigest     string   `json:"spki_digest"`
}

func LoadIssuer(certFile, keyFile, rootFile string, now time.Time) (*Issuer, error) {
	key, err := readPrivateFile(keyFile, MaxCertificateBytes)
	if err != nil {
		return nil, ErrIssuer
	}
	defer clear(key)
	cert, err := readPublicPEM(certFile)
	if err != nil {
		return nil, ErrIssuer
	}
	root, err := readPublicPEM(rootFile)
	if err != nil {
		return nil, ErrIssuer
	}
	return NewIssuer(cert, key, root, now)
}
func readPublicPEM(path string) ([]byte, error) {
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Size() > MaxCertificateBytes {
		return nil, ErrIssuer
	}
	value, err := os.ReadFile(path)
	if err != nil || len(value) > MaxCertificateBytes {
		return nil, ErrIssuer
	}
	return value, nil
}
func parseSingleCertificate(value []byte) (*x509.Certificate, error) {
	block, rest := pem.Decode(value)
	if len(value) > MaxCertificateBytes || block == nil || block.Type != "CERTIFICATE" || len(block.Headers) != 0 || len(bytes.TrimSpace(rest)) != 0 {
		return nil, ErrIssuer
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return nil, ErrIssuer
	}
	return cert, nil
}

// NewIssuer accepts only a direct root -> dedicated client-auth, pathLen=0
// P-256 issuing CA and its matching PKCS#8 key. The offline root key is absent.
func NewIssuer(certPEM, keyPEM, rootPEM []byte, now time.Time) (*Issuer, error) {
	cert, err := parseSingleCertificate(certPEM)
	if err != nil {
		return nil, ErrIssuer
	}
	root, err := parseSingleCertificate(rootPEM)
	if err != nil {
		return nil, ErrIssuer
	}
	block, rest := pem.Decode(keyPEM)
	if len(keyPEM) > MaxCertificateBytes || block == nil || block.Type != "PRIVATE KEY" || len(block.Headers) != 0 || len(bytes.TrimSpace(rest)) != 0 {
		return nil, ErrIssuer
	}
	parsed, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, ErrIssuer
	}
	key, ok := parsed.(*ecdsa.PrivateKey)
	if !ok || key.Curve != elliptic.P256() {
		return nil, ErrIssuer
	}
	pub, ok := cert.PublicKey.(*ecdsa.PublicKey)
	if !ok || !pub.Equal(&key.PublicKey) || !cert.IsCA || !cert.BasicConstraintsValid || cert.MaxPathLen != 0 || !cert.MaxPathLenZero || cert.KeyUsage != x509.KeyUsageCertSign|x509.KeyUsageCRLSign || len(cert.ExtKeyUsage) != 1 || cert.ExtKeyUsage[0] != x509.ExtKeyUsageClientAuth || len(cert.UnknownExtKeyUsage) != 0 || bytes.Equal(cert.Raw, root.Raw) || cert.CheckSignatureFrom(root) != nil {
		return nil, ErrIssuer
	}
	i := &Issuer{cert: cert, root: root, key: key, id: Digest(cert.Raw), chain: string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: cert.Raw})) + string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: root.Raw}))}
	if !i.Ready(now) {
		return nil, ErrIssuer
	}
	return i, nil
}
func (i *Issuer) Ready(now time.Time) bool {
	if !i.validChain(now) {
		return false
	}
	i.rootMu.RLock()
	defer i.rootMu.RUnlock()
	return !i.rootRequired || (i.rootCRL != nil && !i.rootCRL.ThisUpdate.After(now) && i.rootCRL.NextUpdate.After(now))
}
func (i *Issuer) validChain(now time.Time) bool {
	roots := x509.NewCertPool()
	roots.AddCert(i.root)
	_, err := i.cert.Verify(x509.VerifyOptions{Roots: roots, CurrentTime: now, KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth}})
	return err == nil
}

func (i *Issuer) setRootCRL(der []byte, now time.Time) error {
	crl, err := i.validateRootCRL(der, 0, "", now)
	i.rootMu.Lock()
	defer i.rootMu.Unlock()
	i.rootRequired = true
	i.rootCRL = crl
	return err
}

func (i *Issuer) blockRootCRL() {
	i.rootMu.Lock()
	i.rootRequired = true
	i.rootCRL = nil
	i.rootMu.Unlock()
}
func (v Identity) valid() bool {
	return v.Version == ProtocolVersion && v.PolicyEpoch > 0 && stableIDPattern.MatchString(v.OrganizationID) && stableIDPattern.MatchString(v.UserID) && devicePattern.MatchString(v.DeviceID)
}

// Issue ignores all CSR subject, SAN, attributes and requested extensions. The
// caller must commit the public result and serial in its issuing transaction
// before returning it to a device; Issue itself is never a public HTTP handler.
func (i *Issuer) Issue(csrPEM string, identity Identity, now time.Time) (IssuedCertificate, error) {
	binding, code := ParseCSR(csrPEM)
	if code != "" || !identity.valid() || !i.Ready(now) {
		return IssuedCertificate{}, ErrIssuer
	}
	var serialBytes [20]byte
	if _, err := rand.Read(serialBytes[:]); err != nil {
		return IssuedCertificate{}, ErrIssuer
	}
	serialBytes[0] &= 0x7f
	serial := new(big.Int).SetBytes(serialBytes[:])
	if serial.Sign() == 0 {
		return IssuedCertificate{}, ErrIssuer
	}
	notBefore, notAfter := now.Add(-ClockSkew), now.Add(LeafLifetime)
	if notBefore.Before(i.cert.NotBefore) {
		notBefore = i.cert.NotBefore
	}
	if notAfter.After(i.cert.NotAfter) {
		notAfter = i.cert.NotAfter
	}
	if notAfter.After(i.root.NotAfter) {
		notAfter = i.root.NotAfter
	}
	if !notAfter.After(now) {
		return IssuedCertificate{}, ErrIssuer
	}
	claims, err := json.Marshal(identity)
	if err != nil {
		return IssuedCertificate{}, ErrIssuer
	}
	uri, err := url.Parse(identityURIPrefix + base64.RawURLEncoding.EncodeToString(claims))
	if err != nil {
		return IssuedCertificate{}, ErrIssuer
	}
	template := &x509.Certificate{
		SerialNumber: serial, Subject: pkix.Name{CommonName: identity.DeviceID}, NotBefore: notBefore, NotAfter: notAfter,
		BasicConstraintsValid: true, IsCA: false, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
		URIs: []*url.URL{uri}, SignatureAlgorithm: x509.ECDSAWithSHA256,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, i.cert, binding.Request.PublicKey, i.key)
	if err != nil || len(der) > MaxCertificateBytes {
		return IssuedCertificate{}, ErrIssuer
	}
	certPEM := string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}))
	return IssuedCertificate{i.id, serial.Text(16), certPEM, i.chain, identity, notBefore.Unix(), notAfter.Unix(), binding.SPKIDigest}, nil
}
