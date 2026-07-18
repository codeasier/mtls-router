package modelcatalog

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

func TestFetchSendsExactRequest(t *testing.T) {
	const key = "catalog-key-canary"
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			t.Errorf("method = %q", r.Method)
		}
		if r.URL.Path != "/v1/models" || r.URL.RawQuery != "" || r.RequestURI != "/v1/models" {
			t.Errorf("request URL = %q", r.RequestURI)
		}
		if r.Body == nil {
			t.Error("request body is nil; expected http.NoBody")
		} else if body, err := io.ReadAll(r.Body); err != nil || len(body) != 0 {
			t.Errorf("body = %q, err = %v", body, err)
		}
		values := r.Header.Values("Authorization")
		if len(values) != 1 || values[0] != "Bearer "+key {
			t.Errorf("Authorization values = %q", values)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"data":[{"id":"b"},{"id":"a"}]}`)
	}))
	defer server.Close()

	models, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: key})
	if err != nil {
		t.Fatalf("Fetch() error = %v", err)
	}
	if strings.Join(models, ",") != "a,b" {
		t.Fatalf("Fetch() = %q", models)
	}
}

func TestFetchStatusMappingAndSanitization(t *testing.T) {
	const key = "secret-key-canary"
	const responseCanary = "response-body-canary"
	const headerCanary = "response-header-canary"
	tests := []struct {
		status int
		code   protocol.ErrorCode
	}{
		{status: http.StatusUnauthorized, code: protocol.CodeModelAuthFailed},
		{status: http.StatusForbidden, code: protocol.CodeModelAuthFailed},
		{status: http.StatusBadRequest, code: protocol.CodeModelDiscoveryFailed},
		{status: http.StatusTooManyRequests, code: protocol.CodeModelDiscoveryFailed},
		{status: http.StatusInternalServerError, code: protocol.CodeModelDiscoveryFailed},
	}
	for _, test := range tests {
		t.Run(http.StatusText(test.status), func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("X-Canary", headerCanary)
				w.WriteHeader(test.status)
				_, _ = io.WriteString(w, responseCanary)
			}))
			defer server.Close()
			_, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: key})
			assertCode(t, err, test.code)
			assertSanitized(t, err, key, responseCanary, headerCanary, server.URL)
		})
	}
}

func TestFetchDoesNotFollowRedirect(t *testing.T) {
	const key = "redirect-key-canary"
	var followed atomic.Bool
	target := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		followed.Store(true)
	}))
	defer target.Close()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Redirect(w, &http.Request{}, target.URL+"/key-target", http.StatusFound)
	}))
	defer server.Close()

	_, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: key})
	assertCode(t, err, protocol.CodeModelDiscoveryFailed)
	if followed.Load() {
		t.Fatal("redirect target was requested")
	}
	assertSanitized(t, err, key, target.URL, "key-target")
}

func TestFetchBypassesEnvironmentProxy(t *testing.T) {
	var proxyRequests atomic.Int64
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		proxyRequests.Add(1)
		http.Error(w, "proxy-canary", http.StatusBadGateway)
	}))
	defer proxy.Close()
	t.Setenv("HTTP_PROXY", proxy.URL)
	t.Setenv("HTTPS_PROXY", proxy.URL)
	t.Setenv("NO_PROXY", "")

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = io.WriteString(w, `{"data":[{"id":"direct"}]}`)
	}))
	defer server.Close()
	models, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: "proxy-key"})
	if err != nil || len(models) != 1 || models[0] != "direct" {
		t.Fatalf("Fetch() = %q, %v", models, err)
	}
	if proxyRequests.Load() != 0 {
		t.Fatalf("proxy received %d requests", proxyRequests.Load())
	}
}

func TestFetchBodyLimit(t *testing.T) {
	prefix := `{"data":[{"id":"at-limit"}],"padding":"`
	suffix := `"}`
	atLimit := prefix + strings.Repeat("x", maxBodyBytes-len(prefix)-len(suffix)) + suffix
	tests := []struct {
		name string
		body string
		code protocol.ErrorCode
	}{
		{name: "exact limit", body: atLimit},
		{name: "over limit", body: strings.Repeat("x", maxBodyBytes+1), code: protocol.CodeModelResponseInvalid},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				_, _ = io.WriteString(w, test.body)
			}))
			defer server.Close()
			models, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: "limit-key"})
			if test.code == "" {
				if err != nil || len(models) != 1 || models[0] != "at-limit" {
					t.Fatalf("Fetch() = %q, %v", models, err)
				}
				return
			}
			assertCode(t, err, test.code)
		})
	}
}

func TestFetchValidatesResponseCatalog(t *testing.T) {
	uniqueOverflow := makeCatalog(maxModels + 1)
	tests := []struct {
		name string
		body string
		code protocol.ErrorCode
	}{
		{name: "malformed JSON", body: `{"data":[`},
		{name: "trailing JSON", body: `{"data":[{"id":"a"}]} {}`},
		{name: "invalid item", body: `{"data":[{"id":1}]}`},
		{name: "boundary whitespace", body: `{"data":[{"id":" model"}]}`},
		{name: "too many unique models", body: uniqueOverflow},
		{name: "empty catalog", body: `{"data":[]}`, code: protocol.CodeModelCatalogEmpty},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				_, _ = io.WriteString(w, test.body)
			}))
			defer server.Close()
			_, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: "response-key"})
			want := test.code
			if want == "" {
				want = protocol.CodeModelResponseInvalid
			}
			assertCode(t, err, want)
			assertSanitized(t, err, "response-key", test.body)
		})
	}
}

