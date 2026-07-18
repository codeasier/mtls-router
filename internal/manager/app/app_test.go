package app

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/agent"
	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/lifecycle"
	managerpaths "github.com/codeasier/mtls-router/internal/manager/paths"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
	"github.com/codeasier/mtls-router/internal/manager/trustedrouter"
)

const (
	integrationKey         = "sk-manager-integration-canary-9f3c"
	integrationURLUsername = "manager-url-username-canary-7a8b"
	integrationURLPassword = "manager-url-password-canary-9c0d"
)

type fakeLifecycle struct {
	start   func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error)
	reclaim func() (state.RouterState, *lifecycle.Error)
	stop    func(context.Context) *lifecycle.Error
	recent  string
	exits   chan lifecycle.UnexpectedExit
}

func (f *fakeLifecycle) Start(ctx context.Context, owner protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
	return f.start(ctx, owner)
}

func (f *fakeLifecycle) Reclaim() (state.RouterState, *lifecycle.Error) {
	return f.reclaim()
}

func (f *fakeLifecycle) Stop(ctx context.Context) *lifecycle.Error       { return f.stop(ctx) }
func (f *fakeLifecycle) MonitorParent(context.Context) *lifecycle.Error  { return nil }
func (f *fakeLifecycle) RecentOutput() string                            { return f.recent }
func (f *fakeLifecycle) UnexpectedExit() <-chan lifecycle.UnexpectedExit { return f.exits }

type fakeAgent struct {
	render          func(context.Context, []agent.Kind, string, json.RawMessage) (agent.RenderResult, error)
	preview         func(context.Context, []agent.Kind) (agent.Preview, error)
	validatePreview func(context.Context, agent.WriteRequest) error
	binding         func(context.Context, []agent.Kind, string, json.RawMessage) (agent.CatalogBinding, error)
	write           func(context.Context, agent.WriteRequest) (agent.WriteResult, error)
}

func (f *fakeAgent) Render(ctx context.Context, selected []agent.Kind, token string, config json.RawMessage) (agent.RenderResult, error) {
	if f.render == nil {
		return agent.RenderResult{}, errors.New("render unavailable")
	}
	return f.render(ctx, selected, token, config)
}

type fakeModelsService struct {
	discover func(context.Context, []agent.Kind, []string, modelconfig.CatalogClaims) (agent.ModelsResult, error)
}

func (f fakeModelsService) DiscoverModels(ctx context.Context, selected []agent.Kind, catalog []string, claims modelconfig.CatalogClaims) (agent.ModelsResult, error) {
	return f.discover(ctx, selected, catalog, claims)
}

type fakeTrustedRouter struct {
	fetch      func(context.Context, protocol.RouterOwner, string) (trustedrouter.Result, *protocol.Error)
	revalidate func(context.Context, protocol.RouterOwner, string, trustedrouter.Binding) ([]string, *protocol.Error)
}

func (f fakeTrustedRouter) Fetch(ctx context.Context, owner protocol.RouterOwner, key string) (trustedrouter.Result, *protocol.Error) {
	return f.fetch(ctx, owner, key)
}

func (f fakeTrustedRouter) Revalidate(ctx context.Context, owner protocol.RouterOwner, key string, binding trustedrouter.Binding) ([]string, *protocol.Error) {
	if f.revalidate == nil {
		return nil, nil
	}
	return f.revalidate(ctx, owner, key, binding)
}

func TestDesktopSessionEOFStopsOwnedRouter(t *testing.T) {
	stops := 0
	lifecycleManager := &fakeLifecycle{
		stop: func(context.Context) *lifecycle.Error {
			stops++
			return nil
		},
	}
	manager := newWithDependencies(Config{DesktopSession: "desktop-session"}, dependencies{lifecycle: lifecycleManager})
	if err := manager.Serve(context.Background(), strings.NewReader(""), io.Discard); err != nil {
		t.Fatal(err)
	}
	if stops != 1 {
		t.Fatalf("router stops = %d, want 1", stops)
	}
}

func (f *fakeAgent) Preview(ctx context.Context, selected []agent.Kind) (agent.Preview, error) {
	return f.preview(ctx, selected)
}

func (f *fakeAgent) Write(ctx context.Context, request agent.WriteRequest) (agent.WriteResult, error) {
	return f.write(ctx, request)
}

func (f *fakeAgent) ValidatePreview(ctx context.Context, request agent.WriteRequest) error {
	if f.validatePreview == nil {
		return nil
	}
	return f.validatePreview(ctx, request)
}

func (f *fakeAgent) CatalogBinding(ctx context.Context, selected []agent.Kind, token string, config json.RawMessage) (agent.CatalogBinding, error) {
	if f.binding == nil {
		return agent.CatalogBinding{}, errors.New("binding unavailable")
	}
	return f.binding(ctx, selected, token, config)
}

