package apikeyusage

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
	// RequestTimeout is the independent budget for one /v1/usage aggregate fetch.
	RequestTimeout = 25 * time.Second
	maxBodyBytes   = 1 << 20
)

// Request contains the transient inputs for one per-key usage fetch. URL must
// identify the exact /v1/usage endpoint and contain no query or fragment.
type Request struct {
	URL    string
	Period Period
	APIKey string
}

// Client performs usage requests. A caller may inject a transport that is
// already bound to a validated connection.
type Client struct {
	httpClient *http.Client
	timeout    time.Duration
}

// New returns a client that never follows redirects. A nil transport uses a
// direct transport that never consults environment proxy settings.
func New(transport http.RoundTripper) *Client {
	if transport == nil {
		transport = &http.Transport{
			Proxy:       nil,
			DialContext: (&net.Dialer{}).DialContext,
		}
	}
	return &Client{httpClient: &http.Client{
		Transport: transport,
		Timeout:   RequestTimeout,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}, timeout: RequestTimeout}
}

// Fetch requests and returns one fail-closed per-key usage snapshot.
func (c *Client) Fetch(ctx context.Context, request Request) (Snapshot, error) {
	period, err := NormalizePeriod(string(request.Period))
	if err != nil {
		return Snapshot{}, err
	}
	endpoint, err := url.Parse(request.URL)
	if err != nil || endpoint.Scheme == "" || endpoint.Host == "" || endpoint.User != nil ||
		endpoint.Path != "/v1/usage" || endpoint.RawPath != "" || endpoint.RawQuery != "" ||
		endpoint.ForceQuery || endpoint.Fragment != "" {
		return Snapshot{}, requestFailed()
	}
	query := endpoint.Query()
	query.Set("period", string(period))
	endpoint.RawQuery = query.Encode()

	requestCtx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(requestCtx, http.MethodGet, endpoint.String(), nil)
	if err != nil {
		return Snapshot{}, requestFailed()
	}
	req.Header.Set("Authorization", "Bearer "+request.APIKey)

	response, err := c.httpClient.Do(req)
	if err != nil {
		return Snapshot{}, requestFailed()
	}
	defer response.Body.Close()

	switch response.StatusCode {
	case http.StatusUnauthorized, http.StatusForbidden:
		return Snapshot{}, authFailed()
	case http.StatusNotFound, http.StatusMethodNotAllowed, http.StatusNotImplemented:
		return Snapshot{}, unavailable()
	case http.StatusOK:
	default:
		return Snapshot{}, requestFailed()
	}

	body, err := io.ReadAll(io.LimitReader(response.Body, maxBodyBytes+1))
	if err != nil {
		return Snapshot{}, requestFailed()
	}
	if len(body) > maxBodyBytes {
		return Snapshot{}, responseInvalid()
	}
	return Parse(body, period)
}

// Error is safe to expose through the management protocol. It deliberately
// does not retain an underlying error, URL, status text, headers, or body.
type Error struct {
	Code protocol.ErrorCode
	msg  string
}

func (e *Error) Error() string { return e.msg }

// CodeOf returns the stable usage error code, or an empty code for an error
// not created by this package.
func CodeOf(err error) protocol.ErrorCode {
	var usageErr *Error
	if errors.As(err, &usageErr) {
		return usageErr.Code
	}
	return ""
}

func authFailed() error {
	return &Error{Code: protocol.CodeUsageAuthFailed, msg: "usage authentication failed"}
}

func unavailable() error {
	return &Error{Code: protocol.CodeUsageUnavailable, msg: "usage is unavailable"}
}

func requestFailed() error {
	return &Error{Code: protocol.CodeUsageRequestFailed, msg: "usage request failed"}
}

func responseInvalid() error {
	return &Error{Code: protocol.CodeUsageResponseInvalid, msg: "usage response is invalid"}
}
