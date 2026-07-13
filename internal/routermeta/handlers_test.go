package routermeta

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
	"time"

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
	for _, key := range []string{"version", "commit", "build_date", "deployment_id", "management_protocol_version", "pid", "started_at"} {
		if _, ok := got[key]; !ok {
			t.Fatalf("/version body missing %q (got %s)", key, rec.Body.String())
		}
	}
	if got["management_protocol_version"] == "" {
		t.Fatal("management_protocol_version must not be empty")
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

func TestVersionHandlerProviderCannotOverrideBuildOrProcessIdentity(t *testing.T) {
	rec := httptest.NewRecorder()
	VersionHandler(InfoProviderFunc(func() map[string]any {
		return map[string]any{
			"pid":                         -1,
			"deployment_id":               "forged",
			"management_protocol_version": "forged",
		}
	})).ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/version", nil))
	var got map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got["pid"] == float64(-1) || got["deployment_id"] == "forged" || got["management_protocol_version"] == "forged" {
		t.Fatalf("provider overrode trusted identity: %+v", got)
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

func TestHealthHandlerPassesRuntimeProbeOptions(t *testing.T) {
	want := health.ProbeOptions{
		UpstreamURL: "https://upstream.example.test",
		ClientCert:  "client-cert-pem",
		ClientKey:   "client-key-pem",
		UpstreamCA:  "upstream-ca-pem",
		TLSMin:      "tls1.3",
		Timeout:     2 * time.Second,
	}
	var got health.ProbeOptions
	probe := health.ProbeFunc(func(opts health.ProbeOptions) error {
		got = opts
		return nil
	})
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	HealthHandler(probe, want).ServeHTTP(rec, req)

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("probe options = %+v, want %+v", got, want)
	}
}

func TestHealthHandlerDefaultsProbeTimeoutWhenUnset(t *testing.T) {
	var got health.ProbeOptions
	probe := health.ProbeFunc(func(opts health.ProbeOptions) error {
		got = opts
		return nil
	})
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/health", nil)
	HealthHandler(probe, health.ProbeOptions{UpstreamURL: "https://upstream.example.test"}).ServeHTTP(rec, req)

	if got.Timeout != 5*time.Second {
		t.Fatalf("timeout = %s, want 5s", got.Timeout)
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
	const upstreamCanary = "https://user:auth-canary@upstream.example/private?api_key=sk-health-canary"
	probe := health.ProbeFunc(func(health.ProbeOptions) error {
		return errors.New("probe failed for " + upstreamCanary)
	})
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
	if got["upstream"] != "unreachable" {
		t.Fatalf("upstream = %q, want unreachable", got["upstream"])
	}
	if got["error"] != "upstream probe failed" {
		t.Fatalf("error = %q, want sanitized probe failure", got["error"])
	}
	if strings.Contains(rec.Body.String(), upstreamCanary) || strings.Contains(rec.Body.String(), "sk-health-canary") {
		t.Fatalf("health response leaked upstream detail: %q", rec.Body.String())
	}
}