func TestServeWiresEveryMethodSequentiallyAndSanitizesOutput(t *testing.T) {
	dir := t.TempDir()
	logPath := filepath.Join(dir, "router.log")
	rawLog := "ok line\napi_key=" + integrationKey + "\nAuthorization: Bearer bearer-header-canary\nGET https://example.test/v1?token=" + integrationKey + "\nGET https://" + integrationURLUsername + ":" + integrationURLPassword + "@example.test/v1\n-----BEGIN PRIVATE KEY-----\nprivate-canary\n-----END PRIVATE KEY-----\n"
	if err := os.WriteFile(logPath, []byte(rawLog), 0o600); err != nil {
		t.Fatal(err)
	}
	found := discovery.Result{
		Classification: discovery.DesktopOwned,
		Owner:          "desktop",
		ListenAddr:     "http://127.0.0.1:19099",
		Version: discovery.Version{
			Version: "router-v1", PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "2",
		},
		Health: discovery.Health{Status: "ok"},
		State: state.RouterState{
			PID: 91, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099", LogPath: logPath,
			RouterVersion: "router-v1", DeploymentID: "prod-a", ManagementProtocolVersion: "2",
		},
	}
	lifecycleManager := &fakeLifecycle{
		start: func(_ context.Context, owner protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			return state.RouterState{PID: 91, Owner: string(owner), ListenAddr: found.ListenAddr}, nil
		},
		stop: func(context.Context) *lifecycle.Error { return nil },
	}
	agentManager := &fakeAgent{
		preview: func(_ context.Context, selected []agent.Kind) (agent.Preview, error) {
			return agent.Preview{RevisionToken: "revision", Agents: []agent.AgentPreview{{Agent: selected[0]}}}, nil
		},
		write: func(_ context.Context, request agent.WriteRequest) (agent.WriteResult, error) {
			if request.APIKey != integrationKey {
				t.Fatalf("write key = %q", request.APIKey)
			}
			return agent.WriteResult{TransactionID: "transaction", Agents: []agent.AgentWriteStatus{{Agent: request.Agents[0], Success: true}}}, nil
		},
	}
	manager := newWithDependencies(Config{RouterPath: os.Args[0], Paths: managerpaths.Paths{DesktopLogFile: logPath}}, dependencies{
		info: func() protocol.ManagerInfoResult {
			return protocol.ManagerInfoResult{Version: "manager-v1", Commit: "abc123", BuildDate: "2026-07-12T00:00:00Z", Target: "test/test", DeploymentID: "prod-a", ManagementProtocolVersion: "2"}
		},
		discover:  func(context.Context) discovery.Result { return found },
		lifecycle: lifecycleManager,
		detect: func() ([]agent.State, error) {
			return []agent.State{{Agent: agent.ClaudeCode, Name: "Claude Code", Detected: true, Path: filepath.Join(dir, "settings.json"), Format: agent.FormatJSON, Writable: true}}, nil
		},
		agent: agentManager,
		now:   func() time.Time { return time.Date(2026, 7, 12, 1, 2, 3, 0, time.UTC) },
	})

	requests := []string{
		`{"id":"1","method":"manager.info"}`,
		`{"id":"2","method":"diagnostics.collect"}`,
		`{"id":"3","method":"router.status"}`,
		`{"id":"4","method":"router.start","params":{"owner":"desktop"}}`,
		`{"id":"5","method":"router.stop"}`,
		`{"id":"6","method":"router.health"}`,
		`{"id":"7","method":"router.version"}`,
		`{"id":"8","method":"router.logs","params":{"limit":20}}`,
		`{"id":"9","method":"agent.detect"}`,
	}
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(output.String(), integrationKey) || strings.Contains(output.String(), integrationURLUsername) || strings.Contains(output.String(), integrationURLPassword) || strings.Contains(output.String(), "bearer-header-canary") || strings.Contains(output.String(), "private-canary") {
		t.Fatalf("protocol output contains sensitive input: %s", output.String())
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != len(requests) {
		t.Fatalf("response lines = %d, want %d: %s", len(lines), len(requests), output.String())
	}
	wantIDs := []string{"1", "2", "3", "4", "5", "6", "7", "8", "9"}
	for index, line := range lines {
		var response protocol.Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatalf("response %d: %v", index, err)
		}
		wantID := wantIDs[index]
		if response.ID == nil || *response.ID != wantID || response.Error != nil {
			t.Fatalf("response %d = %#v, want ID %q", index, response, wantID)
		}
	}
	if !strings.Contains(output.String(), "[REDACTED") || !strings.Contains(output.String(), `"commit":"abc123"`) {
		t.Fatalf("output lacks sanitized logs or version metadata: %s", output.String())
	}
}

