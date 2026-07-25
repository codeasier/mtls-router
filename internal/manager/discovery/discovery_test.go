package discovery

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

func TestDiscoverClassifiesCorrelatedRouters(t *testing.T) {
	for _, tt := range []struct {
		name       string
		owner      string
		health     string
		deployment string
		want       Classification
	}{
		{name: "desktop", owner: "desktop", health: "ok", deployment: "prod-a", want: DesktopOwned},
		{name: "external", owner: "cli", health: "ok", deployment: "prod-a", want: ExternalCompatible},
		{name: "degraded", owner: "cli", health: "degraded", deployment: "prod-a", want: Degraded},
		{name: "development external reuse disabled", owner: "cli", health: "ok", deployment: "dev", want: UnknownOccupant},
	} {
		t.Run(tt.name, func(t *testing.T) {
			server := routerServer(t, 73, tt.deployment, "1", tt.health)
			states := map[string]state.RouterState{"desktop": {}, "cli": {}}
			value := completeState(tt.owner, 73, tt.deployment)
			value.ListenAddr = server.URL
			states[tt.owner] = value
			d := New(Config{
				BaseURL: server.URL, DesktopStatePath: "desktop", CLIStatePath: "cli",
				DeploymentID: tt.deployment, ManagementProtocolVersion: "1",
				ReadState: func(path string) (state.RouterState, error) {
					value := states[path]
					if value.PID == 0 {
						return state.RouterState{}, os.ErrNotExist
					}
					return value, nil
				},
				ValidateProcess: func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
			})
			got := d.Discover(context.Background())
			if got.Classification != tt.want {
				t.Fatalf("classification = %q, want %q; result=%+v", got.Classification, tt.want, got)
			}
		})
	}
}

func TestDiscoverStatusDoesNotProbeHealth(t *testing.T) {
	var healthCalls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/version":
			_ = json.NewEncoder(w).Encode(Version{Version: "v1", PID: 73, DeploymentID: "prod-a", ManagementProtocolVersion: "1"})
		case "/health":
			healthCalls.Add(1)
			time.Sleep(100 * time.Millisecond)
			_ = json.NewEncoder(w).Encode(Health{Status: "ok"})
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)

	d := correlatedDesktopDiscoverer(server.URL, 20*time.Millisecond, 150*time.Millisecond)
	if got := d.DiscoverStatus(context.Background()); got.Classification != DesktopOwned {
		t.Fatalf("classification = %q, want %q; result=%+v", got.Classification, DesktopOwned, got)
	}
	if got := healthCalls.Load(); got != 0 {
		t.Fatalf("health calls = %d, want 0", got)
	}
}

func TestDiscoverAcceptsSlowHealthyResponse(t *testing.T) {
	var healthCalls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/version":
			_ = json.NewEncoder(w).Encode(Version{Version: "v1", PID: 73, DeploymentID: "prod-a", ManagementProtocolVersion: "1"})
		case "/health":
			healthCalls.Add(1)
			time.Sleep(40 * time.Millisecond)
			_ = json.NewEncoder(w).Encode(Health{Status: "ok"})
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)

	got := correlatedDesktopDiscoverer(server.URL, 20*time.Millisecond, 100*time.Millisecond).Discover(context.Background())
	if got.Classification != DesktopOwned || got.Health.Status != "ok" {
		t.Fatalf("result = %+v", got)
	}
	if got := healthCalls.Load(); got != 1 {
		t.Fatalf("health calls = %d, want 1", got)
	}
}

func TestGetJSONReusesConnectionAfterTrailingBody(t *testing.T) {
	var newConnections atomic.Int32
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(Version{Version: "v1"})
		_, _ = w.Write([]byte(strings.Repeat(" ", 32*1024)))
	}))
	server.Config.ConnState = func(_ net.Conn, state http.ConnState) {
		if state == http.StateNew {
			newConnections.Add(1)
		}
	}
	server.Start()
	t.Cleanup(server.Close)

	d := New(Config{BaseURL: server.URL})
	for range 2 {
		var version Version
		if err := d.getJSON(context.Background(), "/version", &version, time.Second); err != nil {
			t.Fatal(err)
		}
	}
	if got := newConnections.Load(); got != 1 {
		t.Fatalf("new connections = %d, want 1", got)
	}
}

