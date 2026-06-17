package certs

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
)

func LoadFromStrings(certPEM, keyPEM, caPEM string) (*tls.Certificate, *x509.CertPool, error) {
	if certPEM == "" {
		return nil, nil, fmt.Errorf("client cert is required")
	}
	if keyPEM == "" {
		return nil, nil, fmt.Errorf("client key is required")
	}
	if caPEM == "" {
		return nil, nil, fmt.Errorf("upstream CA is required")
	}
	cert, err := tls.X509KeyPair([]byte(certPEM), []byte(keyPEM))
	if err != nil {
		return nil, nil, fmt.Errorf("load client cert/key: %w", err)
	}
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM([]byte(caPEM)) {
		return nil, nil, fmt.Errorf("load upstream CA")
	}
	return &cert, pool, nil
}