func TestSanitizeTextRedactsURLUserinfoBeforeQueryValues(t *testing.T) {
	for _, tc := range []struct {
		name  string
		input string
		want  string
	}{
		{
			name:  "credentials without query",
			input: "http://alice:secret@example.test:8080/v1",
			want:  "http://[REDACTED]@example.test:8080/v1",
		},
		{
			name:  "credentials with query",
			input: "https://alice:secret@example.test:8443/v1?token=query-secret",
			want:  "https://[REDACTED]@example.test:8443/v1?[REDACTED]",
		},
		{
			name:  "username only",
			input: "http://alice@example.test/v1",
			want:  "http://[REDACTED]@example.test/v1",
		},
		{
			name:  "percent encoded userinfo",
			input: "https://alice%40example:secret%2Fvalue@example.test/v1",
			want:  "https://[REDACTED]@example.test/v1",
		},
		{
			name:  "uppercase scheme",
			input: "HtTpS://alice:secret@example.test:9443/v1?token=query-secret",
			want:  "HtTpS://[REDACTED]@example.test:9443/v1?[REDACTED]",
		},
		{
			name:  "at sign in path",
			input: "https://example.test/users/alice@example.test",
			want:  "https://example.test/users/alice@example.test",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := sanitizeText(tc.input); got != tc.want {
				t.Fatalf("sanitizeText(%q) = %q, want %q", tc.input, got, tc.want)
			}
		})
	}
}

func TestHandlersRejectInvalidTypedParameters(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{
		info:     func() protocol.ManagerInfoResult { return protocol.ManagerInfoResult{} },
		discover: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle: &fakeLifecycle{start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			return state.RouterState{}, nil
		}, stop: func(context.Context) *lifecycle.Error { return nil }},
		detect: func() ([]agent.State, error) { return nil, nil },
		agent: &fakeAgent{
			preview: func(context.Context, []agent.Kind) (agent.Preview, error) { return agent.Preview{}, nil },
			write:   func(context.Context, agent.WriteRequest) (agent.WriteResult, error) { return agent.WriteResult{}, nil },
		},
		now: time.Now,
	})
	requests := []string{
		`{"id":"1","method":"manager.info","params":{"extra":true}}`,
		`{"id":"2","method":"router.start","params":{"owner":"other"}}`,
		`{"id":"3","method":"router.logs","params":{"limit":1001}}`,
		`{"id":"4","method":"agent.preview","params":{"agents":[]}}`,
		`{"id":"5","method":"agent.write","params":{"agents":["claude","claude"],"catalog_token":"c","model_config":{},"revision_token":"r","approve_managed_overwrite":false,"approve_codex_auth_change":false,"api_key":"secret"}}`,
	}
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(strings.TrimSpace(output.String()), "\n") {
		var response protocol.Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatal(err)
		}
		if response.Error == nil || response.Error.Code != protocol.CodeInvalidParams {
			t.Fatalf("response = %#v", response)
		}
	}
	if strings.Contains(output.String(), "secret") {
		t.Fatalf("invalid parameter response exposed key: %s", output.String())
	}
}