func TestGetJSONReusesConnectionAfterNonOKBody(t *testing.T) {
	var newConnections atomic.Int32
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = w.Write([]byte(strings.Repeat("unavailable", 4096)))
	}))
	server.Config.ConnState = func(_ net.Conn, state http.ConnState) {
		if state == http.StateNew {
			newConnections.Add(1)
		}
	}
	server.Start()
	t.Cleanup(server.Close)

	d := New(Config{BaseURL: server.URL})
	for range 2 {
		var version Version
		if err := d.getJSON(context.Background(), "/version", &version, time.Second); err == nil {
			t.Fatal("getJSON unexpectedly succeeded")
		}
	}
	if got := newConnections.Load(); got != 1 {
		t.Fatalf("new connections = %d, want 1", got)
	}
}

func TestEndpointOnlyAndMetadataMismatchAreUnknown(t *testing.T) {
	for _, tt := range []struct{ name, deployment, protocol string }{
		{name: "no state", deployment: "prod-a", protocol: "1"},
		{name: "deployment mismatch", deployment: "prod-b", protocol: "1"},
		{name: "protocol mismatch", deployment: "prod-a", protocol: "2"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var healthCalls atomic.Int32
			mux := http.NewServeMux()
			mux.HandleFunc("/version", func(w http.ResponseWriter, _ *http.Request) {
				_ = json.NewEncoder(w).Encode(Version{Version: "v1", PID: 73, DeploymentID: tt.deployment, ManagementProtocolVersion: tt.protocol})
			})
			mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
				healthCalls.Add(1)
				_ = json.NewEncoder(w).Encode(Health{Status: "ok"})
			})
			server := httptest.NewServer(mux)
			t.Cleanup(server.Close)
			d := New(Config{BaseURL: server.URL, DeploymentID: "prod-a", ManagementProtocolVersion: "1"})
			if got := d.Discover(context.Background()); got.Classification != UnknownOccupant {
				t.Fatalf("classification = %q, want unknown_occupant", got.Classification)
			}
			if got := healthCalls.Load(); got != 0 {
				t.Fatalf("health calls = %d, want 0", got)
			}
		})
	}
}

func TestUnknownServiceAndBoundedTimeout(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		time.Sleep(100 * time.Millisecond)
		_, _ = w.Write([]byte("not json"))
	}))
	t.Cleanup(server.Close)
	started := time.Now()
	got := New(Config{
		BaseURL: server.URL, RequestTimeout: 20 * time.Millisecond, HealthRequestTimeout: 20 * time.Millisecond,
	}).Discover(context.Background())
	if got.Classification != UnknownOccupant {
		t.Fatalf("classification = %q", got.Classification)
	}
	if elapsed := time.Since(started); elapsed > 90*time.Millisecond {
		t.Fatalf("discovery took %s, request timeout was not bounded", elapsed)
	}
}

func TestAbsentAndStaleWithoutListener(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	baseURL := "http://" + listener.Addr().String()
	_ = listener.Close()

	if got := New(Config{BaseURL: baseURL}).Discover(context.Background()); got.Classification != Absent {
		t.Fatalf("missing classification = %q, want absent", got.Classification)
	}
	d := New(Config{
		BaseURL: baseURL, DesktopStatePath: "desktop",
		ReadState: func(string) (state.RouterState, error) { return completeState("desktop", 73, "prod-a"), nil },
		ValidateProcess: func(process.Identity, string) (process.Status, error) {
			return process.StatusStale, errors.New("denied")
		},
	})
	if got := d.Discover(context.Background()); got.Classification != Stale {
		t.Fatalf("stale classification = %q, want stale", got.Classification)
	}
}

