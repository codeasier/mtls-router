package proxy

import (
	"bytes"
	"context"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func TestNewErrorHandler(t *testing.T) {
	for _, tc := range []struct {
		err  error
		want int
	}{
		{context.DeadlineExceeded, http.StatusGatewayTimeout},
		{clientBodyReadError{err: io.ErrUnexpectedEOF}, http.StatusBadRequest},
		{errors.New("x509 private key secret upstream.internal"), http.StatusBadGateway},
	} {
		rr := httptest.NewRecorder()
		NewErrorHandler(nil)(rr, httptest.NewRequest(http.MethodGet, "/", nil), tc.err)
		if rr.Code != tc.want {
			t.Fatalf("status=%d want=%d", rr.Code, tc.want)
		}
		if ct := rr.Header().Get("Content-Type"); ct != "application/json" {
			t.Fatalf("content-type=%q", ct)
		}
		body := rr.Body.String()
		if !strings.Contains(body, "error") || strings.Contains(body, "private key") || strings.Contains(body, "upstream.internal") {
			t.Fatalf("unsafe JSON body: %q", body)
		}
	}
}

func TestProxyErrorLogSanitizesRequestAndUpstreamDetails(t *testing.T) {
	const (
		queryCanary    = "sk-query-canary-1a2b3c"
		authCanary     = "Bearer auth-canary-4d5e6f"
		bodyCanary     = "body-canary-7g8h9i"
		upstreamCanary = "upstream-error-canary-0j1k2l"
	)
	var buf bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&buf, nil))
	req := httptest.NewRequest(http.MethodPost, "/v1/chat?api_key="+queryCanary, strings.NewReader(bodyCanary))
	req.Header.Set("Authorization", authCanary)
	err := errors.New(`Get "https://upstream.example/private?token=` + upstreamCanary + `": private key ` + upstreamCanary)
	rec := httptest.NewRecorder()

	NewErrorHandler(logger)(rec, req, err)

	out := buf.String()
	for _, want := range []string{"msg=\"proxy request failed\"", "method=POST", "path=/v1/chat", "status=502", "reason=upstream_failure"} {
		if !strings.Contains(out, want) {
			t.Fatalf("proxy error log missing %q: %s", want, out)
		}
	}
	for _, canary := range []string{queryCanary, authCanary, bodyCanary, upstreamCanary, "api_key", "Authorization", "private key"} {
		if strings.Contains(out, canary) || strings.Contains(rec.Body.String(), canary) {
			t.Fatalf("proxy error output leaked %q: log=%q body=%q", canary, out, rec.Body.String())
		}
	}
}

func TestReverseProxySanitizesStandardLibraryErrorLog(t *testing.T) {
	const upstreamCanary = "sk-upstream-stream-error-canary"
	var buf bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&buf, nil))
	upstream, err := url.Parse("https://upstream.example")
	if err != nil {
		t.Fatal(err)
	}
	rp := New(Options{Upstream: upstream, ErrorLog: logger})

	rp.ErrorLog.Printf("httputil: ReverseProxy read error during body copy: %s", upstreamCanary)

	out := buf.String()
	if !strings.Contains(out, "msg=\"proxy stream failed\"") {
		t.Fatalf("proxy stream log missing sanitized message: %s", out)
	}
	if strings.Contains(out, upstreamCanary) {
		t.Fatalf("proxy stream log leaked upstream error: %s", out)
	}
}