func TestV2AgentHandlersEnforceContractsWithoutLegacyWrites(t *testing.T) {
	legacyCalls := 0
	manager := newWithDependencies(Config{}, dependencies{agent: &fakeAgent{
		render: func(context.Context, []agent.Kind, string, json.RawMessage) (agent.RenderResult, error) {
			return agent.RenderResult{}, &agent.OperationError{Code: agent.CodeModelCatalogStale}
		},
		preview: func(context.Context, []agent.Kind) (agent.Preview, error) { legacyCalls++; return agent.Preview{}, nil },
		write: func(context.Context, agent.WriteRequest) (agent.WriteResult, error) {
			legacyCalls++
			return agent.WriteResult{}, nil
		},
	}})
	requests := []string{
		`{"id":"models","method":"agent.models","params":{"owner":"cli","agents":["claude"],"api_key":"secret"}}`,
		`{"id":"render","method":"agent.render","params":{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1,"claude":{}}}}`,
		`{"id":"preview-old","method":"agent.preview","params":{"agents":["claude"]}}`,
		`{"id":"write-old","method":"agent.write","params":{"agents":["claude"],"revision_token":"revision","api_key":"secret"}}`,
		`{"id":"write-missing-approval","method":"agent.write","params":{"agents":["claude"],"catalog_token":"catalog","model_config":{},"revision_token":"revision","api_key":"secret"}}`,
	}
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	if legacyCalls != 0 {
		t.Fatalf("legacy agent service calls = %d, want 0", legacyCalls)
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if !strings.Contains(lines[0], `"code":"MODEL_DISCOVERY_FAILED"`) || !strings.Contains(lines[1], `"code":"MODEL_CATALOG_STALE"`) {
		t.Fatalf("scaffold responses = %s", output.String())
	}
	for _, line := range lines[2:] {
		if !strings.Contains(line, `"code":"INVALID_PARAMS"`) {
			t.Fatalf("old v1 shape accepted: %s", line)
		}
	}
	if strings.Contains(output.String(), "secret") {
		t.Fatalf("protocol output exposed key: %s", output.String())
	}
}

func TestAgentModelsReturnsCompleteKeyFreeCatalogResult(t *testing.T) {
	const key = "agent-models-protocol-secret-canary"
	binding := trustedrouter.Binding{
		RouterBaseURL: "http://[::1]:19443", APIBaseURL: "http://[::1]:19443/v1",
		DeploymentID: "prod-a", ProtocolVersion: "2",
	}
	manager := newWithDependencies(Config{}, dependencies{
		trusted: fakeTrustedRouter{fetch: func(_ context.Context, owner protocol.RouterOwner, gotKey string) (trustedrouter.Result, *protocol.Error) {
			if owner != protocol.RouterOwnerCLI || gotKey != key {
				t.Fatalf("owner=%q key=%q", owner, gotKey)
			}
			return trustedrouter.Result{Models: []string{"model-a", "model-b"}, Binding: binding}, nil
		}},
		models: fakeModelsService{discover: func(_ context.Context, selected []agent.Kind, catalog []string, claims modelconfig.CatalogClaims) (agent.ModelsResult, error) {
			if strings.Join([]string{string(selected[0]), string(selected[1])}, ",") != "claude,codex" || strings.Join(catalog, ",") != "model-a,model-b" {
				t.Fatalf("selected=%v catalog=%v", selected, catalog)
			}
			if claims.Owner != "cli" || claims.RouterBaseURL != binding.RouterBaseURL || claims.DeploymentID != binding.DeploymentID || claims.ProtocolVersion != "2" {
				t.Fatalf("claims=%+v", claims)
			}
			return agent.ModelsResult{CatalogToken: "signed-catalog", Existing: agent.ModelsExisting{
				ModelConfig:       json.RawMessage(`{"version":1,"codex":{"model":"model-a"}}`),
				UnavailableModels: map[string][]string{"claude": {"old-model"}}, DriftedAgents: []string{"claude", "codex"},
			}}, nil
		}},
	})
	var output bytes.Buffer
	request := `{"id":"models","method":"agent.models","params":{"owner":"cli","agents":["claude","codex"],"api_key":"` + key + `"}}` + "\n"
	if err := manager.Serve(context.Background(), strings.NewReader(request), &output); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(output.String(), key) {
		t.Fatalf("protocol result leaked key: %s", output.String())
	}
	var response protocol.Response
	if err := json.Unmarshal(bytes.TrimSpace(output.Bytes()), &response); err != nil {
		t.Fatal(err)
	}
	if response.Error != nil {
		t.Fatalf("response error = %+v", response.Error)
	}
	var result protocol.AgentModelsResult
	if err := json.Unmarshal(response.Result, &result); err != nil {
		t.Fatal(err)
	}
	if strings.Join(result.Models, ",") != "model-a,model-b" || result.CatalogToken != "signed-catalog" ||
		result.RouterBaseURL != binding.RouterBaseURL || result.APIBaseURL != binding.APIBaseURL ||
		strings.Join(result.Existing.UnavailableModels["claude"], ",") != "old-model" ||
		strings.Join(result.Existing.DriftedAgents, ",") != "claude,codex" {
		t.Fatalf("result = %+v", result)
	}
}

func TestAgentRenderReturnsCanonicalRedactedFragments(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{agent: &fakeAgent{render: func(_ context.Context, selected []agent.Kind, token string, config json.RawMessage) (agent.RenderResult, error) {
		if len(selected) != 1 || selected[0] != agent.ClaudeCode || token != "signed-catalog" || !strings.Contains(string(config), "dynamic-model") {
			t.Fatalf("render input selected=%v token=%q config=%s", selected, token, config)
		}
		return agent.RenderResult{ModelConfig: json.RawMessage(`{"claude":{},"version":1}`), Fragments: []agent.Fragment{{Agent: agent.ClaudeCode, Role: "config", Path: "/safe/settings.json", Format: agent.FormatJSON, Content: `{"token":"<redacted-api-key>"}`}}}, nil
	}}})
	request := `{"id":"render","method":"agent.render","params":{"agents":["claude"],"catalog_token":"signed-catalog","model_config":{"version":1,"claude":{"primary":{"model":"dynamic-model"}}}}}` + "\n"
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(request), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), `"fragments"`) || !strings.Contains(output.String(), `redacted-api-key`) || strings.Contains(output.String(), `MODEL_CATALOG_STALE`) {
		t.Fatalf("render response = %s", output.String())
	}
}

func TestAgentV1AndMixedRequestsAreRejectedBeforeHandlers(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{agent: &fakeAgent{}})
	requests := []string{
		`{"id":"v1-preview","method":"agent.preview","params":{"agents":["claude"]}}`,
		`{"id":"mixed-preview","method":"agent.preview","params":{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1},"api_key":"legacy"}}`,
		`{"id":"v1-write","method":"agent.write","params":{"agents":["claude"],"revision_token":"revision","api_key":"legacy"}}`,
		`{"id":"mixed-write","method":"agent.write","params":{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1},"revision_token":"revision","approve_managed_overwrite":false,"approve_codex_auth_change":false,"api_key":"legacy","config":{"claude":{}}}}`,
	}
	for _, request := range requests {
		var output bytes.Buffer
		if err := manager.Serve(context.Background(), strings.NewReader(request+"\n"), &output); err != nil {
			t.Fatal(err)
		}
		var response protocol.Response
		if err := json.Unmarshal(bytes.TrimSpace(output.Bytes()), &response); err != nil {
			t.Fatal(err)
		}
		if response.Error == nil || response.Error.Code != protocol.CodeInvalidParams || response.Result != nil {
			t.Fatalf("request %s response = %#v", request, response)
		}
		if strings.Contains(output.String(), "legacy") {
			t.Fatalf("rejected request value leaked: %s", output.String())
		}
	}
}

