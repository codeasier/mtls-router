package mlog

import (
	"bytes"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

type flushWriter struct {
	http.ResponseWriter
	flushed bool
}

func (w *flushWriter) Flush() {
	w.flushed = true
}

func TestResponseRecorderRecordsStatusAndBytes(t *testing.T) {
	rr := &ResponseRecorder{ResponseWriter: httptest.NewRecorder()}
	rr.WriteHeader(http.StatusCreated)
	_, _ = rr.Write([]byte("abc"))
	if rr.Status != http.StatusCreated || rr.Bytes != 3 {
		t.Fatalf("status=%d bytes=%d", rr.Status, rr.Bytes)
	}
}

func TestResponseRecorderUnwrapAllowsResponseControllerFlush(t *testing.T) {
	underlying := &flushWriter{ResponseWriter: httptest.NewRecorder()}
	recorder := &ResponseRecorder{ResponseWriter: underlying}

	if _, ok := any(recorder).(http.Flusher); ok {
		t.Fatal("ResponseRecorder must not unconditionally implement http.Flusher")
	}
	if err := http.NewResponseController(recorder).Flush(); err != nil {
		t.Fatalf("Flush() error = %v", err)
	}
	if !underlying.flushed {
		t.Fatal("Flush() did not reach the underlying ResponseWriter")
	}
}

func TestAccessLog(t *testing.T) {
	const (
		queryCanary        = "sk-query-canary-1a2b3c"
		authCanary         = "Bearer auth-canary-4d5e6f"
		requestBodyCanary  = "request-body-canary-7g8h9i"
		responseBodyCanary = "response-body-canary-0j1k2l"
	)
	var buf bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&buf, nil))
	req := httptest.NewRequest(http.MethodPost, "/v1/chat?api_key="+queryCanary, strings.NewReader(requestBodyCanary))
	req.Header.Set("Authorization", authCanary)
	response := httptest.NewRecorder()
	_, _ = response.WriteString(responseBodyCanary)
	rec := &ResponseRecorder{ResponseWriter: response, Status: http.StatusBadGateway, Bytes: 12}
	AccessLog(logger, req, rec, time.Now().Add(-time.Millisecond))
	out := buf.String()
	for _, want := range []string{"method=POST", "path=/v1/chat", "status=502", "bytes=12", "latency="} {
		if !strings.Contains(out, want) {
			t.Fatalf("log missing %q: %s", want, out)
		}
	}
	for _, canary := range []string{queryCanary, authCanary, requestBodyCanary, responseBodyCanary, "api_key", "Authorization"} {
		if strings.Contains(out, canary) {
			t.Fatalf("access log leaked %q: %s", canary, out)
		}
	}
}
