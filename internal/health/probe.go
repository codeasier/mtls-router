package health

import (
	"context"
	"crypto/tls"
	"fmt"
	"net/http"
	"net/url"
	"time"

	"github.com/codeasier/mtls-router/internal/certs"
	"github.com/codeasier/mtls-router/internal/tlspolicy"
)

type ProbeOptions struct {
	UpstreamURL string
	ClientCert  string
	ClientKey   string
	UpstreamCA  string
	TLSMin      string
	Timeout     time.Duration
}

// ProbeFunc is the signature used by /health. The production code uses Probe;
// tests pass a stub.
type ProbeFunc func(ProbeOptions) error

// Probe is the default ProbeFunc that does a real mTLS+TCP dial.
var Probe ProbeFunc = func(opts ProbeOptions) error {
	if _, err := url.ParseRequestURI(opts.UpstreamURL); err != nil {
		return fmt.Errorf("invalid probe URL: %w", err)
	}
	clientCert, rootCAs, err := certs.LoadFromStrings(opts.ClientCert, opts.ClientKey, opts.UpstreamCA)
	if err != nil {
		return fmt.Errorf("load probe mTLS config: %w", err)
	}
	tlsMin, err := tlspolicy.MinVersion(opts.TLSMin)
	if err != nil {
		return fmt.Errorf("configure probe TLS: %w", err)
	}
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = 5 * time.Second
	}
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, opts.UpstreamURL, nil)
	if err != nil {
		return fmt.Errorf("create probe request: %w", err)
	}
	client := &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{Certificates: []tls.Certificate{*clientCert}, RootCAs: rootCAs, MinVersion: tlsMin}}}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("probe upstream: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= http.StatusInternalServerError {
		return fmt.Errorf("probe upstream returned status %d", resp.StatusCode)
	}
	return nil
}