func TestAgentWritePreflightOrderPrecedesWriteArtifacts(t *testing.T) {
	var calls []string
	config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-a"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	agentManager := &fakeAgent{
		validatePreview: func(context.Context, agent.WriteRequest) error { calls = append(calls, "preview"); return nil },
		binding: func(context.Context, []agent.Kind, string, json.RawMessage) (agent.CatalogBinding, error) {
			calls = append(calls, "router-binding")
			return agent.CatalogBinding{Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2", Models: []string{"model-a"}}, nil
		},
		write: func(context.Context, agent.WriteRequest) (agent.WriteResult, error) {
			calls = append(calls, "artifacts")
			return agent.WriteResult{TransactionID: "transaction"}, nil
		},
	}
	manager := newWithDependencies(Config{}, dependencies{
		agent: agentManager,
		trusted: fakeTrustedRouter{revalidate: func(_ context.Context, owner protocol.RouterOwner, key string, binding trustedrouter.Binding) ([]string, *protocol.Error) {
			calls = append(calls, "catalog-refresh")
			if owner != protocol.RouterOwnerCLI || key != integrationKey || binding.RouterBaseURL != "http://127.0.0.1:19099" {
				t.Fatalf("refresh request = owner=%q key=%q binding=%+v", owner, key, binding)
			}
			return []string{"model-a", "unrelated-addition"}, nil
		}},
	})
	params := writeParams(config, true, true)
	if _, err := manager.agentWrite(context.Background(), params); err != nil {
		t.Fatalf("agentWrite() error = %+v", err)
	}
	want := []string{"preview", "router-binding", "catalog-refresh", "artifacts"}
	if fmt.Sprint(calls) != fmt.Sprint(want) {
		t.Fatalf("call order = %v, want %v", calls, want)
	}
}

func TestAgentWriteEveryPreflightFailureCreatesZeroArtifacts(t *testing.T) {
	config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-a"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	for _, test := range []struct {
		name            string
		params          json.RawMessage
		previewErr      error
		bindingErr      error
		refreshErr      *protocol.Error
		refreshedModels []string
		want            protocol.ErrorCode
	}{
		{name: "invalid config", params: writeParams(json.RawMessage(`{"version":`), true, true), want: protocol.CodeInvalidParams},
		{name: "missing approval", params: writeParams(config, false, true), want: protocol.CodeInvalidParams},
		{name: "stale preview", params: writeParams(config, true, true), previewErr: &agent.OperationError{Code: agent.CodePreviewStale}, want: protocol.CodePreviewStale},
		{name: "stale router binding", params: writeParams(config, true, true), bindingErr: &agent.OperationError{Code: agent.CodeModelCatalogStale}, want: protocol.CodeModelCatalogStale},
		{name: "authentication", params: writeParams(config, true, true), refreshErr: &protocol.Error{Code: protocol.CodeModelAuthFailed, Message: "authentication failed"}, want: protocol.CodeModelAuthFailed},
		{name: "transport", params: writeParams(config, true, true), refreshErr: &protocol.Error{Code: protocol.CodeModelDiscoveryFailed, Message: "discovery failed"}, want: protocol.CodeModelDiscoveryFailed},
		{name: "invalid catalog", params: writeParams(config, true, true), refreshErr: &protocol.Error{Code: protocol.CodeModelResponseInvalid, Message: "invalid catalog"}, want: protocol.CodeModelResponseInvalid},
		{name: "removed selected model", params: writeParams(config, true, true), refreshedModels: []string{"other-model"}, want: protocol.CodeModelNotAvailable},
	} {
		t.Run(test.name, func(t *testing.T) {
			home := t.TempDir()
			sentinel := filepath.Join(home, "existing-agent.json")
			if err := os.WriteFile(sentinel, []byte(`{"unchanged":true}`), 0o600); err != nil {
				t.Fatal(err)
			}
			writeCalls := 0
			agentManager := &fakeAgent{
				validatePreview: func(context.Context, agent.WriteRequest) error { return test.previewErr },
				binding: func(context.Context, []agent.Kind, string, json.RawMessage) (agent.CatalogBinding, error) {
					if test.bindingErr != nil {
						return agent.CatalogBinding{}, test.bindingErr
					}
					return agent.CatalogBinding{Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2"}, nil
				},
				write: func(context.Context, agent.WriteRequest) (agent.WriteResult, error) {
					writeCalls++
					return agent.WriteResult{}, nil
				},
			}
			manager := newWithDependencies(Config{}, dependencies{
				agent: agentManager,
				trusted: fakeTrustedRouter{revalidate: func(context.Context, protocol.RouterOwner, string, trustedrouter.Binding) ([]string, *protocol.Error) {
					if test.refreshErr != nil {
						return nil, test.refreshErr
					}
					if test.refreshedModels != nil {
						return test.refreshedModels, nil
					}
					return []string{"model-a"}, nil
				}},
			})
			_, gotErr := manager.agentWrite(context.Background(), test.params)
			if gotErr == nil || gotErr.Code != test.want {
				t.Fatalf("error = %+v, want %q", gotErr, test.want)
			}
			if writeCalls != 0 {
				t.Fatalf("write artifact phase called %d times", writeCalls)
			}
			entries, err := os.ReadDir(home)
			if err != nil {
				t.Fatal(err)
			}
			if len(entries) != 1 || entries[0].Name() != filepath.Base(sentinel) {
				t.Fatalf("preflight created files/state/backups/journal/temp: %#v", entries)
			}
			content, err := os.ReadFile(sentinel)
			if err != nil || string(content) != `{"unchanged":true}` {
				t.Fatalf("existing file changed: content=%q err=%v", content, err)
			}
		})
	}
}

