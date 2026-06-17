package proxy

import (
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestNewMTLSTransportMissingInputs(t *testing.T) {
	if _, err := NewMTLSTransport("", "key", "ca"); err == nil {
		t.Fatal("expected missing cert error")
	}
	if _, err := NewMTLSTransport("cert", "", "ca"); err == nil {
		t.Fatal("expected missing key error")
	}
	if _, err := NewMTLSTransport("cert", "key", ""); err == nil {
		t.Fatal("expected missing ca error")
	}
}

func TestNewMTLSTransportTLSMin(t *testing.T) {
	caPEM, _, ca, caKey := testCertificate(t, "ca", true, nil, nil)
	certPEM, keyPEM, _, _ := testCertificate(t, "client", false, ca, caKey)
	for version, want := range map[string]uint16{"tls1.2": tls.VersionTLS12, "tls1.3": tls.VersionTLS13} {
		tr, err := NewMTLSTransport(certPEM, keyPEM, caPEM, WithTLSMin(version))
		if err != nil {
			t.Fatal(err)
		}
		if tr.TLSClientConfig.MinVersion != want {
			t.Fatalf("MinVersion=%d want %d", tr.TLSClientConfig.MinVersion, want)
		}
	}
	if _, err := NewMTLSTransport(certPEM, keyPEM, caPEM, WithTLSMin("tls1.1")); err == nil || !strings.Contains(err.Error(), "invalid TLS") {
		t.Fatalf("expected invalid TLS error, got %v", err)
	}
}

func TestNewMTLSTransportCallsMTLSServer(t *testing.T) {
	caPEM, _, ca, caKey := testCertificate(t, "ca", true, nil, nil)
	clientCertPEM, clientKeyPEM, _, _ := testCertificate(t, "client", false, ca, caKey)
	serverCertPEM, serverKeyPEM, _, _ := testCertificate(t, "server", false, ca, caKey)
	serverCert, err := tls.X509KeyPair([]byte(serverCertPEM), []byte(serverKeyPEM))
	if err != nil {
		t.Fatal(err)
	}
	clientCAs := x509.NewCertPool()
	clientCAs.AppendCertsFromPEM([]byte(caPEM))
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNoContent) }))
	server.TLS = &tls.Config{Certificates: []tls.Certificate{serverCert}, ClientCAs: clientCAs, ClientAuth: tls.RequireAndVerifyClientCert}
	server.StartTLS()
	defer server.Close()
	tr, err := NewMTLSTransport(clientCertPEM, clientKeyPEM, caPEM)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := (&http.Client{Transport: tr}).Get(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusNoContent {
		t.Fatalf("status=%d", resp.StatusCode)
	}
}

func testCertificate(t *testing.T, commonName string, isCA bool, parent *x509.Certificate, parentKey *rsa.PrivateKey) (string, string, *x509.Certificate, *rsa.PrivateKey) {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	serial, err := rand.Int(rand.Reader, big.NewInt(1<<62))
	if err != nil {
		t.Fatal(err)
	}
	tmpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: commonName},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature,
	}
	if isCA {
		tmpl.IsCA = true
		tmpl.KeyUsage |= x509.KeyUsageCertSign
		tmpl.BasicConstraintsValid = true
	} else {
		tmpl.ExtKeyUsage = []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth, x509.ExtKeyUsageClientAuth}
		tmpl.DNSNames = []string{"example.com"}
		if commonName == "server" {
			tmpl.IPAddresses = []net.IP{net.ParseIP("127.0.0.1")}
		}
	}
	if parent == nil {
		parent = tmpl
		parentKey = key
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, parent, &key.PublicKey, parentKey)
	if err != nil {
		t.Fatal(err)
	}
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)})
	return string(certPEM), string(keyPEM), tmpl, key
}
