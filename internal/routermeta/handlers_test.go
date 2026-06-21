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
	if cc := rec.Header().Get("Cache-Control"); cc != "no-store" {
		t.Fatalf("Cache-Control = %q, want no-store", cc)
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

func TestVersionHandlerProviderCanKeepStartedAtStable(t *testing.T) {
	handler := VersionHandler(InfoProviderFunc(func() map[string]any {
		return map[string]any{"started_at": "2026-06-21T00:00:00Z"}
	}))

	first := httptest.NewRecorder()
	handler.ServeHTTP(first, httptest.NewRequest(http.MethodGet, "/version", nil))
	second := httptest.NewRecorder()
	handler.ServeHTTP(second, httptest.NewRequest(http.MethodGet, "/version", nil))

	for _, rec := range []*httptest.ResponseRecorder{first, second} {
		var got map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
			t.Fatalf("body not JSON: %v", err)
		}
		if got["started_at"] != "2026-06-21T00:00:00Z" {
			t.Fatalf("started_at = %q, want stable provider value", got["started_at"])
		}
	}
}

func TestVersionHandlerRejectsNonGET(t *testing.T) {
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/version", nil)
	VersionHandler(nil).ServeHTTP(rec, req)

	assertMethodNotAllowed(t, rec)
}

func TestHealthHandlerRejectsNonGET(t *testing.T) {
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodPost, "/health", nil)
	HealthHandler(health.ProbeFunc(func(health.ProbeOptions) error { return nil })).ServeHTTP(rec, req)

	assertMethodNotAllowed(t, rec)
}

func assertMethodNotAllowed(t *testing.T, rec *httptest.ResponseRecorder) {
	t.Helper()
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("status = %d, want 405", rec.Code)
	}
	if allow := rec.Header().Get("Allow"); allow != http.MethodGet {
		t.Fatalf("Allow = %q, want GET", allow)
	}
	if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "application/json") {
		t.Fatalf("Content-Type = %q, want application/json", ct)
	}
	var got map[string]string
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("body not JSON: %v", err)
	}
	if got["error"] != "method not allowed" {
		t.Fatalf("error = %q, want method not allowed", got["error"])
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