func writeParams(config json.RawMessage, includeApprovals, approvals bool) json.RawMessage {
	request := map[string]any{
		"agents": []string{"claude"}, "catalog_token": "catalog", "model_config": config,
		"revision_token": "revision", "api_key": integrationKey,
	}
	if includeApprovals {
		request["approve_managed_overwrite"] = approvals
		request["approve_codex_auth_change"] = approvals
	}
	raw, _ := json.Marshal(request)
	return raw
}

func TestDeadlineWaitsForLifecycleCleanup(t *testing.T) {
	cleaned := false
	lifecycleManager := &fakeLifecycle{
		start: func(ctx context.Context, _ protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			<-ctx.Done()
			cleaned = true
			return state.RouterState{}, &lifecycle.Error{Code: protocol.CodeOperationTimeout, Err: errors.New("sensitive cleanup detail")}
		},
		stop: func(context.Context) *lifecycle.Error { return nil },
	}
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{lifecycle: lifecycleManager})
	manager.server.Deadlines[protocol.MethodRouterStart] = 10 * time.Millisecond
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(`{"id":"timeout","method":"router.start","params":{"owner":"desktop"}}`+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	if !cleaned || !strings.Contains(output.String(), `"code":"OPERATION_TIMEOUT"`) || strings.Contains(output.String(), "sensitive") {
		t.Fatalf("cleaned=%t output=%s", cleaned, output.String())
	}
}

func TestDesktopRouterStartReclaimsAfterOwnedOrStaleNormalStart(t *testing.T) {
	for _, startCode := range []protocol.ErrorCode{protocol.CodeRouterAlreadyRunning, protocol.CodeRouterStateStale} {
		t.Run(string(startCode), func(t *testing.T) {
			startCalls := 0
			reclaimCalls := 0
			found := discovery.Result{Classification: discovery.Stale}
			lifecycleManager := &fakeLifecycle{
				start: func(_ context.Context, owner protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
					startCalls++
					if owner != protocol.RouterOwnerDesktop {
						t.Fatalf("owner = %q", owner)
					}
					return state.RouterState{}, &lifecycle.Error{Code: startCode, Err: errors.New("normal start rejected")}
				},
				reclaim: func() (state.RouterState, *lifecycle.Error) {
					reclaimCalls++
					value := state.RouterState{PID: 91, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099"}
					found = discovery.Result{Classification: discovery.DesktopOwned, Owner: "desktop", ListenAddr: value.ListenAddr, State: value}
					return value, nil
				},
				stop: func(context.Context) *lifecycle.Error { return nil },
			}
			manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
				discover:  func(context.Context) discovery.Result { return found },
				lifecycle: lifecycleManager,
			})
			input := strings.NewReader("{\"id\":\"start\",\"method\":\"router.start\",\"params\":{\"owner\":\"desktop\"}}\n{\"id\":\"status\",\"method\":\"router.status\"}\n")
			var output bytes.Buffer
			if err := manager.Serve(context.Background(), input, &output); err != nil {
				t.Fatal(err)
			}
			if startCalls != 1 || reclaimCalls != 1 {
				t.Fatalf("start calls = %d, reclaim calls = %d", startCalls, reclaimCalls)
			}
			lines := strings.Split(strings.TrimSpace(output.String()), "\n")
			if len(lines) != 2 {
				t.Fatalf("responses = %q", output.String())
			}
			for _, line := range lines {
				var response protocol.Response
				if err := json.Unmarshal([]byte(line), &response); err != nil {
					t.Fatal(err)
				}
				if response.Error != nil || !strings.Contains(string(response.Result), `"state":"desktop_owned"`) {
					t.Fatalf("response = %s", line)
				}
			}
		})
	}
}

