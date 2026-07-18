package proxy

import (
	"bytes"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	mlog "github.com/codeasier/mtls-router/internal/log"
)

const (
	contractModel        = "advertised-model-fixture"
	contractBearer       = "Bearer sk-fixture-auth-4d5e6f7g8h9i"
	contractRequestBody  = "request-body-canary-1a2b3c"
	contractResponseBody = "response-body-canary-0j1k2l"
	contractQuery        = "query-canary-3m4n5o"
)

type contractRequest struct {
	method  string
	uri     string
	body    string
	headers http.Header
	stream  bool
	release chan struct{}
}

type contractRoundTripper struct {
	base       http.RoundTripper
	failureErr error
}

func (t contractRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	if req.URL.Path == "/v1/failure" {
		return nil, t.failureErr
	}
	return t.base.RoundTrip(req)
}

func TestAdvertisedModelServiceContract(t *testing.T) {
	messagesBody := `{"model":"advertised-model-fixture","stream":true,"messages":[{"role":"user","content":"request-body-canary-1a2b3c"}],"tools":[{"type":"future_anthropic_tool_20991231","name":"deferred_fixture","description":"open-list tool fixture","defer_loading":true,"input_schema":{"type":"object","properties":{"value":{"type":"string"}}},"input_examples":[{"value":"deferred-field-canary"}]}]}`
	requests := []contractRequest{
		{method: http.MethodGet, uri: "/v1/models", headers: http.Header{"Authorization": {contractBearer}}},
		{
			method: http.MethodPost,
			uri:    "/v1/messages?beta=true",
			body:   messagesBody,
			headers: http.Header{
				"Authorization":            {contractBearer},
				"Content-Type":             {"application/json"},
				"Anthropic-Version":        {"2023-06-01"},
				"Anthropic-Beta":           {"tools-2025-04-14,deferred-tools-2025-06-01"},
				"Anthropic-Open-List-2099": {"open-list-header-canary"},
			},
			stream: true,
		},
		{
			method:  http.MethodPost,
			uri:     "/v1/messages/count_tokens",
			body:    `{"model":"advertised-model-fixture","messages":[{"role":"user","content":"request-body-canary-1a2b3c"}]}`,
			headers: http.Header{"Authorization": {contractBearer}, "Content-Type": {"application/json"}, "Anthropic-Version": {"2023-06-01"}},
		},
		{
			method:  http.MethodPost,
			uri:     "/v1/chat/completions",
			body:    `{"model":"advertised-model-fixture","stream":true,"messages":[{"role":"user","content":"request-body-canary-1a2b3c"}]}`,
			headers: http.Header{"Authorization": {contractBearer}, "Content-Type": {"application/json"}},
			stream:  true,
		},
		{
			method:  http.MethodPost,
			uri:     "/v1/completions",
			body:    `{"model":"advertised-model-fixture","stream":true,"prompt":"request-body-canary-1a2b3c"}`,
			headers: http.Header{"Authorization": {contractBearer}, "Content-Type": {"application/json"}},
			stream:  true,
		},
		{
			method:  http.MethodPost,
			uri:     "/v1/responses",
			body:    `{"model":"advertised-model-fixture","stream":true,"input":"request-body-canary-1a2b3c"}`,
			headers: http.Header{"Authorization": {contractBearer}, "Content-Type": {"application/json"}},
			stream:  true,
		},
	}
	for i := range requests {
		if requests[i].stream {
			requests[i].release = make(chan struct{})
		}
	}

	caPEM, _, ca, caKey := testCertificate(t, "contract-ca-cert-canary", true, nil, nil)
	clientCertPEM, clientKeyPEM, _, _ := testCertificate(t, "contract-client-cert-canary", false, ca, caKey)
	serverCertPEM, serverKeyPEM, _, _ := testCertificate(t, "server", false, ca, caKey)
	expected := make(map[string]contractRequest, len(requests))
	for _, request := range requests {
		expected[request.method+" "+request.uri] = request
	}
	upstream := newContractMTLSServer(t, caPEM, serverCertPEM, serverKeyPEM, expected)

	transport, err := NewMTLSTransport(clientCertPEM, clientKeyPEM, caPEM)
	if err != nil {
		t.Fatal(err)
	}
	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	upstreamURL, err := url.Parse(upstream.URL)
	if err != nil {
		t.Fatal(err)
	}
	reverseProxy := New(Options{Upstream: upstreamURL, Transport: transport, ErrorLog: logger})
	reverseProxy.Transport = contractRoundTripper{
		base: transport,
		failureErr: errors.New(strings.Join([]string{
			contractResponseBody,
			contractQuery,
			contractBearer,
			caPEM,
			clientCertPEM,
			clientKeyPEM,
		}, " ")),
	}
	downstream := httptest.NewServer(contractAccessLog(reverseProxy, logger))
	t.Cleanup(downstream.Close)

	for _, request := range requests {
		request := request
		t.Run(request.method+" "+request.uri, func(t *testing.T) {
			if request.stream {
				assertContractStream(t, downstream.Client(), downstream.URL, request)
				return
			}
			response := doContractRequest(t, downstream.Client(), downstream.URL, request)
			defer response.Body.Close()
			body, err := io.ReadAll(response.Body)
			if err != nil {
				t.Fatal(err)
			}
			if response.StatusCode != http.StatusOK {
				t.Fatalf("status = %d, body = %q", response.StatusCode, body)
			}
			if request.uri == "/v1/models" && string(body) != `{"object":"list","data":[{"id":"advertised-model-fixture","object":"model"}]}` {
				t.Fatalf("catalog body = %q", body)
			}
			if request.uri == "/v1/messages/count_tokens" && string(body) != `{"input_tokens":17}` {
				t.Fatalf("count_tokens body = %q", body)
			}
		})
	}

	failure := contractRequest{
		method:  http.MethodPost,
		uri:     "/v1/failure?api_key=" + contractQuery,
		body:    contractRequestBody,
		headers: http.Header{"Authorization": {contractBearer}, "Content-Type": {"application/json"}},
	}
	failureResponse := doContractRequest(t, downstream.Client(), downstream.URL, failure)
	failureBody, err := io.ReadAll(failureResponse.Body)
	_ = failureResponse.Body.Close()
	if err != nil {
		t.Fatal(err)
	}
	if failureResponse.StatusCode != http.StatusBadGateway || string(failureBody) != "{\"error\":\"Bad Gateway\"}\n" {
		t.Fatalf("failure response = %d %q", failureResponse.StatusCode, failureBody)
	}

	out := logs.String()
	for _, want := range []string{
		"path=/v1/models",
		"path=/v1/messages",
		"path=/v1/messages/count_tokens",
		"path=/v1/chat/completions",
		"path=/v1/completions",
		"path=/v1/responses",
		"msg=\"proxy request failed\"",
		"path=/v1/failure",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("raw logs missing %q: %s", want, out)
		}
	}
	for _, secret := range []string{
		contractBearer,
		strings.TrimPrefix(contractBearer, "Bearer "),
		contractRequestBody,
		contractResponseBody,
		contractQuery,
		"api_key",
		"beta=true",
		"Anthropic-Version",
		"Anthropic-Beta",
		"open-list-header-canary",
		"deferred-field-canary",
		"BEGIN CERTIFICATE",
		"BEGIN RSA PRIVATE KEY",
		"contract-ca-cert-canary",
		"contract-client-cert-canary",
		caPEM,
		clientCertPEM,
		clientKeyPEM,
	} {
		if strings.Contains(out, secret) || strings.Contains(string(failureBody), secret) {
			t.Errorf("raw logs/error response leaked %q", secret)
		}
	}
}

