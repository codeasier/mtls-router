package trustedrouter_test

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/state"
	"github.com/codeasier/mtls-router/internal/manager/trustedrouter"
)

type conformanceVector struct {
	Name                string  `json:"name"`
	Scenario            string  `json:"scenario"`
	ExpectedOutcome     string  `json:"expected_outcome"`
	AuthMustNeverBeSent bool    `json:"auth_must_never_be_sent"`
	GoErrorCode         *string `json:"go_error_code"`
	RustError           string  `json:"rust_error"`
}

type conformanceFile struct {
	Description string              `json:"description"`
	Vectors     []conformanceVector `json:"vectors"`
	Invariants  []string            `json:"invariants"`
}

func TestConformanceVectors(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("testdata", "conformance_vectors.json"))
	if err != nil {
		t.Fatalf("read conformance vectors: %v", err)
	}
	var cf conformanceFile
	if err := json.Unmarshal(data, &cf); err != nil {
		t.Fatalf("parse conformance vectors: %v", err)
	}
	if len(cf.Vectors) == 0 {
		t.Fatal("no conformance vectors found")
	}
	for _, v := range cf.Vectors {
		t.Run(v.Name, func(t *testing.T) {
			if v.Scenario == "" {
				t.Error("scenario is empty")
			}
			if v.ExpectedOutcome != "fail_before_auth" && v.ExpectedOutcome != "fail_after_auth" && v.ExpectedOutcome != "success" {
				t.Errorf("expected_outcome = %q, must be fail_before_auth, fail_after_auth, or success", v.ExpectedOutcome)
			}
			if v.ExpectedOutcome == "fail_before_auth" && !v.AuthMustNeverBeSent {
				t.Error("fail_before_auth must have auth_must_never_be_sent = true")
			}
			if v.ExpectedOutcome != "success" && v.RustError == "" {
				t.Error("non-success outcome must have a rust_error")
			}
		})
	}
	for i, inv := range cf.Invariants {
		if inv == "" {
			t.Errorf("invariant %d is empty", i)
		}
	}
}

func TestConformanceVectorsCoverRequiredScenarios(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("testdata", "conformance_vectors.json"))
	if err != nil {
		t.Fatalf("read conformance vectors: %v", err)
	}
	var cf conformanceFile
	if err := json.Unmarshal(data, &cf); err != nil {
		t.Fatalf("parse conformance vectors: %v", err)
	}
	required := []string{
		"version_pid_mismatch",
		"version_deployment_mismatch",
		"version_protocol_mismatch",
		"process_identity_mismatch",
		"non_loopback_address",
		"redirect_response",
		"connection_close_after_version",
		"protocol_upgrade",
		"timeout_during_version",
		"cancel_during_generation",
		"redial_attempt",
		"health_not_ok",
	}
	names := make(map[string]bool, len(cf.Vectors))
	for _, v := range cf.Vectors {
		names[v.Name] = true
	}
	for _, req := range required {
		if !names[req] {
			t.Errorf("missing required conformance vector: %s", req)
		}
	}
}

func TestGoConformanceVectorsExecuteBeforeAuthFailures(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("testdata", "conformance_vectors.json"))
	if err != nil {
		t.Fatal(err)
	}
	var cf conformanceFile
	if err := json.Unmarshal(data, &cf); err != nil {
		t.Fatal(err)
	}
	for _, vector := range cf.Vectors {
		if vector.GoErrorCode == nil || vector.ExpectedOutcome != "fail_before_auth" {
			continue
		}
		t.Run(vector.Name, func(t *testing.T) {
			var authObserved atomic.Bool
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Authorization") != "" {
					authObserved.Store(true)
				}
				if r.URL.Path != "/version" {
					w.WriteHeader(http.StatusNoContent)
					return
				}
				switch vector.Name {
				case "redirect_response":
					w.Header().Set("Location", "/other")
					w.WriteHeader(http.StatusFound)
					return
				case "connection_close_after_version", "redial_attempt":
					w.Header().Set("Connection", "close")
				case "protocol_upgrade":
					w.Header().Set("Connection", "Upgrade")
					w.Header().Set("Upgrade", "websocket")
					w.WriteHeader(http.StatusSwitchingProtocols)
					return
				case "timeout_during_version":
					<-r.Context().Done()
					return
				}
				version := discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"}
				switch vector.Name {
				case "version_pid_mismatch":
					version.PID++
				case "version_deployment_mismatch":
					version.DeploymentID = "other"
				case "version_protocol_mismatch":
					version.ManagementProtocolVersion = "other"
				}
				_ = json.NewEncoder(w).Encode(version)
			}))
			defer server.Close()

			parsed, err := url.Parse(server.URL)
			if err != nil {
				t.Fatal(err)
			}
			listener, err := trustedrouter.NormalizeListener(parsed.Host)
			if err != nil {
				t.Fatal(err)
			}
			trusted := discovery.Result{
				Classification: discovery.ExternalCompatible,
				Owner:          "cli",
				ListenAddr:     listener.RouterBaseURL,
				Version:        discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"},
				State: state.RouterState{
					PID: 91, Owner: "cli", ListenAddr: listener.RouterBaseURL, BinaryPath: "/router",
					ProcessStartedAt: "start", ProcessExecutable: "/router", DeploymentID: "prod-a", ManagementProtocolVersion: "4",
				},
			}
			validate := func(process.Identity, string) (process.Status, error) {
				if vector.Name == "process_identity_mismatch" {
					return process.StatusStale, nil
				}
				return process.StatusGenuine, nil
			}
			ctx := context.Background()
			if vector.Name == "timeout_during_version" {
				var cancel context.CancelFunc
				ctx, cancel = context.WithTimeout(ctx, 20*time.Millisecond)
				defer cancel()
			}
			_, got := (trustedrouter.Channel{ValidateProcess: validate}).Fetch(ctx, listener, trusted, "key-canary")
			if got == nil || string(got.Code) != *vector.GoErrorCode {
				t.Fatalf("error = %#v, want code %s", got, *vector.GoErrorCode)
			}
			if authObserved.Load() {
				t.Fatal("Authorization was sent before trust completed")
			}
		})
	}
}