func TestRouterStartReclaimsThroughProtocolAndLifecycleThenStatusSucceeds(t *testing.T) {
	dir := t.TempDir()
	managerIdentity := process.Identity{PID: 72, StartedAt: "manager-new", Executable: "/manager"}
	value := state.RouterState{
		PID: 91, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099", BinaryPath: "/router", LogPath: filepath.Join(dir, "router.log"),
		ProcessStartedAt: "router-start", ProcessExecutable: "/router", DesktopSessionID: "session", ManagerPID: 71,
		ManagerProcessStartedAt: "manager-old", ManagerProcessExecutable: "/manager", ManagerVersion: "v1", RouterVersion: "v1",
		DeploymentID: "prod-a", ManagementProtocolVersion: "1",
	}
	lockAcquires := 0
	writes := 0
	signals := 0
	lifecycleManager := lifecycle.New(lifecycle.Config{
		RouterPath: os.Args[0], ListenAddr: "127.0.0.1:19099", DesktopStatePath: filepath.Join(dir, "desktop.json"),
		DesktopLockPath: filepath.Join(dir, "desktop.lock"), DesktopLogPath: value.LogPath, SessionID: "session",
		ManagerIdentity: managerIdentity, ParentIdentity: process.Identity{PID: 73, StartedAt: "desktop-start", Executable: "/desktop"},
		ManagerVersion: "v1", DeploymentID: "prod-a", ManagementProtocolVersion: "1",
	}, lifecycle.Dependencies{
		ReadState: func(string) (state.RouterState, error) { return value, nil },
		WriteState: func(_ string, updated state.RouterState) error {
			writes++
			value = updated
			return nil
		},
		AcquireLock: func(string) (io.Closer, error) {
			lockAcquires++
			return closerFunc(func() error { return nil }), nil
		},
		Validate: func(identity process.Identity, _ string) (process.Status, error) {
			if identity.PID == 71 {
				return process.StatusAbsent, nil
			}
			return process.StatusGenuine, nil
		},
		Signal: func(process.Identity, string, os.Signal) error {
			signals++
			return nil
		},
	})
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
		discover: func(context.Context) discovery.Result {
			classification := discovery.Stale
			if value.ManagerPID == managerIdentity.PID {
				classification = discovery.DesktopOwned
			}
			return discovery.Result{Classification: classification, Owner: "desktop", ListenAddr: value.ListenAddr, State: value}
		},
		lifecycle: lifecycleManager,
	})
	input := strings.NewReader("{\"id\":\"start\",\"method\":\"router.start\",\"params\":{\"owner\":\"desktop\"}}\n{\"id\":\"status\",\"method\":\"router.status\"}\n")
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	if lockAcquires != 2 || writes != 1 || signals != 0 {
		t.Fatalf("lock acquires=%d writes=%d signals=%d", lockAcquires, writes, signals)
	}
	if value.ManagerPID != managerIdentity.PID || value.ManagerProcessStartedAt != managerIdentity.StartedAt || value.ManagerProcessExecutable != managerIdentity.Executable {
		t.Fatalf("manager identity was not atomically replaced: %+v", value)
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("responses = %q", output.String())
	}
	for _, line := range lines {
		if !strings.Contains(line, `"state":"desktop_owned"`) || strings.Contains(line, `"error"`) {
			t.Fatalf("response = %s", line)
		}
	}
}

func TestRouterStartReclaimFailureFailsClosed(t *testing.T) {
	for _, test := range []struct {
		name      string
		owner     protocol.RouterOwner
		startCode protocol.ErrorCode
		wantCode  protocol.ErrorCode
		wantCalls int
	}{
		{name: "desktop reclaim rejected", owner: protocol.RouterOwnerDesktop, startCode: protocol.CodeRouterStateStale, wantCode: protocol.CodeRouterAlreadyRunning, wantCalls: 1},
		{name: "desktop unrelated start failure", owner: protocol.RouterOwnerDesktop, startCode: protocol.CodeRouterStartFailed, wantCode: protocol.CodeRouterStartFailed},
		{name: "cli never reclaims", owner: protocol.RouterOwnerCLI, startCode: protocol.CodeRouterAlreadyRunning, wantCode: protocol.CodeRouterAlreadyRunning},
	} {
		t.Run(test.name, func(t *testing.T) {
			startCalls := 0
			reclaimCalls := 0
			lifecycleManager := &fakeLifecycle{
				start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
					startCalls++
					return state.RouterState{}, &lifecycle.Error{Code: test.startCode, Err: errors.New("start failed")}
				},
				reclaim: func() (state.RouterState, *lifecycle.Error) {
					reclaimCalls++
					return state.RouterState{}, &lifecycle.Error{Code: test.wantCode, Err: errors.New("reclaim rejected")}
				},
				stop: func(context.Context) *lifecycle.Error { return nil },
			}
			manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{lifecycle: lifecycleManager})
			request := fmt.Sprintf("{\"id\":\"start\",\"method\":\"router.start\",\"params\":{\"owner\":%q}}\n", test.owner)
			var output bytes.Buffer
			if err := manager.Serve(context.Background(), strings.NewReader(request), &output); err != nil {
				t.Fatal(err)
			}
			if startCalls != 1 || reclaimCalls != test.wantCalls {
				t.Fatalf("start calls = %d, reclaim calls = %d", startCalls, reclaimCalls)
			}
			if !strings.Contains(output.String(), `"code":"`+string(test.wantCode)+`"`) {
				t.Fatalf("response = %s", output.String())
			}
		})
	}
}

