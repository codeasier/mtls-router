package health

import (
	"context"
	"crypto/tls"
	"fmt"
	"io"
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

type Prober struct {
	url       string
	timeout   time.Duration
	client    *http.Client
	transport *http.Transport
}

func NewProber(opts ProbeOptions) (*Prober, error) {
	if _, err := url.ParseRequestURI(opts.UpstreamURL); err != nil {
		return nil, fmt.Errorf("invalid probe URL")
	}
	clientCert, rootCAs, err := certs.LoadFromStrings(opts.ClientCert, opts.ClientKey, opts.UpstreamCA)
	if err != nil {
		return nil, fmt.Errorf("load probe mTLS config")
	}
	tlsMin, err := tlspolicy.MinVersion(opts.TLSMin)
	if err != nil {
		return nil, fmt.Errorf("configure probe TLS")
	}
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = 5 * time.Second
	}
	transport := &http.Transport{TLSClientConfig: &tls.Config{
		Certificates: []tls.Certificate{*clientCert},
		RootCAs:      rootCAs,
		MinVersion:   tlsMin,
	}}
	return &Prober{
		url:       opts.UpstreamURL,
		timeout:   timeout,
		client:    &http.Client{Transport: transport},
		transport: transport,
	}, nil
}

func (p *Prober) Probe(_ ProbeOptions) error {
	ctx, cancel := context.WithTimeout(context.Background(), p.timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, p.url, nil)
	if err != nil {
		return fmt.Errorf("create probe request")
	}
	resp, err := p.client.Do(req)
	if err != nil {
		return fmt.Errorf("probe upstream failed")
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, resp.Body)
	if resp.StatusCode >= http.StatusInternalServerError {
		return fmt.Errorf("probe upstream returned status %d", resp.StatusCode)
	}
	return nil
}

func (p *Prober) Close() {
	p.transport.CloseIdleConnections()
}

// Probe is the default ProbeFunc that performs a one-shot mTLS probe.
var Probe ProbeFunc = func(opts ProbeOptions) error {
	prober, err := NewProber(opts)
	if err != nil {
		return err
	}
	defer prober.Close()
	return prober.Probe(opts)
}
