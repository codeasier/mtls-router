package health

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
	"testing"
	"time"
)

func TestProbeSucceedsAgainstMTLSServer(t *testing.T) {
	server, certs := newMTLSServer(t, http.StatusNoContent, 0, 0)
	defer server.Close()
	if err := Probe(ProbeOptions{UpstreamURL: server.URL, ClientCert: certs.clientCert, ClientKey: certs.clientKey, UpstreamCA: certs.ca, Timeout: time.Second}); err != nil {
		t.Fatal(err)
	}
}

func TestProbeTLSMinimumHandshake(t *testing.T) {
	tests := []struct {
		name          string
		serverVersion uint16
		tlsMin        string
		wantErr       bool
	}{
		{name: "TLS 1.2 server rejects TLS 1.3 minimum", serverVersion: tls.VersionTLS12, tlsMin: "tls1.3", wantErr: true},
		{name: "TLS 1.3 server accepts TLS 1.3 minimum", serverVersion: tls.VersionTLS13, tlsMin: "tls1.3"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			server, certs := newMTLSServer(t, http.StatusNoContent, 0, tt.serverVersion)
			defer server.Close()
			err := Probe(ProbeOptions{
				UpstreamURL: server.URL,
				ClientCert:  certs.clientCert,
				ClientKey:   certs.clientKey,
				UpstreamCA:  certs.ca,
				TLSMin:      tt.tlsMin,
				Timeout:     time.Second,
			})
			if (err != nil) != tt.wantErr {
				t.Fatalf("Probe() error = %v, wantErr %v", err, tt.wantErr)
			}
		})
	}
}

func TestProbeFails(t *testing.T) {
	if err := Probe(ProbeOptions{UpstreamURL: "://bad", Timeout: time.Second}); err == nil {
		t.Fatal("expected invalid URL error")
	}
	server, certs := newMTLSServer(t, http.StatusNoContent, 0, 0)
	defer server.Close()
	if err := Probe(ProbeOptions{UpstreamURL: server.URL, ClientCert: certs.clientCert, ClientKey: certs.clientKey, UpstreamCA: certs.clientCert, Timeout: time.Second}); err == nil {
		t.Fatal("expected handshake error")
	}
	server5xx, certs5xx := newMTLSServer(t, http.StatusServiceUnavailable, 0, 0)
	defer server5xx.Close()
	if err := Probe(ProbeOptions{UpstreamURL: server5xx.URL, ClientCert: certs5xx.clientCert, ClientKey: certs5xx.clientKey, UpstreamCA: certs5xx.ca, Timeout: time.Second}); err == nil {
		t.Fatal("expected 5xx error")
	}
	slow, slowCerts := newMTLSServer(t, http.StatusNoContent, 50*time.Millisecond, 0)
	defer slow.Close()
	if err := Probe(ProbeOptions{UpstreamURL: slow.URL, ClientCert: slowCerts.clientCert, ClientKey: slowCerts.clientKey, UpstreamCA: slowCerts.ca, Timeout: time.Nanosecond}); err == nil {
		t.Fatal("expected timeout error")
	}
}

type mtlsCerts struct {
	ca         string
	clientCert string
	clientKey  string
}

func newMTLSServer(t *testing.T, status int, delay time.Duration, tlsVersion uint16) (*httptest.Server, mtlsCerts) {
	t.Helper()
	caPEM, _, ca, caKey := testCertificate(t, "ca", true, nil, nil)
	clientCertPEM, clientKeyPEM, _, _ := testCertificate(t, "client", false, ca, caKey)
	serverCertPEM, serverKeyPEM, _, _ := testCertificate(t, "server", false, ca, caKey)
	serverCert, err := tls.X509KeyPair([]byte(serverCertPEM), []byte(serverKeyPEM))
	if err != nil {
		t.Fatal(err)
	}
	clientCAs := x509.NewCertPool()
	clientCAs.AppendCertsFromPEM([]byte(caPEM))
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { time.Sleep(delay); w.WriteHeader(status) }))
	server.TLS = &tls.Config{
		Certificates: []tls.Certificate{serverCert},
		ClientCAs:    clientCAs,
		ClientAuth:   tls.RequireAndVerifyClientCert,
		MinVersion:   tlsVersion,
		MaxVersion:   tlsVersion,
	}
	server.StartTLS()
	return server, mtlsCerts{ca: caPEM, clientCert: clientCertPEM, clientKey: clientKeyPEM}
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
