// Package modelcatalog fetches and normalizes the bounded model catalog exposed
// by a trusted router.
package modelcatalog

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

const (
	requestTimeout = 15 * time.Second
	maxBodyBytes   = 1 << 20
)

// Request contains the transient inputs for one model-catalog request. URL must
// identify the exact /v1/models endpoint and contain no query or fragment.
type Request struct {
	URL    string
	APIKey string
}

// Client performs model-catalog requests. A caller may inject a transport that
// is already bound to a validated connection; redirects and the HTTP deadline
// remain enforced by Client.
type Client struct {
	httpClient *http.Client
	timeout    time.Duration
}

// New returns a client using transport. A nil transport uses a direct transport
// that never consults environment proxy settings.
func New(transport http.RoundTripper) *Client {
	if transport == nil {
		transport = &http.Transport{
			Proxy:       nil,
			DialContext: (&net.Dialer{}).DialContext,
		}
	}
	return &Client{httpClient: &http.Client{
		Transport: transport,
		Timeout:   requestTimeout,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}, timeout: requestTimeout}
}

// Fetch requests and normalizes the complete model catalog.
func (c *Client) Fetch(ctx context.Context, request Request) ([]string, error) {
	endpoint, err := url.Parse(request.URL)
	if err != nil || endpoint.Scheme == "" || endpoint.Host == "" || endpoint.User != nil ||
		endpoint.Path != "/v1/models" || endpoint.RawPath != "" || endpoint.RawQuery != "" ||
		endpoint.ForceQuery || endpoint.Fragment != "" {
		return nil, discoveryFailed()
	}

	requestCtx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(requestCtx, http.MethodGet, endpoint.String(), nil)
	if err != nil {
		return nil, discoveryFailed()
	}
	req.Header.Set("Authorization", "Bearer "+request.APIKey)

	response, err := c.httpClient.Do(req)
	if err != nil {
		return nil, discoveryFailed()
	}
	defer response.Body.Close()

	if response.StatusCode == http.StatusUnauthorized || response.StatusCode == http.StatusForbidden {
		return nil, authFailed()
	}
	if response.StatusCode != http.StatusOK {
		return nil, discoveryFailed()
	}

	body, err := io.ReadAll(io.LimitReader(response.Body, maxBodyBytes+1))
	if err != nil {
		return nil, discoveryFailed()
	}
	if len(body) > maxBodyBytes {
		return nil, responseInvalid()
	}
	models, err := Parse(body)
	if err != nil {
		return nil, err
	}
	return models, nil
}

// Error is safe to expose through the management protocol. It deliberately
// does not retain an underlying error, URL, status text, headers, or body.
type Error struct {
	Code protocol.ErrorCode
	msg  string
}

func (e *Error) Error() string { return e.msg }

// CodeOf returns the stable catalog error code, or an empty code for an error
// not created by this package.
func CodeOf(err error) protocol.ErrorCode {
	var catalogErr *Error
	if errors.As(err, &catalogErr) {
		return catalogErr.Code
	}
	return ""
}

func authFailed() error {
	return &Error{Code: protocol.CodeModelAuthFailed, msg: "model catalog authentication failed"}
}

func discoveryFailed() error {
	return &Error{Code: protocol.CodeModelDiscoveryFailed, msg: "model catalog request failed"}
}

func responseInvalid() error {
	return &Error{Code: protocol.CodeModelResponseInvalid, msg: "model catalog response is invalid"}
}

func catalogEmpty() error {
	return &Error{Code: protocol.CodeModelCatalogEmpty, msg: "model catalog is empty"}
}
