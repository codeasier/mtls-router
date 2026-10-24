package routerenroll

import (
	"bytes"
	"context"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestLoopbackAndServerBudgets(t *testing.T) {
	for _, address := range []string{"0.0.0.0:19101", "[::]:19101", "localhost:19101", "192.0.2.1:19101", "127.0.0.1:0", "127.0.0.1:65536", "[::1%zone]:19101"} {
		if ValidateListen(address) == nil {
			t.Fatalf("unsafe bind %s", address)
		}
	}
	for _, address := range []string{DefaultListen, "[::1]:19101"} {
		server, err := NewHTTPServer(address, NewHandler(nil), nil)
		if err != nil || server.ReadHeaderTimeout <= 0 || server.ReadTimeout <= 0 || server.WriteTimeout <= 0 || server.IdleTimeout <= 0 || server.MaxHeaderBytes != 16*1024 {
			t.Fatal("missing budget")
		}
	}
}

func TestEnrollmentRoutesAndClosedLogs(t *testing.T) {
	var log bytes.Buffer
	server, err := NewHTTPServer(DefaultListen, NewHandler(func(context.Context) bool { return true }), slog.New(slog.NewJSONHandler(&log, nil)))
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct {
		method, path string
		status       int
	}{
		{"GET", "/enroll/ready", 204}, {"POST", "/enroll/ready", 405},
		{"GET", "/enroll/ready/", 404}, {"GET", "/v1/models", 404},
		{"GET", "/enroll/gitcode/callback?code=oauth-code-canary&state=proof-canary", 503},
		{"private-key-canary", "/api-key-canary", 404},
	} {
		w := httptest.NewRecorder()
		r := httptest.NewRequest(test.method, test.path, strings.NewReader("private-key-canary"))
		server.Handler.ServeHTTP(w, r)
		if w.Code != test.status {
			t.Fatalf("%s: %d", test.path, w.Code)
		}
		if strings.Contains(w.Body.String(), "canary") {
			t.Fatal("HTTP leak")
		}
	}
	for _, secret := range []string{"oauth-code-canary", "proof-canary", "private-key-canary", "api-key-canary"} {
		if strings.Contains(log.String(), secret) {
			t.Fatal("log leak", secret)
		}
	}
	w := httptest.NewRecorder()
	r := httptest.NewRequest("GET", "/enroll/ready", nil)
	r.Header.Set("Authorization", "Bearer business-key-canary")
	server.Handler.ServeHTTP(w, r)
	if w.Code != 400 || strings.Contains(w.Body.String()+log.String(), "business-key-canary") {
		t.Fatal("business key accepted/leaked")
	}
}

func TestEnrollmentReferrerPolicyKeepsSameOriginForms(t *testing.T) {
	server, err := NewHTTPServer(DefaultListen, NewHandler(func(context.Context) bool { return true }), nil)
	if err != nil {
		t.Fatal(err)
	}
	w := httptest.NewRecorder()
	server.Handler.ServeHTTP(w, httptest.NewRequest("GET", "/enroll/ready", nil))
	if got := w.Header().Get("Referrer-Policy"); got != "same-origin" {
		t.Fatalf("referrer policy %q", got)
	}
}

func TestBodyBudgetStrictDecodeAndPanic(t *testing.T) {
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body NativeStart
		if decodeJSON(w, r, &body) {
			w.WriteHeader(204)
		}
	})
	server, err := NewHTTPServer(DefaultListen, handler, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, body := range []string{`{"private_key":"canary"}`, `{}` + `{}`, `{"csr_pem":"` + strings.Repeat("x", MaxBodyBytes) + `"}`} {
		r := httptest.NewRequest("POST", "/enroll/native/start", strings.NewReader(body))
		r.Header.Set("Content-Type", "application/json")
		w := httptest.NewRecorder()
		server.Handler.ServeHTTP(w, r)
		if w.Code != 400 {
			t.Fatal("accepted invalid/oversized request")
		}
	}
	server, _ = NewHTTPServer(DefaultListen, http.HandlerFunc(func(http.ResponseWriter, *http.Request) { panic("CA-key-canary") }), nil)
	w := httptest.NewRecorder()
	server.Handler.ServeHTTP(w, httptest.NewRequest("GET", "/enroll/ready", nil))
	if w.Code != 503 || strings.Contains(w.Body.String(), "canary") {
		t.Fatal("panic leak")
	}
}

func TestGracefulServeAndOccupiedPort(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server, err := NewHTTPServer(listener.Addr().String(), NewHandler(func(context.Context) bool { return true }), nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := Serve(context.Background(), server); !errors.Is(err, ErrListen) {
		t.Fatal("occupied port must fail without moving")
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- serveListener(ctx, server, listener) }()
	client := &http.Client{Timeout: time.Second}
	response, err := client.Get("http://" + listener.Addr().String() + "/enroll/ready")
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, response.Body)
	_ = response.Body.Close()
	client.CloseIdleConnections()
	if response.StatusCode != 204 {
		t.Fatal(response.StatusCode)
	}
	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(ShutdownTimeout + time.Second):
		t.Fatal("shutdown exceeded budget")
	}
}