func TestUnexpectedDesktopExitIsSanitizedLatchedAndClearedBySuccessfulRestart(t *testing.T) {
	exits := make(chan lifecycle.UnexpectedExit, 2)
	failedIdentity := process.Identity{PID: 91, StartedAt: "failed-start", Executable: "/router"}
	restartedIdentity := process.Identity{PID: 92, StartedAt: "restart", Executable: "/router"}
	found := discovery.Result{Classification: discovery.Absent}
	startCalls := 0
	statusCalls := 0
	lifecycleManager := &fakeLifecycle{
		exits: exits,
		start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			startCalls++
			if startCalls == 1 {
				return state.RouterState{}, &lifecycle.Error{Code: protocol.CodeRouterStartFailed, Err: errors.New("retry failed")}
			}
			value := state.RouterState{
				PID: restartedIdentity.PID, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099",
				ProcessStartedAt: restartedIdentity.StartedAt, ProcessExecutable: restartedIdentity.Executable,
			}
			found = discovery.Result{Classification: discovery.DesktopOwned, Owner: "desktop", State: value}
			return value, nil
		},
		stop: func(context.Context) *lifecycle.Error { return nil },
	}
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
		discover: func(context.Context) discovery.Result {
			statusCalls++
			if statusCalls == 2 {
				return discovery.Result{Classification: discovery.Stale}
			}
			return found
		},
		lifecycle: lifecycleManager,
	})
	recentOutput := make([]string, defaultLogLines+5)
	for index := range recentOutput {
		recentOutput[index] = fmt.Sprintf("operational line %d", index)
	}
	recentOutput = append(recentOutput,
		"Authorization: Bearer bearer-exit-canary",
		"api_key="+integrationKey,
		"GET https://example.test/v1?token="+integrationKey,
		"-----BEGIN PRIVATE KEY-----",
		"private-exit-canary",
		"-----END PRIVATE KEY-----",
		"safe ending",
	)
	exits <- lifecycle.UnexpectedExit{
		Identity:     failedIdentity,
		Err:          errors.New("exit status 1: " + integrationKey),
		RecentOutput: strings.Join(recentOutput, "\n"),
	}

	input := strings.NewReader(strings.Join([]string{
		`{"id":"failed-absent","method":"router.status"}`,
		`{"id":"retry","method":"router.start","params":{"owner":"desktop"}}`,
		`{"id":"failed-stale","method":"router.status"}`,
		`{"id":"restart","method":"router.start","params":{"owner":"desktop"}}`,
		`{"id":"cleared","method":"router.status"}`,
	}, "\n") + "\n")
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(output.String(), integrationKey) || strings.Contains(output.String(), "bearer-exit-canary") || strings.Contains(output.String(), "private-exit-canary") {
		t.Fatalf("unexpected-exit status exposed sensitive output: %s", output.String())
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 5 {
		t.Fatalf("responses = %q", output.String())
	}
	assertStatusResponse(t, lines[0], "start_failed", true)
	if !strings.Contains(lines[0], `"last_error":"desktop-owned router exited unexpectedly"`) || !strings.Contains(lines[0], `"recent_logs"`) || !strings.Contains(lines[0], "safe ending") || !strings.Contains(lines[0], "[REDACTED") {
		t.Fatalf("failed status lacks bounded sanitized diagnostics: %s", lines[0])
	}
	if !strings.Contains(lines[1], `"code":"ROUTER_START_FAILED"`) {
		t.Fatalf("failed retry response = %s", lines[1])
	}
	assertStatusResponse(t, lines[2], "start_failed", true)
	assertStatusResponse(t, lines[3], "desktop_owned", false)
	assertStatusResponse(t, lines[4], "desktop_owned", false)

	// A delayed notification from the previous identity cannot overwrite the
	// successful restart state.
	found = discovery.Result{Classification: discovery.Absent}
	exits <- lifecycle.UnexpectedExit{Identity: failedIdentity, RecentOutput: "old failure"}
	var delayed bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(`{"id":"delayed","method":"router.status"}`+"\n"), &delayed); err != nil {
		t.Fatal(err)
	}
	assertStatusResponse(t, strings.TrimSpace(delayed.String()), "absent", false)
}

func assertStatusResponse(t *testing.T, line, wantState string, wantLogs bool) {
	t.Helper()
	var response protocol.Response
	if err := json.Unmarshal([]byte(line), &response); err != nil {
		t.Fatal(err)
	}
	if response.Error != nil {
		t.Fatalf("response error = %+v", response.Error)
	}
	var status protocol.RouterStatusResult
	if err := json.Unmarshal(response.Result, &status); err != nil {
		t.Fatal(err)
	}
	if status.State != wantState || (len(status.RecentLogs) > 0) != wantLogs {
		t.Fatalf("status = %+v, want state=%q logs=%t", status, wantState, wantLogs)
	}
	if len(status.RecentLogs) > defaultLogLines {
		t.Fatalf("recent logs = %d, want at most %d", len(status.RecentLogs), defaultLogLines)
	}
	for _, line := range status.RecentLogs {
		if len(line) > maxLogLineBytes+len("[truncated]") {
			t.Fatalf("recent log line is not bounded: %d bytes", len(line))
		}
	}
}

type closerFunc func() error

func (f closerFunc) Close() error { return f() }