func newContractMTLSServer(t *testing.T, caPEM, certPEM, keyPEM string, expected map[string]contractRequest) *httptest.Server {
	t.Helper()
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		want, ok := expected[r.Method+" "+r.URL.RequestURI()]
		if !ok {
			http.Error(w, "unexpected request", http.StatusNotFound)
			t.Errorf("unexpected upstream request: %s %s", r.Method, r.URL.RequestURI())
			return
		}
		body, err := io.ReadAll(r.Body)
		if err != nil {
			http.Error(w, "body read failed", http.StatusInternalServerError)
			t.Errorf("read upstream request: %v", err)
			return
		}
		if string(body) != want.body {
			t.Errorf("%s body = %q, want %q", want.uri, body, want.body)
		}
		for name, values := range want.headers {
			if got := r.Header.Values(name); fmt.Sprint(got) != fmt.Sprint(values) {
				t.Errorf("%s header %s = %q, want %q", want.uri, name, got, values)
			}
		}
		if want.stream {
			w.Header().Set("Content-Type", "text/event-stream; charset=utf-8")
			_, _ = io.WriteString(w, "data: "+contractResponseBody+"-first\n\n")
			if err := http.NewResponseController(w).Flush(); err != nil {
				t.Errorf("flush upstream response: %v", err)
				return
			}
			<-want.release
			_, _ = io.WriteString(w, "data: "+contractResponseBody+"-second\n\n")
			return
		}
		w.Header().Set("Content-Type", "application/json")
		if want.uri == "/v1/models" {
			_, _ = io.WriteString(w, `{"object":"list","data":[{"id":"advertised-model-fixture","object":"model"}]}`)
			return
		}
		_, _ = io.WriteString(w, `{"input_tokens":17}`)
	}))
	configureTestMTLS(t, server, caPEM, certPEM, keyPEM)
	server.StartTLS()
	t.Cleanup(server.Close)
	return server
}

