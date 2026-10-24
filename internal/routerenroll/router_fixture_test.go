package routerenroll

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
)

// This checks the public golden document, not the still-legacy runtime. The
// shared Rust core/discovery must consume it when implementing tasks 5.1–5.6.
func TestRouterGenerationFixture(t *testing.T) {
	data, err := os.ReadFile("../../testdata/router-mtls-v1/router.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Version  map[string]any `json:"version"`
		State    map[string]any `json:"cli_state"`
		Requires []string       `json:"trust_requires"`
		Routes   []struct {
			Path         string
			Management   bool
			ProxyLimited bool `json:"proxy_limited"`
			Status       int
		} `json:"management_routes"`
		HTTP struct {
			Replay     bool   `json:"replay_sent_requests"`
			Calls      int    `json:"control_plane_model_calls"`
			Required   string `json:"missing_identity_business_error"`
			Management bool   `json:"missing_identity_local_management"`
			Updater    bool   `json:"missing_identity_updater"`
		} `json:"http"`
		Differences []string `json:"intentional_legacy_differences"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, value := range []map[string]any{fixture.Version, fixture.State} {
		if value["management_protocol_version"] != "4" || value["upstream_auth"] != "per_user_mtls" || value["auth_protocol_version"] != ProtocolVersion {
			t.Fatal("generation fixture drift")
		}
		for name := range value {
			for _, secret := range []string{"key", "token", "verifier", "proof", "pem", "credential", "csr", "state_path"} {
				if strings.Contains(name, secret) {
					t.Fatalf("private/arbitrary-source field: %s", name)
				}
			}
		}
	}
	if fixture.State["owner"] != "cli" || len(fixture.Requires) != 8 || len(fixture.Differences) == 0 {
		t.Fatal("ownership fixture incomplete")
	}
	for _, route := range fixture.Routes {
		want := route.Path == "/version" || route.Path == "/health"
		if route.Management != want || route.ProxyLimited || (route.Path == "/health" && route.Status != 200) {
			t.Fatal("management exactness drift")
		}
	}
	if fixture.HTTP.Replay || fixture.HTTP.Calls != 0 || fixture.HTTP.Required != ClientCertRequired || !fixture.HTTP.Management || !fixture.HTTP.Updater {
		t.Fatal("HTTP isolation drift")
	}
}
