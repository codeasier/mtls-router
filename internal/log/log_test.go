package mlog

import (
	"bytes"
	"errors"
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
	var buf bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&buf, nil))
	req := httptest.NewRequest(http.MethodPost, "/v1/chat?q=1", strings.NewReader("secret-body"))
	rec := &ResponseRecorder{Status: http.StatusBadGateway, Bytes: 12}
	AccessLog(logger, req, rec, time.Now().Add(-time.Millisecond), errors.New("boom"))
	out := buf.String()
	for _, want := range []string{"method=POST", "path=\"/v1/chat?q=1\"", "status=502", "bytes=12", "latency=", "error=boom"} {
		if !strings.Contains(out, want) {
			t.Fatalf("log missing %q: %s", want, out)
		}
	}
	if strings.Contains(out, "secret-body") {
		t.Fatal("body logged without explicit debug enable")
	}
}