func configureTestMTLS(t *testing.T, server *httptest.Server, caPEM, certPEM, keyPEM string) {
	t.Helper()
	serverCert, err := tls.X509KeyPair([]byte(certPEM), []byte(keyPEM))
	if err != nil {
		t.Fatal(err)
	}
	clientCAs := x509.NewCertPool()
	if !clientCAs.AppendCertsFromPEM([]byte(caPEM)) {
		t.Fatal("parse fixture CA")
	}
	server.TLS = &tls.Config{
		Certificates: []tls.Certificate{serverCert},
		ClientCAs:    clientCAs,
		ClientAuth:   tls.RequireAndVerifyClientCert,
		MinVersion:   tls.VersionTLS12,
	}
}

func contractAccessLog(next http.Handler, logger *slog.Logger) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		recorder := &mlog.ResponseRecorder{ResponseWriter: w}
		start := time.Now()
		next.ServeHTTP(recorder, r)
		mlog.AccessLog(logger, r, recorder, start)
	})
}

func doContractRequest(t *testing.T, client *http.Client, baseURL string, request contractRequest) *http.Response {
	t.Helper()
	req, err := http.NewRequest(request.method, baseURL+request.uri, strings.NewReader(request.body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header = request.headers.Clone()
	response, err := client.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	return response
}

func assertContractStream(t *testing.T, client *http.Client, baseURL string, request contractRequest) {
	t.Helper()
	response := doContractRequest(t, client, baseURL, request)
	defer response.Body.Close()
	first := "data: " + contractResponseBody + "-first\n\n"
	result := make(chan error, 1)
	go func() {
		buf := make([]byte, len(first))
		_, err := io.ReadFull(response.Body, buf)
		if err == nil && string(buf) != first {
			err = fmt.Errorf("first SSE chunk = %q, want %q", buf, first)
		}
		result <- err
	}()
	select {
	case err := <-result:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("first SSE chunk was buffered")
	}
	select {
	case <-request.release:
		t.Fatal("stream release must remain blocked while first chunk is asserted")
	default:
	}
	if got := response.Header.Get("Content-Type"); got != "text/event-stream" {
		t.Errorf("Content-Type = %q", got)
	}
	if got := response.Header.Get("Cache-Control"); got != "no-cache" {
		t.Errorf("Cache-Control = %q", got)
	}
	if got := response.Header.Get("X-Accel-Buffering"); got != "no" {
		t.Errorf("X-Accel-Buffering = %q", got)
	}
	close(request.release)
	rest, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	wantRest := "data: " + contractResponseBody + "-second\n\n"
	if string(rest) != wantRest {
		t.Errorf("remaining SSE body = %q, want %q", rest, wantRest)
	}
}