func TestDesktopStartIgnoresOnlyStaleCLIStateWithoutListener(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	baseURL := "http://" + listener.Addr().String()
	_ = listener.Close()

	d := New(Config{
		BaseURL: baseURL, DesktopStatePath: "desktop", CLIStatePath: "cli",
		ReadState: func(path string) (state.RouterState, error) {
			if path == "desktop" {
				return state.RouterState{}, os.ErrNotExist
			}
			return completeState("cli", 73, "prod-a"), nil
		},
		ValidateProcess: func(process.Identity, string) (process.Status, error) {
			return process.StatusStale, nil
		},
	})

	if got := d.DiscoverStatus(context.Background()); got.Classification != Stale {
		t.Fatalf("generic classification = %q, want stale", got.Classification)
	}
	if got := d.DiscoverStartupStatus(context.Background(), protocol.RouterOwnerDesktop); got.Classification != Absent {
		t.Fatalf("desktop classification = %q, want absent", got.Classification)
	}
	if got := d.DiscoverStartupStatus(context.Background(), protocol.RouterOwnerCLI); got.Classification != Stale {
		t.Fatalf("CLI classification = %q, want stale", got.Classification)
	}
}

func TestDesktopStartRemainsBlockedByCurrentState(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	baseURL := "http://" + listener.Addr().String()
	_ = listener.Close()

	for _, tt := range []struct {
		name      string
		statePath string
		value     state.RouterState
		status    process.Status
		want      Classification
		wantOwner string
	}{
		{name: "stale desktop", statePath: "desktop", value: completeState("desktop", 73, "prod-a"), status: process.StatusStale, want: Stale},
		{name: "genuine CLI", statePath: "cli", value: completeState("cli", 73, "prod-a"), status: process.StatusGenuine, want: Degraded, wantOwner: "cli"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			d := New(Config{
				BaseURL: baseURL, DesktopStatePath: "desktop", CLIStatePath: "cli",
				DeploymentID: "prod-a", ManagementProtocolVersion: "1",
				ReadState: func(path string) (state.RouterState, error) {
					if path != tt.statePath {
						return state.RouterState{}, os.ErrNotExist
					}
					return tt.value, nil
				},
				ValidateProcess: func(process.Identity, string) (process.Status, error) {
					return tt.status, nil
				},
			})

			got := d.DiscoverStartupStatus(context.Background(), protocol.RouterOwnerDesktop)
			if got.Classification != tt.want || got.Owner != tt.wantOwner {
				t.Fatalf("result = %+v, want classification %q owner %q", got, tt.want, tt.wantOwner)
			}
		})
	}
}

func TestDesktopStartDoesNotIgnoreCorrelatedStaleCLIStateWithListener(t *testing.T) {
	server := routerServer(t, 73, "prod-a", "1", "ok")
	value := completeState("cli", 73, "prod-a")
	value.ListenAddr = server.URL
	d := New(Config{
		BaseURL: server.URL, DesktopStatePath: "desktop", CLIStatePath: "cli",
		DeploymentID: "prod-a", ManagementProtocolVersion: "1",
		ReadState: func(path string) (state.RouterState, error) {
			if path == "desktop" {
				return state.RouterState{}, os.ErrNotExist
			}
			return value, nil
		},
		ValidateProcess: func(process.Identity, string) (process.Status, error) {
			return process.StatusStale, nil
		},
	})

	if got := d.DiscoverStartupStatus(context.Background(), protocol.RouterOwnerDesktop); got.Classification != Stale {
		t.Fatalf("listener-present classification = %q, want stale", got.Classification)
	}
}

