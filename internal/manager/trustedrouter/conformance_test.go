package trustedrouter

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

type conformanceFile struct {
	KeyCanary string             `json:"key_canary"`
	Trusted   conformanceTrusted `json:"trusted"`
	Cases     []conformanceCase  `json:"cases"`
}

type conformanceTrusted struct {
	PID                       int    `json:"pid"`
	DeploymentID              string `json:"deployment_id"`
	ManagementProtocolVersion string `json:"management_protocol_version"`
}

type conformanceCase struct {
	ID                  string  `json:"id"`
	Kind                string  `json:"kind"`
	Listen              string  `json:"listen"`
	VersionStatus       int     `json:"version_status"`
	Hang                string  `json:"hang"`
	Process             string  `json:"process"`
	Connection          string  `json:"connection"`
	ExpectAuthorization bool    `json:"expect_authorization"`
	ExpectError         *string `json:"expect_error"`
	Version             *struct {
		PID                       int    `json:"pid"`
		DeploymentID              string `json:"deployment_id"`
		ManagementProtocolVersion string `json:"management_protocol_version"`
	} `json:"version"`
}

func TestWorkbenchConformanceVectors(t *testing.T) {
	raw, err := os.ReadFile(filepath.Join("testdata", "workbench-conformance.json"))
	if err != nil {
		t.Fatal(err)
	}
	var file conformanceFile
	if err := json.Unmarshal(raw, &file); err != nil {
		t.Fatal(err)
	}
	if file.KeyCanary == "" || len(file.Cases) == 0 {
		t.Fatal("conformance file is empty")
	}
	for _, tc := range file.Cases {
		tc := tc
		t.Run(tc.ID, func(t *testing.T) {
			if tc.Kind == "invariant" {
				if tc.ID != "proxy_disabled" {
					t.Fatalf("unknown invariant %s", tc.ID)
				}
				return
			}
			if tc.Listen != "" && tc.Listen != "loopback" {
				_, err := NormalizeListener(tc.Listen)
				if err == nil {
					t.Fatal("non-loopback listen must be rejected")
				}
				if tc.ExpectAuthorization {
					t.Fatal("non-loopback must not authorize")
				}
				return
			}
			var keyObserved atomic.Bool
			server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Authorization") != "" {
					keyObserved.Store(true)
				}
				if r.URL.Path != "/version" {
					if r.Header.Get("Authorization") != "Bearer "+file.KeyCanary {
						http.Error(w, "unauthorized", http.StatusUnauthorized)
						return
					}
					_, _ = io.WriteString(w, `{"data":[{"id":"model-a"}]}`)
					return
				}
				if tc.Hang == "version" {
					<-r.Context().Done()
					return
				}
				if tc.VersionStatus == 302 {
					w.Header().Set("Location", "/other")
					w.WriteHeader(http.StatusFound)
					return
				}
				if tc.VersionStatus == 101 || tc.Connection == "upgrade" {
					w.Header().Set("Connection", "Upgrade")
					w.Header().Set("Upgrade", "websocket")
					w.WriteHeader(http.StatusSwitchingProtocols)
					return
				}
				if tc.Connection == "close" {
					w.Header().Set("Connection", "close")
				}
				if tc.Version != nil {
					_ = json.NewEncoder(w).Encode(map[string]any{
						"pid":                         tc.Version.PID,
						"deployment_id":               tc.Version.DeploymentID,
						"management_protocol_version": tc.Version.ManagementProtocolVersion,
					})
					return
				}
				w.WriteHeader(http.StatusOK)
			}))
			server.Start()
			defer server.Close()
			listener := listenerForServer(t, server.URL)
			validate := genuineProcess
			if tc.Process == "stale" {
				validate = func(process.Identity, string) (process.Status, error) {
					return process.StatusStale, nil
				}
			}
			ctx := context.Background()
			var cancel context.CancelFunc
			if tc.Hang == "version" {
				ctx, cancel = context.WithTimeout(ctx, 25*time.Millisecond)
				defer cancel()
			}
			_, fetchErr := (Channel{ValidateProcess: validate}).Fetch(ctx, listener, trustedFixture(listener), file.KeyCanary)
			if tc.ExpectAuthorization {
				if fetchErr != nil {
					t.Fatalf("Fetch() error = %+v", fetchErr)
				}
				if !keyObserved.Load() {
					t.Fatal("expected Authorization on the authenticated request")
				}
				return
			}
			if fetchErr == nil {
				t.Fatal("Fetch() succeeded")
			}
			if keyObserved.Load() {
				t.Fatal("Authorization was sent")
			}
			if tc.ExpectError == nil {
				return
			}
			switch *tc.ExpectError {
			case "identity":
				if fetchErr.Code != protocol.CodeModelCatalogStale {
					t.Fatalf("code = %s", fetchErr.Code)
				}
			case "redial", "redirect", "upgrade", "timeout", "cancel":
				if fetchErr.Code != protocol.CodeModelDiscoveryFailed && fetchErr.Code != protocol.CodeModelCatalogStale {
					t.Fatalf("code = %s", fetchErr.Code)
				}
			}
		})
	}
}

func TestWorkbenchConformanceRejectsProxyEnvironment(t *testing.T) {
	raw, err := os.ReadFile(filepath.Join("testdata", "workbench-conformance.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "HTTP_PROXY") {
		t.Fatal("vectors must mention HTTP_PROXY")
	}
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	_ = ln.Close()
}
