// Package routermeta exposes the management endpoints /version and /health
// for the running mtls-router process. These endpoints are mounted on the
// same listener as the reverse proxy but are NOT forwarded to the upstream.
package routermeta

import (
	"encoding/json"
	"net/http"
	"os"
	"time"

	"github.com/codeasier/mtls-router/internal/health"
	"github.com/codeasier/mtls-router/internal/version"
)

// InfoProvider supplies extra /version body fields. The default VersionHandler
// implementation always includes version/commit/build_date/pid/started_at;
// a non-nil provider may add or override non-identity keys and is called for
// each request.
type InfoProvider interface {
	Info() map[string]any
}

// InfoProviderFunc adapts a plain function to InfoProvider.
type InfoProviderFunc func() map[string]any

func (f InfoProviderFunc) Info() map[string]any { return f() }

func writeMethodNotAllowed(w http.ResponseWriter) {
	w.Header().Set("Allow", http.MethodGet)
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusMethodNotAllowed)
	_ = json.NewEncoder(w).Encode(map[string]string{"error": "method not allowed"})
}

// VersionHandler returns an http.Handler that responds to GET /version with
// JSON describing the running binary and process.
func VersionHandler(provider InfoProvider) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			writeMethodNotAllowed(w)
			return
		}
		info := version.Info()
		payload := map[string]any{
			"started_at": time.Now().UTC().Format(time.RFC3339),
		}
		if provider != nil {
			for k, v := range provider.Info() {
				payload[k] = v
			}
		}
		// Build and process identity cannot be overridden by a provider.
		payload["version"] = info.Version
		payload["commit"] = info.Commit
		payload["build_date"] = info.BuildDate
		payload["deployment_id"] = info.DeploymentID
		payload["management_protocol_version"] = info.ManagementProtocolVersion
		payload["pid"] = os.Getpid()
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		_ = json.NewEncoder(w).Encode(payload)
	})
}

// HealthHandler returns an http.Handler that always responds with 200 and
// reports upstream mTLS+TCP reachability in the JSON body. The HTTP status
// is intentionally never changed — "connection refused = not started, 200 =
// started (possibly degraded)" is the only way to keep the setup script's
// logic unambiguous.
func HealthHandler(probe health.ProbeFunc) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			writeMethodNotAllowed(w)
			return
		}
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		err := probe()
		if err == nil {
			_ = json.NewEncoder(w).Encode(map[string]string{
				"status":   "ok",
				"upstream": "reachable",
			})
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]string{
			"status":   "degraded",
			"upstream": "unreachable",
			"error":    "upstream probe failed",
		})
	})
}