func TestGenericStatusPrefersRunningDesktopOverStaleCLIState(t *testing.T) {
	server := routerServer(t, 73, "prod-a", "1", "ok")
	desktop := completeState("desktop", 73, "prod-a")
	desktop.ListenAddr = server.URL
	cli := completeState("cli", 99, "prod-a")
	cli.ListenAddr = server.URL
	d := New(Config{
		BaseURL: server.URL, DesktopStatePath: "desktop", CLIStatePath: "cli",
		DeploymentID: "prod-a", ManagementProtocolVersion: "1",
		ReadState: func(path string) (state.RouterState, error) {
			if path == "desktop" {
				return desktop, nil
			}
			return cli, nil
		},
		ValidateProcess: func(identity process.Identity, _ string) (process.Status, error) {
			if identity.PID == cli.PID {
				return process.StatusStale, nil
			}
			return process.StatusGenuine, nil
		},
	})

	if got := d.DiscoverStatus(context.Background()); got.Classification != DesktopOwned || got.Owner != "desktop" {
		t.Fatalf("post-start status = %+v, want desktop_owned", got)
	}
}

func TestGenuineStateWithUnavailableEndpointIsDegraded(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	baseURL := "http://" + listener.Addr().String()
	_ = listener.Close()
	d := New(Config{
		BaseURL: baseURL, DesktopStatePath: "desktop", DeploymentID: "prod-a", ManagementProtocolVersion: "1",
		ReadState:       func(string) (state.RouterState, error) { return completeState("desktop", 73, "prod-a"), nil },
		ValidateProcess: func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
	})
	got := d.Discover(context.Background())
	if got.Classification != Degraded || got.Owner != "desktop" || got.State.PID != 73 {
		t.Fatalf("result = %+v", got)
	}
}

func TestHealthyDesktopRouterWithAbsentManagerIsStale(t *testing.T) {
	server := routerServer(t, 73, "prod-a", "1", "ok")
	value := completeState("desktop", 73, "prod-a")
	value.ListenAddr = server.URL
	d := New(Config{
		BaseURL: server.URL, DesktopStatePath: "desktop", DeploymentID: "prod-a", ManagementProtocolVersion: "1",
		ReadState: func(string) (state.RouterState, error) { return value, nil },
		ValidateProcess: func(identity process.Identity, _ string) (process.Status, error) {
			if identity.PID == value.ManagerPID {
				return process.StatusAbsent, nil
			}
			return process.StatusGenuine, nil
		},
	})
	if got := d.Discover(context.Background()); got.Classification != Stale {
		t.Fatalf("classification = %q, want stale", got.Classification)
	}
}

func routerServer(t *testing.T, pid int, deployment, protocol, health string) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.HandleFunc("/version", func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(Version{Version: "v1", PID: pid, DeploymentID: deployment, ManagementProtocolVersion: protocol})
	})
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(Health{Status: health})
	})
	server := httptest.NewServer(mux)
	t.Cleanup(server.Close)
	return server
}

func correlatedDesktopDiscoverer(baseURL string, requestTimeout, healthRequestTimeout time.Duration) *Discoverer {
	value := completeState("desktop", 73, "prod-a")
	value.ListenAddr = baseURL
	return New(Config{
		BaseURL: baseURL, DesktopStatePath: "desktop", DeploymentID: "prod-a", ManagementProtocolVersion: "1",
		RequestTimeout: requestTimeout, HealthRequestTimeout: healthRequestTimeout,
		ReadState:       func(string) (state.RouterState, error) { return value, nil },
		ValidateProcess: func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
	})
}

func completeState(owner string, pid int, deployment string) state.RouterState {
	value := state.RouterState{
		PID: pid, Owner: owner, ListenAddr: "http://127.0.0.1:19099", BinaryPath: "/router", LogPath: "/router.log",
		ProcessStartedAt: "router-start", ProcessExecutable: "/router", RouterVersion: "v1", DeploymentID: deployment, ManagementProtocolVersion: "1",
	}
	if owner == "desktop" {
		value.DesktopSessionID = "session"
		value.ManagerPID = 74
		value.ManagerProcessStartedAt = "manager-start"
		value.ManagerProcessExecutable = "/manager"
		value.ManagerVersion = "v1"
	}
	return value
}
