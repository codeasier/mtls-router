package trustedrouter

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/apikeyusage"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

const channelKeyCanary = "channel-key-canary-7831"

func TestChannelValidatesAndFetchesOnExactlyOneConnection(t *testing.T) {
	type connectionKey struct{}
	var nextID atomic.Int64
	var mu sync.Mutex
	requestConnections := []int64{}
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := r.Context().Value(connectionKey{}).(int64)
		mu.Lock()
		requestConnections = append(requestConnections, id)
		mu.Unlock()
		switch r.URL.Path {
		case "/version":
			if r.Header.Get("Authorization") != "" {
				t.Error("version request contained Authorization")
			}
			_ = json.NewEncoder(w).Encode(discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"})
		case "/v1/models":
			if r.Header.Get("Authorization") != "Bearer "+channelKeyCanary {
				t.Errorf("Authorization = %q", r.Header.Get("Authorization"))
			}
			_, _ = io.WriteString(w, `{"data":[{"id":"model-a"}]}`)
		default:
			http.NotFound(w, r)
		}
	}))
	server.Config.ConnContext = func(ctx context.Context, _ net.Conn) context.Context {
		return context.WithValue(ctx, connectionKey{}, nextID.Add(1))
	}
	server.Start()
	defer server.Close()

	listener := listenerForServer(t, server.URL)
	dials := atomic.Int64{}
	models, err := (Channel{
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			dials.Add(1)
			return (&net.Dialer{}).DialContext(ctx, network, address)
		},
		ValidateProcess: genuineProcess,
	}).Fetch(context.Background(), listener, trustedFixture(listener), channelKeyCanary)
	if err != nil || len(models) != 1 || models[0] != "model-a" {
		t.Fatalf("Fetch() = %q, %+v", models, err)
	}
	mu.Lock()
	defer mu.Unlock()
	if dials.Load() != 1 || len(requestConnections) != 2 || requestConnections[0] != requestConnections[1] {
		t.Fatalf("dials=%d request connections=%v", dials.Load(), requestConnections)
	}
}

func TestChannelFetchesUsageOnExactlyOneConnection(t *testing.T) {
	type connectionKey struct{}
	var nextID atomic.Int64
	var mu sync.Mutex
	requestConnections := []int64{}
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := r.Context().Value(connectionKey{}).(int64)
		mu.Lock()
		requestConnections = append(requestConnections, id)
		mu.Unlock()
		switch r.URL.Path {
		case "/version":
			if r.Header.Get("Authorization") != "" {
				t.Error("version request contained Authorization")
			}
			_ = json.NewEncoder(w).Encode(discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"})
		case "/v1/usage":
			if r.URL.RawQuery != "period=7d" {
				t.Errorf("usage query = %q", r.URL.RawQuery)
			}
			if r.Header.Get("Authorization") != "Bearer "+channelKeyCanary {
				t.Errorf("Authorization = %q", r.Header.Get("Authorization"))
			}
			_, _ = io.WriteString(w, `{"period":"7d","summary":{"requests":4,"prompt_tokens":8,"completion_tokens":2,"cost":0.5},"by_model":[]}`)
		default:
			http.NotFound(w, r)
		}
	}))
	server.Config.ConnContext = func(ctx context.Context, _ net.Conn) context.Context {
		return context.WithValue(ctx, connectionKey{}, nextID.Add(1))
	}
	server.Start()
	defer server.Close()

	listener := listenerForServer(t, server.URL)
	dials := atomic.Int64{}
	snapshot, err := (Channel{
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			dials.Add(1)
			return (&net.Dialer{}).DialContext(ctx, network, address)
		},
		ValidateProcess: genuineProcess,
	}).FetchUsage(context.Background(), listener, trustedFixture(listener), apikeyusage.Period7d, channelKeyCanary)
	if err != nil || snapshot.Summary.Requests != 4 || snapshot.Summary.Cost != 0.5 {
		t.Fatalf("FetchUsage() = %+v, %+v", snapshot, err)
	}
	mu.Lock()
	defer mu.Unlock()
	if dials.Load() != 1 || len(requestConnections) != 2 || requestConnections[0] != requestConnections[1] {
		t.Fatalf("dials=%d request connections=%v", dials.Load(), requestConnections)
	}
}

func TestChannelSimplifiesMixedCatalog(t *testing.T) {
	var modelAuthorizationObserved atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/version":
			if authorization := r.Header.Get("Authorization"); authorization != "" {
				t.Errorf("version Authorization = %q, want empty", authorization)
			}
			_ = json.NewEncoder(w).Encode(discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"})
		case "/v1/models":
			if authorization := r.Header.Get("Authorization"); authorization != "Bearer "+channelKeyCanary {
				t.Errorf("models Authorization = %q", authorization)
				http.Error(w, "unauthorized", http.StatusUnauthorized)
				return
			}
			modelAuthorizationObserved.Store(true)
			_, _ = io.WriteString(w, `{"data":[{"id":"provider/model"},{"id":"model-a"}]}`)
		default:
			http.NotFound(w, r)
		}
	}))
	defer server.Close()

	listener := listenerForServer(t, server.URL)
	var validationCalls atomic.Int64
	models, err := (Channel{Simplify: true, ValidateProcess: func(identity process.Identity, binaryPath string) (process.Status, error) {
		validationCalls.Add(1)
		return genuineProcess(identity, binaryPath)
	}}).Fetch(context.Background(), listener, trustedFixture(listener), channelKeyCanary)
	if err != nil || len(models) != 1 || models[0] != "model-a" {
		t.Fatalf("Fetch() = %q, %+v", models, err)
	}
	if !modelAuthorizationObserved.Load() {
		t.Fatal("models request did not contain the expected Authorization header")
	}
	if validationCalls.Load() != 1 {
		t.Fatalf("process validation calls = %d, want 1", validationCalls.Load())
	}
}

