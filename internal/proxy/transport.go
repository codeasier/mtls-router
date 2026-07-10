package proxy

import (
	"crypto/tls"
	"net/http"

	"github.com/codeasier/mtls-router/internal/certs"
	"github.com/codeasier/mtls-router/internal/tlspolicy"
)

type TransportOption func(*transportOptions) error

type transportOptions struct {
	tlsMin uint16
}

func WithTLSMin(version string) TransportOption {
	return func(opts *transportOptions) error {
		minVersion, err := tlspolicy.MinVersion(version)
		if err != nil {
			return err
		}
		opts.tlsMin = minVersion
		return nil
	}
}

func NewMTLSTransport(certPEM, keyPEM, caPEM string, opts ...TransportOption) (*http.Transport, error) {
	clientCert, rootCAs, err := certs.LoadFromStrings(certPEM, keyPEM, caPEM)
	if err != nil {
		return nil, err
	}
	options := transportOptions{tlsMin: tls.VersionTLS12}
	for _, opt := range opts {
		if err := opt(&options); err != nil {
			return nil, err
		}
	}
	return &http.Transport{
		TLSClientConfig: &tls.Config{
			Certificates: []tls.Certificate{*clientCert},
			RootCAs:      rootCAs,
			MinVersion:   options.tlsMin,
		},
	}, nil
}
