package certs

import (
	"os"
	"strings"
	"testing"
)

func TestLoadFromStringsMissingInputs(t *testing.T) {
	cases := []struct{ cert, key, ca, want string }{{"", "k", "c", "client cert"}, {"c", "", "c", "client key"}, {"c", "k", "", "upstream CA"}}
	for _, tc := range cases {
		_, _, err := LoadFromStrings(tc.cert, tc.key, tc.ca)
		if err == nil || !strings.Contains(err.Error(), tc.want) {
			t.Fatalf("expected error naming %q, got %v", tc.want, err)
		}
	}
}

func TestLoadFromStringsMalformed(t *testing.T) {
	if _, _, err := LoadFromStrings("bad", "bad", "bad"); err == nil {
		t.Fatal("expected malformed cert/key error")
	}
}

func TestLoadFromStringsValid(t *testing.T) {
	certPEM, err := os.ReadFile("/tmp/mtls-router-test/cert.pem")
	if err != nil {
		t.Skip("test cert missing; run openssl setup")
	}
	keyPEM, err := os.ReadFile("/tmp/mtls-router-test/key.pem")
	if err != nil {
		t.Skip("test key missing; run openssl setup")
	}
	cert, pool, err := LoadFromStrings(string(certPEM), string(keyPEM), string(certPEM))
	if err != nil {
		t.Fatal(err)
	}
	if cert == nil || len(cert.Certificate) == 0 {
		t.Fatal("expected loaded certificate")
	}
	if pool == nil {
		t.Fatal("expected CA pool")
	}
}