func makeCatalog(count int) string {
	var body strings.Builder
	body.WriteString(`{"data":[`)
	for i := range count {
		if i > 0 {
			body.WriteByte(',')
		}
		fmt.Fprintf(&body, `{"id":"model-%04d"}`, i)
	}
	body.WriteString(`]}`)
	return body.String()
}

func TestFetchTimeoutsDuringHeadersAndBody(t *testing.T) {
	if testing.Short() {
		t.Skip("exercises the fixed five-second HTTP deadline")
	}
	tests := []struct {
		name    string
		handler http.HandlerFunc
	}{
		{name: "headers", handler: func(http.ResponseWriter, *http.Request) { time.Sleep(requestTimeout + 250*time.Millisecond) }},
		{name: "body", handler: func(w http.ResponseWriter, _ *http.Request) {
			_, _ = io.WriteString(w, `{"data":[`)
			if flusher, ok := w.(http.Flusher); ok {
				flusher.Flush()
			}
			time.Sleep(requestTimeout + 250*time.Millisecond)
		}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewServer(test.handler)
			defer server.Close()
			started := time.Now()
			_, err := New(nil).Fetch(context.Background(), Request{URL: server.URL + "/v1/models", APIKey: "timeout-key"})
			assertCode(t, err, protocol.CodeModelDiscoveryFailed)
			if elapsed := time.Since(started); elapsed < 4*time.Second || elapsed > 6*time.Second {
				t.Fatalf("deadline elapsed = %v", elapsed)
			}
		})
	}
}

func TestFetchHonorsEarlierContextDeadline(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		time.Sleep(time.Second)
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	started := time.Now()
	_, err := New(nil).Fetch(ctx, Request{URL: server.URL + "/v1/models", APIKey: "context-key"})
	assertCode(t, err, protocol.CodeModelDiscoveryFailed)
	if elapsed := time.Since(started); elapsed > 500*time.Millisecond {
		t.Fatalf("parent deadline elapsed = %v", elapsed)
	}
}

func TestFetchRejectsInvalidEndpointWithoutSendingKey(t *testing.T) {
	var requests atomic.Int64
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		requests.Add(1)
	}))
	defer server.Close()
	for _, endpoint := range []string{
		server.URL + "/v1/models?query=1",
		server.URL + "/v1/models?",
		server.URL + "/v1/models#fragment",
		server.URL + "/other",
		"not a URL",
	} {
		_, err := New(nil).Fetch(context.Background(), Request{URL: endpoint, APIKey: "unsent-key"})
		assertCode(t, err, protocol.CodeModelDiscoveryFailed)
	}
	if requests.Load() != 0 {
		t.Fatalf("invalid endpoints received %d requests", requests.Load())
	}
}

func TestFetchSupportsInjectedTransport(t *testing.T) {
	transport := roundTripperFunc(func(request *http.Request) (*http.Response, error) {
		if request.URL.String() != "http://127.0.0.1:19099/v1/models" {
			t.Fatalf("URL = %q", request.URL)
		}
		return &http.Response{
			StatusCode: http.StatusOK,
			Header:     make(http.Header),
			Body:       io.NopCloser(strings.NewReader(`{"data":[{"id":"bound"}]}`)),
			Request:    request,
		}, nil
	})
	models, err := New(transport).Fetch(context.Background(), Request{
		URL: "http://127.0.0.1:19099/v1/models", APIKey: "bound-key",
	})
	if err != nil || len(models) != 1 || models[0] != "bound" {
		t.Fatalf("Fetch() = %q, %v", models, err)
	}
}

func TestFetchSanitizesTransportErrors(t *testing.T) {
	const key = "transport-key-canary"
	const detail = "transport-detail-canary"
	transport := roundTripperFunc(func(*http.Request) (*http.Response, error) {
		return nil, errors.New(detail)
	})
	_, err := New(transport).Fetch(context.Background(), Request{URL: "http://127.0.0.1:19099/v1/models", APIKey: key})
	assertCode(t, err, protocol.CodeModelDiscoveryFailed)
	assertSanitized(t, err, key, detail, "127.0.0.1:19099")
}

func TestFetchDoesNotWriteLogs(t *testing.T) {
	const key = "log-key-canary"
	oldStderr := os.Stderr
	read, write, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	os.Stderr = write
	t.Cleanup(func() { os.Stderr = oldStderr })
	transport := roundTripperFunc(func(*http.Request) (*http.Response, error) {
		return nil, errors.New(key)
	})
	_, fetchErr := New(transport).Fetch(context.Background(), Request{URL: "http://127.0.0.1:19099/v1/models", APIKey: key})
	_ = write.Close()
	logged, readErr := io.ReadAll(read)
	_ = read.Close()
	if readErr != nil {
		t.Fatal(readErr)
	}
	assertCode(t, fetchErr, protocol.CodeModelDiscoveryFailed)
	if len(logged) != 0 {
		t.Fatalf("stderr = %q", logged)
	}
}

type roundTripperFunc func(*http.Request) (*http.Response, error)

func (f roundTripperFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return f(request)
}

func assertSanitized(t *testing.T, err error, canaries ...string) {
	t.Helper()
	message := err.Error()
	for _, canary := range canaries {
		if canary != "" && strings.Contains(message, canary) {
			t.Fatalf("error %q contains canary %q", message, canary)
		}
	}
}
