package routermeta

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/health"
)

func TestVersionHandlerReturnsJSON(t *testing.T) {
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/version", nil)
	VersionHandler(InfoProviderFunc(func() map[string]any { return map[string]any{"extra": "stub"} })).ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200", rec.Code)
	}
	if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "application/json") {
		t.Fatalf("Content-Type = %q, want application/json", ct)
	}
	var got map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("body not JSON: %v (body=%q)", err, rec.Body.String())
	}
	for _, key := range []string{"version", "commit", "build_date", "pid", "started_at"} {
		if _, ok := got[key]; !ok {
			t.Fatalf("/version body missing %q (got %s)", key, rec.Body.String())
		}
	}
}

func TestHealthHandlerReturnsOKWhenProbeSucceeds(t *testing.T) {
	probe := health.ProbeFunc(func(health.ProbeOptions) error { return nil })
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	HealthHandler(probe).ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200 (health must never fail the HTTP layer)", rec.Code)
	}
	var got map[string]string
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("body not JSON: %v", err)
	}
	if got["status"] != "ok" {
		t.Fatalf("status = %q, want ok", got["status"])
	}
}

func TestHealthHandlerReturnsDegradedButStill200WhenProbeFails(t *testing.T) {
	probe := health.ProbeFunc(func(health.ProbeOptions) error { return errors.New("upstream down") })
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	HealthHandler(probe).ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200 (health must never fail the HTTP layer)", rec.Code)
	}
	var got map[string]string
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("body not JSON: %v", err)
	}
	if got["status"] != "degraded" {
		t.Fatalf("status = %q, want degraded", got["status"])
	}
	if !strings.Contains(got["error"], "upstream down") {
		t.Fatalf("error = %q, want to contain %q", got["error"], "upstream down")
	}
}
