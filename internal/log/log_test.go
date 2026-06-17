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

func TestResponseRecorderRecordsStatusAndBytes(t *testing.T) {
	rr := &ResponseRecorder{ResponseWriter: httptest.NewRecorder()}
	rr.WriteHeader(http.StatusCreated)
	_, _ = rr.Write([]byte("abc"))
	if rr.Status != http.StatusCreated || rr.Bytes != 3 {
		t.Fatalf("status=%d bytes=%d", rr.Status, rr.Bytes)
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