func TestChannelForcedRedialFailsBeforeKeyTransmission(t *testing.T) {
	var modelRequests atomic.Int64
	var keyObserved atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "" {
			keyObserved.Store(true)
		}
		if r.URL.Path == "/version" {
			w.Header().Set("Connection", "close")
			_ = json.NewEncoder(w).Encode(discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"})
			return
		}
		modelRequests.Add(1)
	}))
	defer server.Close()
	listener := listenerForServer(t, server.URL)
	_, err := (Channel{ValidateProcess: genuineProcess}).Fetch(context.Background(), listener, trustedFixture(listener), channelKeyCanary)
	if err == nil || err.Code != protocol.CodeModelDiscoveryFailed {
		t.Fatalf("Fetch() error = %+v", err)
	}
	if modelRequests.Load() != 0 || keyObserved.Load() {
		t.Fatalf("model requests=%d key observed=%t", modelRequests.Load(), keyObserved.Load())
	}
}

func TestChannelRejectsProcessSwapBeforeKeyTransmission(t *testing.T) {
	var keyObserved atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "" {
			keyObserved.Store(true)
		}
		_ = json.NewEncoder(w).Encode(discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"})
	}))
	defer server.Close()
	listener := listenerForServer(t, server.URL)
	_, err := (Channel{ValidateProcess: func(process.Identity, string) (process.Status, error) {
		return process.StatusStale, nil
	}}).Fetch(context.Background(), listener, trustedFixture(listener), channelKeyCanary)
	if err == nil || err.Code != protocol.CodeModelCatalogStale || keyObserved.Load() {
		t.Fatalf("error=%+v key observed=%t", err, keyObserved.Load())
	}
}

func TestChannelRejectsVersionIdentityMismatchBeforeKeyTransmission(t *testing.T) {
	var keyObserved atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "" {
			keyObserved.Store(true)
		}
		_ = json.NewEncoder(w).Encode(discovery.Version{PID: 92, DeploymentID: "other", ManagementProtocolVersion: "1"})
	}))
	defer server.Close()
	listener := listenerForServer(t, server.URL)
	_, err := (Channel{ValidateProcess: genuineProcess}).Fetch(context.Background(), listener, trustedFixture(listener), channelKeyCanary)
	if err == nil || err.Code != protocol.CodeModelCatalogStale || keyObserved.Load() {
		t.Fatalf("error=%+v key observed=%t", err, keyObserved.Load())
	}
}

func TestChannelRejectsNonOKVersionBeforeKeyTransmission(t *testing.T) {
	tests := []struct {
		name    string
		respond func(w http.ResponseWriter, r *http.Request)
		timeout bool
	}{
		{
			name: "redirect_response",
			respond: func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Location", "/other")
				w.WriteHeader(http.StatusFound)
			},
		},
		{
			name: "protocol_upgrade",
			respond: func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Connection", "Upgrade")
				w.Header().Set("Upgrade", "websocket")
				w.WriteHeader(http.StatusSwitchingProtocols)
			},
		},
		{
			name: "timeout_during_version",
			respond: func(_ http.ResponseWriter, r *http.Request) {
				<-r.Context().Done()
			},
			timeout: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var keyObserved atomic.Bool
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Header.Get("Authorization") != "" {
					keyObserved.Store(true)
				}
				tt.respond(w, r)
			}))
			defer server.Close()
			listener := listenerForServer(t, server.URL)
			ctx := context.Background()
			if tt.timeout {
				var cancel context.CancelFunc
				ctx, cancel = context.WithTimeout(ctx, 20*time.Millisecond)
				defer cancel()
			}
			_, err := (Channel{ValidateProcess: genuineProcess}).Fetch(ctx, listener, trustedFixture(listener), channelKeyCanary)
			if err == nil || err.Code != protocol.CodeModelDiscoveryFailed {
				t.Fatalf("Fetch() error = %+v, want MODEL_DISCOVERY_FAILED", err)
			}
			if keyObserved.Load() {
				t.Fatal("Authorization was sent before trust completed")
			}
		})
	}
}

func listenerForServer(t *testing.T, rawURL string) Listener {
	t.Helper()
	parsed, err := url.Parse(rawURL)
	if err != nil {
		t.Fatal(err)
	}
	listener, err := NormalizeListener(parsed.Host)
	if err != nil {
		t.Fatal(err)
	}
	return listener
}

func trustedFixture(listener Listener) discovery.Result {
	return discovery.Result{
		Classification: discovery.ExternalCompatible, Owner: "cli", ListenAddr: listener.RouterBaseURL,
		Version: discovery.Version{PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4"},
		State: state.RouterState{
			PID: 91, Owner: "cli", ListenAddr: listener.RouterBaseURL, BinaryPath: "/router",
			ProcessStartedAt: "start", ProcessExecutable: "/router", DeploymentID: "prod-a", ManagementProtocolVersion: "4",
		},
	}
}

func genuineProcess(process.Identity, string) (process.Status, error) {
	return process.StatusGenuine, nil
}
