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
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/agent"
	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/lifecycle"
	"github.com/codeasier/mtls-router/internal/manager/occupant"
	managerpaths "github.com/codeasier/mtls-router/internal/manager/paths"
	"github.com/codeasier/mtls-router/internal/manager/preset"
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
	previewRequest  func(context.Context, agent.PreviewRequest) (agent.Preview, error)
	cleanupPreview  func(context.Context, agent.CleanupPreviewRequest) (agent.CleanupPreview, error)
	cleanupWrite    func(context.Context, agent.CleanupWriteRequest) (agent.WriteResult, error)
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

type fakeOccupant struct {
	inspect        func(context.Context) (occupant.Inspection, error)
	forceTerminate func(context.Context, string) (occupant.Result, error)
}

func (f *fakeOccupant) Inspect(ctx context.Context) (occupant.Inspection, error) {
	return f.inspect(ctx)
}

func (f *fakeOccupant) ForceTerminate(ctx context.Context, token string) (occupant.Result, error) {
	return f.forceTerminate(ctx, token)
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

func (f *fakeAgent) PreviewRequest(ctx context.Context, request agent.PreviewRequest) (agent.Preview, error) {
	if f.previewRequest != nil {
		return f.previewRequest(ctx, request)
	}
	return f.preview(ctx, request.Agents)
}

func (f *fakeAgent) Write(ctx context.Context, request agent.WriteRequest) (agent.WriteResult, error) {
	return f.write(ctx, request)
}

func (f *fakeAgent) CleanupPreview(ctx context.Context, request agent.CleanupPreviewRequest) (agent.CleanupPreview, error) {
	return f.cleanupPreview(ctx, request)
}

func (f *fakeAgent) CleanupWrite(ctx context.Context, request agent.CleanupWriteRequest) (agent.WriteResult, error) {
	return f.cleanupWrite(ctx, request)
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

func TestAgentPreviewMapsModesWarningsAndSidecarBackupPlan(t *testing.T) {
	var captured agent.PreviewRequest
	manager := newWithDependencies(Config{}, dependencies{agent: &fakeAgent{
		previewRequest: func(_ context.Context, request agent.PreviewRequest) (agent.Preview, error) {
			captured = request
			return agent.Preview{
				RevisionToken: "revision", ModelConfig: request.ModelConfig,
				Agents: []agent.AgentPreview{{
					Agent: agent.ClaudeCode, Mode: agent.ConfigModeRebuild,
					Files: []agent.FilePreview{{
						Role: "config", Path: "/settings.json", Format: agent.FormatJSON, Operation: agent.OperationReplace,
						Backup: agent.BackupPlan{Required: true, Pattern: "/settings.json.bak-<timestamp>-<random>", Sensitive: true, Warning: "backup warning"}, Warning: "rebuild warning",
					}},
				}},
				StateChange: &agent.FilePreview{Path: "/state.json", Format: agent.FormatJSON, Operation: agent.OperationReplace, Backup: agent.BackupPlan{
					Required: true, Pattern: "/state.json.bak-<timestamp>-<random>", Sensitive: true, Warning: "state backup warning",
				}},
			}, nil
		},
	}})
	request := `{"id":"preview","method":"agent.preview","params":{"agents":["claude"],"modes":{"claude":"rebuild"},"catalog_token":"catalog","model_config":{"version":1}}}` + "\n"
	var output strings.Builder
	if err := manager.Serve(context.Background(), strings.NewReader(request), &output); err != nil {
		t.Fatal(err)
	}
	if captured.Modes[agent.ClaudeCode] != agent.ConfigModeRebuild {
		t.Fatalf("captured request = %#v", captured)
	}
	var response protocol.Response
	if err := json.Unmarshal([]byte(strings.TrimSpace(output.String())), &response); err != nil {
		t.Fatal(err)
	}
	var result protocol.AgentPreviewResult
	if err := json.Unmarshal(response.Result, &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Files) != 1 || result.Files[0].Mode != "rebuild" || result.Files[0].Warning != "rebuild warning" || !result.Files[0].BackupRequired || result.Files[0].BackupPattern == "" {
		t.Fatalf("preview files = %#v", result.Files)
	}
	if result.StateChange == nil || !result.StateChange.BackupRequired || !result.StateChange.BackupSensitive || result.StateChange.BackupPattern != "/state.json.bak-<timestamp>-<random>" || result.StateChange.Warning != "state backup warning" {
		t.Fatalf("state change = %#v", result.StateChange)
	}
	if result.StateChange.BackupPath != "" || result.StateBackup != nil {
		t.Fatalf("preview exposed actual state backup: change=%#v backup=%#v", result.StateChange, result.StateBackup)
	}
}

func TestAgentCleanupHandlersDispatchDirectlyAndMapSafeResults(t *testing.T) {
	const forbidden = "cleanup-secret-canary"
	var previewRequest agent.CleanupPreviewRequest
	var writeRequest agent.CleanupWriteRequest
	routerCalls, catalogCalls := 0, 0
	agentManager := &fakeAgent{
		cleanupPreview: func(_ context.Context, request agent.CleanupPreviewRequest) (agent.CleanupPreview, error) {
			previewRequest = request
			return agent.CleanupPreview{
				RevisionToken: "cleanup-revision", Agent: agent.OpenCode,
				Files: []agent.FilePreview{{
					Role: "config", Path: "/safe/opencode.json", Format: agent.FormatJSON, Operation: agent.OperationDelete,
					Preserves: []string{"unmanaged configuration"}, Backup: agent.BackupPlan{Required: true, Pattern: "/safe/opencode.json.bak-<timestamp>-<random>", Sensitive: true, Warning: "backup warning"},
				}},
				RemovedPaths: []string{"model", "provider.mtls-router"}, ManagedConfigDrift: true,
				StateChange: &agent.FilePreview{Role: "state", Path: "/safe/state.json", Format: agent.FormatJSON, Operation: agent.OperationDelete, Backup: agent.BackupPlan{Required: true, Pattern: "/safe/state.json.bak-<timestamp>-<random>", Sensitive: false, Warning: "state warning"}},
				StateBackup: &agent.FilePreview{Role: "state", Path: "/safe/state.json", Format: agent.FormatJSON, Operation: agent.Operation("backup")},
			}, nil
		},
		cleanupWrite: func(_ context.Context, request agent.CleanupWriteRequest) (agent.WriteResult, error) {
			writeRequest = request
			return agent.WriteResult{
				TransactionID: "cleanup-transaction",
				Agents:        []agent.AgentWriteStatus{{Agent: agent.OpenCode, Success: true, Changed: []string{"/safe/opencode.json"}, Backups: []string{"/safe/opencode.json.bak"}}},
				StateChange:   &agent.FileWriteStatus{Path: "/safe/state.json", Operation: agent.OperationDelete, Replaced: true}, StateBackup: &agent.FileWriteStatus{Path: "/safe/state.json.bak", Operation: agent.OperationBackup, BackupPath: "/safe/state.json.bak"},
			}, nil
		},
		binding: func(context.Context, []agent.Kind, string, json.RawMessage) (agent.CatalogBinding, error) {
			catalogCalls++
			return agent.CatalogBinding{}, errors.New(forbidden)
		},
	}
	manager := newWithDependencies(Config{}, dependencies{
		agent: agentManager,
		trusted: fakeTrustedRouter{
			fetch: func(context.Context, protocol.RouterOwner, string) (trustedrouter.Result, *protocol.Error) {
				routerCalls++
				return trustedrouter.Result{}, nil
			},
			revalidate: func(context.Context, protocol.RouterOwner, string, trustedrouter.Binding) ([]string, *protocol.Error) {
				routerCalls++
				return nil, nil
			},
		},
	})
	requests := []string{
		`{"id":"preview","method":"agent.cleanup.preview","params":{"agent":"opencode"}}`,
		`{"id":"write","method":"agent.cleanup.write","params":{"agent":"opencode","revision_token":"cleanup-revision","approve_managed_overwrite":true}}`,
		`{"id":"reject","method":"agent.cleanup.preview","params":{"agent":"opencode","api_key":"` + forbidden + `","catalog_token":"` + forbidden + `","model_config":{"saved":"` + forbidden + `"},"flow_id":"` + forbidden + `"}}`,
		`{"id":"missing-approval","method":"agent.cleanup.write","params":{"agent":"opencode","revision_token":"cleanup-revision"}}`,
	}
	var output strings.Builder
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	if previewRequest.Agent != agent.OpenCode || writeRequest.Agent != agent.OpenCode || writeRequest.RevisionToken != "cleanup-revision" || !writeRequest.ApproveManagedOverwrite {
		t.Fatalf("cleanup requests = preview %#v write %#v", previewRequest, writeRequest)
	}
	if routerCalls != 0 || catalogCalls != 0 {
		t.Fatalf("cleanup touched router/catalog: trusted=%d catalog=%d", routerCalls, catalogCalls)
	}
	if strings.Contains(output.String(), forbidden) {
		t.Fatalf("cleanup response leaked rejected data: %s", output.String())
	}
	for _, prose := range []string{"unmanaged configuration", "backup warning", "state warning"} {
		if strings.Contains(output.String(), prose) {
			t.Fatalf("cleanup response exposed manager prose %q: %s", prose, output.String())
		}
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	var previewResponse, writeResponse, rejectedResponse, missingApprovalResponse protocol.Response
	for i, target := range []*protocol.Response{&previewResponse, &writeResponse, &rejectedResponse, &missingApprovalResponse} {
		if err := json.Unmarshal([]byte(lines[i]), target); err != nil {
			t.Fatal(err)
		}
	}
	var preview protocol.AgentCleanupPreviewResult
	if previewResponse.Error != nil || json.Unmarshal(previewResponse.Result, &preview) != nil {
		t.Fatalf("preview response = %#v", previewResponse)
	}
	if preview.Agent != "opencode" || len(preview.Files) != 1 || preview.Files[0].Operation != "delete" || strings.Join(preview.RemovedPaths, ",") != "model,provider.mtls-router" || preview.StateChange == nil || preview.StateChange.Operation != "delete" || preview.StateBackup == nil {
		t.Fatalf("cleanup preview = %#v", preview)
	}
	if len(preview.Files[0].Preserves) != 0 || preview.Files[0].Warning != "" || preview.StateChange.Warning != "" {
		t.Fatalf("cleanup preview exposed manager prose = %#v", preview)
	}
	if !preview.Files[0].BackupSensitive || preview.StateChange.BackupSensitive || preview.StateBackup.BackupSensitive {
		t.Fatalf("cleanup backup sensitivity = file %#v state %#v backup %#v", preview.Files[0], preview.StateChange, preview.StateBackup)
	}
	var write protocol.AgentWriteResult
	if writeResponse.Error != nil || json.Unmarshal(writeResponse.Result, &write) != nil || len(write.Agents) != 1 || strings.Join(write.Agents[0].Changed, ",") != "/safe/opencode.json" || strings.Join(write.Agents[0].Backups, ",") != "/safe/opencode.json.bak" {
		t.Fatalf("cleanup write = %#v response=%#v", write, writeResponse)
	}
	if write.StateChange == nil || write.StateChange.Operation != "delete" || write.StateBackup == nil || write.StateBackup.Operation != "backup" {
		t.Fatalf("cleanup write state effects = change %#v backup %#v", write.StateChange, write.StateBackup)
	}
	if rejectedResponse.Error == nil || rejectedResponse.Error.Code != protocol.CodeInvalidParams {
		t.Fatalf("rejected response = %#v", rejectedResponse)
	}
	if missingApprovalResponse.Error == nil || missingApprovalResponse.Error.Code != protocol.CodeInvalidParams {
		t.Fatalf("missing approval response = %#v", missingApprovalResponse)
	}
}

func TestMapWriteUsesActualStateOperation(t *testing.T) {
	for _, operation := range []agent.Operation{agent.OperationCreate, agent.OperationReplace, agent.OperationDelete} {
		t.Run(string(operation), func(t *testing.T) {
			mapped := mapWrite(agent.WriteResult{StateChange: &agent.FileWriteStatus{Path: "/state", Operation: operation, Replaced: true}})
			if mapped.StateChange == nil || mapped.StateChange.Operation != string(operation) {
				t.Fatalf("mapped state change = %#v", mapped.StateChange)
			}
		})
	}
}

func TestAgentCleanupMapsNotManagedAndDetectionStateWithoutDetails(t *testing.T) {
	const forbidden = "saved-model-url-api-key-canary"
	manager := newWithDependencies(Config{}, dependencies{
		detect: func() ([]agent.State, error) {
			return []agent.State{{Agent: agent.OpenCode, Name: "opencode", Cleanup: agent.CleanupState{Managed: true, Available: false, Reason: agent.CleanupWritesDisabled}}}, nil
		},
		agent: &fakeAgent{
			cleanupPreview: func(context.Context, agent.CleanupPreviewRequest) (agent.CleanupPreview, error) {
				return agent.CleanupPreview{}, &agent.OperationError{Code: agent.CodeAgentNotManaged}
			},
			cleanupWrite: func(context.Context, agent.CleanupWriteRequest) (agent.WriteResult, error) {
				return agent.WriteResult{}, errors.New(forbidden)
			},
		},
	})
	requests := []string{
		`{"id":"detect","method":"agent.detect"}`,
		`{"id":"preview","method":"agent.cleanup.preview","params":{"agent":"opencode"}}`,
		`{"id":"write","method":"agent.cleanup.write","params":{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false}}`,
	}
	var output strings.Builder
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(output.String(), forbidden) {
		t.Fatalf("cleanup response leaked service error: %s", output.String())
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if !strings.Contains(lines[0], `"cleanup":{"managed":true,"available":false,"reason":"writes_disabled"}`) {
		t.Fatalf("detection cleanup state = %s", lines[0])
	}
	if !strings.Contains(lines[1], `"code":"AGENT_NOT_MANAGED"`) || !strings.Contains(lines[2], `"code":"WRITE_FAILED"`) {
		t.Fatalf("cleanup errors = %s", output.String())
	}
}

func TestMapPreviewEncodesEmptyCollectionsAsArrays(t *testing.T) {
	encoded, err := json.Marshal(mapPreview(agent.Preview{ModelConfig: json.RawMessage(`{"version":1}`)}))
	if err != nil {
		t.Fatal(err)
	}
	var result map[string]json.RawMessage
	if err := json.Unmarshal(encoded, &result); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"fragments", "files", "drifted_agents", "managed_collisions"} {
		if got := string(result[field]); got != "[]" {
			t.Errorf("%s = %s, want []", field, got)
		}
	}
}

func TestMapCleanupPreviewEncodesEmptyCollectionsAsArrays(t *testing.T) {
	encoded, err := json.Marshal(mapCleanupPreview(agent.CleanupPreview{}))
	if err != nil {
		t.Fatal(err)
	}
	var result map[string]json.RawMessage
	if err := json.Unmarshal(encoded, &result); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"files", "removed_paths"} {
		if got := string(result[field]); got != "[]" {
			t.Errorf("%s = %s, want []", field, got)
		}
	}
}

func TestAgentRenderRejectsModesAndStrictRecoveryOverrides(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{agent: &fakeAgent{render: func(context.Context, []agent.Kind, string, json.RawMessage) (agent.RenderResult, error) {
		return agent.RenderResult{}, nil
	}}})
	requests := []string{
		`{"id":"render-modes","method":"agent.render","params":{"agents":["claude"],"modes":{"claude":"merge"},"catalog_token":"catalog","model_config":{"version":1}}}`,
		`{"id":"force","method":"agent.preview","params":{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1},"force":true}}`,
		`{"id":"ignore","method":"agent.preview","params":{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1},"ignore_parse_errors":true}}`,
	}
	var output strings.Builder
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(strings.TrimSpace(output.String()), "\n") {
		var response protocol.Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatal(err)
		}
		if response.Error == nil || response.Error.Code != protocol.CodeInvalidParams {
			t.Fatalf("response = %s", line)
		}
	}
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
			Version: "router-v1", PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "4",
		},
		Health: discovery.Health{Status: "ok"},
		State: state.RouterState{
			PID: 91, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099", LogPath: logPath,
			RouterVersion: "router-v1", DeploymentID: "prod-a", ManagementProtocolVersion: "4",
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
			return protocol.ManagerInfoResult{Version: "manager-v1", Commit: "abc123", BuildDate: "2026-07-12T00:00:00Z", Target: "test/test", DeploymentID: "prod-a", ManagementProtocolVersion: "4"}
		},
		discoverStatus: func(context.Context) discovery.Result { return found },
		discoverHealth: func(context.Context) discovery.Result { return found },
		lifecycle:      lifecycleManager,
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
	responses := make(map[string]protocol.Response, len(lines))
	for index, line := range lines {
		var response protocol.Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatalf("response %d: %v", index, err)
		}
		wantID := wantIDs[index]
		if response.ID == nil || *response.ID != wantID || response.Error != nil {
			t.Fatalf("response %d = %#v, want ID %q", index, response, wantID)
		}
		responses[wantID] = response
	}
	var detectResponse struct {
		Agents []map[string]json.RawMessage `json:"agents"`
	}
	if err := json.Unmarshal(responses["9"].Result, &detectResponse); err != nil {
		t.Fatal(err)
	}
	command, ok := detectResponse.Agents[0]["command"]
	if !ok || string(command) != `""` {
		t.Fatalf("agent.detect command compatibility field = %s, present = %t", command, ok)
	}
	if !strings.Contains(output.String(), "[REDACTED") || !strings.Contains(output.String(), `"commit":"abc123"`) {
		t.Fatalf("output lacks sanitized logs or version metadata: %s", output.String())
	}
}

func TestRouterStatusUsesStatusDiscovery(t *testing.T) {
	statusCalls, healthCalls := 0, 0
	manager := newWithDependencies(Config{}, dependencies{
		discoverStatus: func(context.Context) discovery.Result {
			statusCalls++
			return discovery.Result{Classification: discovery.DesktopOwned, Owner: "desktop"}
		},
		discoverHealth: func(context.Context) discovery.Result {
			healthCalls++
			return discovery.Result{Classification: discovery.Degraded}
		},
		lifecycle: &fakeLifecycle{},
	})

	result, responseErr := manager.routerStatus(context.Background(), json.RawMessage(`{}`))
	if responseErr != nil {
		t.Fatal(responseErr)
	}
	status := result.(protocol.RouterStatusResult)
	if status.State != string(discovery.DesktopOwned) {
		t.Fatalf("status state = %q, want %q", status.State, discovery.DesktopOwned)
	}
	if statusCalls != 1 || healthCalls != 0 {
		t.Fatalf("status calls=%d health calls=%d", statusCalls, healthCalls)
	}
}

func TestDiagnosticsCollectUsesStatusDiscoveryAndSanitizesOutput(t *testing.T) {
	statusCalls, healthCalls := 0, 0
	manager := newWithDependencies(Config{}, dependencies{
		info: func() protocol.ManagerInfoResult {
			return protocol.ManagerInfoResult{Version: "manager-v1"}
		},
		discoverStatus: func(context.Context) discovery.Result {
			statusCalls++
			return discovery.Result{
				Classification: discovery.DesktopOwned,
				Owner:          "desktop",
				ListenAddr:     "http://127.0.0.1:19099?api_key=" + integrationKey,
			}
		},
		discoverHealth: func(context.Context) discovery.Result {
			healthCalls++
			return discovery.Result{Health: discovery.Health{Status: integrationKey}}
		},
		detect: func() ([]agent.State, error) { return nil, nil },
	})

	result, responseErr := manager.diagnosticsCollect(context.Background(), json.RawMessage(`{}`))
	if responseErr != nil {
		t.Fatal(responseErr)
	}
	summary := result.(protocol.DiagnosticsResult).Summary
	if statusCalls != 1 || healthCalls != 0 {
		t.Fatalf("status calls=%d health calls=%d", statusCalls, healthCalls)
	}
	if strings.Contains(summary, integrationKey) {
		t.Fatalf("diagnostics contain sensitive input: %s", summary)
	}
	if !strings.Contains(summary, "listen=http://127.0.0.1:19099?[REDACTED]") || !strings.Contains(summary, " health=\n") {
		t.Fatalf("diagnostics lack sanitized status or empty process-only health: %s", summary)
	}
}

func TestRouterHealthUsesHealthDiscovery(t *testing.T) {
	statusCalls, healthCalls := 0, 0
	checkedAt := time.Date(2026, 7, 20, 1, 2, 3, 0, time.UTC)
	manager := newWithDependencies(Config{}, dependencies{
		discoverStatus: func(context.Context) discovery.Result {
			statusCalls++
			return discovery.Result{Classification: discovery.DesktopOwned}
		},
		discoverHealth: func(context.Context) discovery.Result {
			healthCalls++
			return discovery.Result{Classification: discovery.DesktopOwned, Health: discovery.Health{Status: "ok"}}
		},
		now: func() time.Time { return checkedAt },
	})

	result, responseErr := manager.routerHealth(context.Background(), json.RawMessage(`{}`))
	if responseErr != nil {
		t.Fatal(responseErr)
	}
	health := result.(protocol.RouterHealthResult)
	if health.Status != "ok" || !health.CheckedAt.Equal(checkedAt) {
		t.Fatalf("health result = %#v", health)
	}
	if statusCalls != 0 || healthCalls != 1 {
		t.Fatalf("status calls=%d health calls=%d", statusCalls, healthCalls)
	}
}

func TestRouterVersionUsesStatusDiscovery(t *testing.T) {
	statusCalls, healthCalls := 0, 0
	manager := newWithDependencies(Config{}, dependencies{
		discoverStatus: func(context.Context) discovery.Result {
			statusCalls++
			return discovery.Result{
				Classification: discovery.DesktopOwned,
				Version: discovery.Version{
					Version: "router-v1", DeploymentID: "prod-a", ManagementProtocolVersion: "4",
				},
			}
		},
		discoverHealth: func(context.Context) discovery.Result {
			healthCalls++
			return discovery.Result{Classification: discovery.Degraded}
		},
	})

	result, responseErr := manager.routerVersion(context.Background(), json.RawMessage(`{}`))
	if responseErr != nil {
		t.Fatal(responseErr)
	}
	versionResult := result.(protocol.RouterVersionResult)
	if versionResult.Version != "router-v1" {
		t.Fatalf("version = %q, want router-v1", versionResult.Version)
	}
	if statusCalls != 1 || healthCalls != 0 {
		t.Fatalf("status calls=%d health calls=%d", statusCalls, healthCalls)
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

func TestMapAgentErrorIncludesSafeValidationDetails(t *testing.T) {
	err := &modelconfig.ValidationError{Path: "/opencode/models/model-a/options/image_url", Rule: "protected_path"}
	got := mapAgentError(err)
	if got.Code != protocol.CodeModelConfigInvalid || got.Details == nil {
		t.Fatalf("mapped error = %#v", got)
	}
	if got.Details.Path != "/opencode/models/model-a/options/image_url" || got.Details.Rule != "protected_path" {
		t.Fatalf("details = %#v", got.Details)
	}
}

func TestHandlersRejectInvalidTypedParameters(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{
		info:           func() protocol.ManagerInfoResult { return protocol.ManagerInfoResult{} },
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
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
		DeploymentID: "prod-a", ProtocolVersion: "4",
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
			if claims.Owner != "cli" || claims.RouterBaseURL != binding.RouterBaseURL || claims.DeploymentID != binding.DeploymentID || claims.ProtocolVersion != "4" {
				t.Fatalf("claims=%+v", claims)
			}
			return agent.ModelsResult{CatalogToken: "signed-catalog", Existing: agent.ModelsExisting{
				ModelConfig:       json.RawMessage(`{"version":1,"codex":{"model":"model-a"}}`),
				UnavailableModels: map[string][]string{"claude": {"old-model"}}, DriftedAgents: []string{"claude", "codex"},
			}, Preset: agent.ModelsPreset{
				ModelConfig:       json.RawMessage(`{"version":1,"codex":{"model":"model-a"}}`),
				UnavailableAgents: map[string][]string{"claude": {"missing-a", "missing-z"}},
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
		strings.Join(result.Existing.DriftedAgents, ",") != "claude,codex" ||
		string(result.Preset.ModelConfig) != `{"version":1,"codex":{"model":"model-a"}}` ||
		result.Preset.UnavailableAgents["claude"].Code != protocol.CodeModelNotAvailable ||
		strings.Join(result.Preset.UnavailableAgents["claude"].Models, ",") != "missing-a,missing-z" {
		t.Fatalf("result = %+v", result)
	}
}

func TestAgentModelsNoPresetUsesStableEmptyObjects(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{
		trusted: fakeTrustedRouter{fetch: func(context.Context, protocol.RouterOwner, string) (trustedrouter.Result, *protocol.Error) {
			return trustedrouter.Result{Models: []string{"model-a"}, Binding: trustedrouter.Binding{RouterBaseURL: "http://127.0.0.1:19099", APIBaseURL: "http://127.0.0.1:19099/v1", DeploymentID: "prod-a", ProtocolVersion: "4"}}, nil
		}},
		models: fakeModelsService{discover: func(context.Context, []agent.Kind, []string, modelconfig.CatalogClaims) (agent.ModelsResult, error) {
			return agent.ModelsResult{CatalogToken: "token", Existing: agent.ModelsExisting{ModelConfig: json.RawMessage(`{}`), UnavailableModels: map[string][]string{}, DriftedAgents: []string{}}}, nil
		}},
	})
	var output bytes.Buffer
	request := `{"id":"models","method":"agent.models","params":{"owner":"cli","agents":["claude"],"api_key":"secret"}}` + "\n"
	if err := manager.Serve(context.Background(), strings.NewReader(request), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), `"preset":{"model_config":{},"unavailable_agents":{}}`) || strings.Contains(output.String(), "secret") {
		t.Fatalf("response = %s", output.String())
	}
}

func TestNewRejectsMalformedPresetBeforeRecovery(t *testing.T) {
	previous := preset.Encoded
	preset.Encoded = "malformed-preset-canary%%%"
	t.Cleanup(func() { preset.Encoded = previous })
	dir := t.TempDir()
	stateDir := filepath.Join(dir, "agent-transactions")
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatal(err)
	}
	journal := filepath.Join(stateDir, "agent-write-journal.json")
	if err := os.WriteFile(journal, []byte(`not-json-recovery-canary`), 0o600); err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer
	_, err := New(Config{ListenAddr: "127.0.0.1:19099", RouterPath: os.Args[0], ManagerIdentity: process.Identity{PID: 1, StartedAt: "test", Executable: os.Args[0]}, Paths: managerpaths.Paths{DesktopDataDir: dir}, Stderr: &stderr}, true)
	if err == nil || err.Error() != "invalid embedded Agent model preset" {
		t.Fatalf("error = %v", err)
	}
	if stderr.Len() != 0 || strings.Contains(err.Error(), "malformed-preset-canary") {
		t.Fatalf("startup leaked or attempted recovery: err=%v stderr=%q", err, stderr.String())
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
			return agent.CatalogBinding{Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "4", Models: []string{"model-a"}}, nil
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

func TestAgentWriteThreadsRebuildModesApprovalAndStateBackupArtifact(t *testing.T) {
	config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-a"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	var validated, written agent.WriteRequest
	agentManager := &fakeAgent{
		validatePreview: func(_ context.Context, request agent.WriteRequest) error { validated = request; return nil },
		binding: func(context.Context, []agent.Kind, string, json.RawMessage) (agent.CatalogBinding, error) {
			return agent.CatalogBinding{Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "2"}, nil
		},
		write: func(_ context.Context, request agent.WriteRequest) (agent.WriteResult, error) {
			written = request
			return agent.WriteResult{TransactionID: "transaction", StateChange: &agent.FileWriteStatus{Path: "/state.json", Operation: agent.OperationReplace, Replaced: true}, StateBackup: &agent.FileWriteStatus{Path: "/state.json.bak-real", Operation: agent.OperationBackup, BackupPath: "/state.json.bak-real"}}, nil
		},
	}
	manager := newWithDependencies(Config{}, dependencies{
		agent: agentManager,
		trusted: fakeTrustedRouter{revalidate: func(context.Context, protocol.RouterOwner, string, trustedrouter.Binding) ([]string, *protocol.Error) {
			return []string{"model-a"}, nil
		}},
	})
	params := map[string]any{
		"agents": []string{"claude"}, "modes": map[string]string{"claude": "rebuild"}, "approve_rebuild": []string{"claude"},
		"catalog_token": "catalog", "model_config": config, "revision_token": "revision", "api_key": integrationKey,
		"approve_managed_overwrite": false, "approve_codex_auth_change": false,
	}
	raw, _ := json.Marshal(params)
	value, gotErr := manager.agentWrite(context.Background(), raw)
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	for name, request := range map[string]agent.WriteRequest{"validated": validated, "written": written} {
		if request.Modes[agent.ClaudeCode] != agent.ConfigModeRebuild || len(request.ApproveRebuild) != 1 || request.ApproveRebuild[0] != agent.ClaudeCode {
			t.Fatalf("%s request = %#v", name, request)
		}
	}
	result := value.(protocol.AgentWriteResult)
	if result.StateChange == nil || result.StateChange.Path != "/state.json" || result.StateBackup == nil || result.StateBackup.Path != "/state.json.bak-real" {
		t.Fatalf("write result = %#v", result)
	}
}

func TestAgentWriteEveryPreflightFailureCreatesZeroArtifacts(t *testing.T) {
	config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-a"},"fable":{"model":"fable-model"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
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
		{name: "removed selected Fable model", params: writeParams(config, true, true), refreshedModels: []string{"model-a"}, want: protocol.CodeModelNotAvailable},
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
					return agent.CatalogBinding{Owner: "cli", RouterBaseURL: "http://127.0.0.1:19099", DeploymentID: "prod-a", ProtocolVersion: "4"}, nil
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
					return []string{"model-a", "fable-model"}, nil
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

func TestOccupantHandlersExposeSafeResultAndSubmitOnlyToken(t *testing.T) {
	expiresAt := time.Date(2026, 7, 18, 1, 2, 33, 0, time.UTC)
	token := "opaque-confirmation"
	executable := filepath.Join(t.TempDir(), "listener")
	forceCalls := 0
	manager := newWithDependencies(Config{}, dependencies{occupant: &fakeOccupant{
		inspect: func(context.Context) (occupant.Inspection, error) {
			return occupant.Inspection{
				PID: 42, VerificationMode: occupant.VerificationModeVerifiedIdentity, ProcessName: "listener", Executable: executable,
				ListenAddr: "127.0.0.1:19099", Recovery: occupant.Recovery{Action: occupant.RecoveryActionForceTerminate}, ConfirmationToken: token, ExpiresAt: &expiresAt,
			}, nil
		},
		forceTerminate: func(_ context.Context, got string) (occupant.Result, error) {
			forceCalls++
			if got != token {
				t.Fatalf("token = %q", got)
			}
			return occupant.Result{Termination: "process_terminated", PortState: "released"}, nil
		},
	}})
	input := strings.NewReader(strings.Join([]string{
		`{"id":"inspect","method":"router.inspect_occupant"}`,
		`{"id":"terminate","method":"router.force_terminate_occupant","params":{"confirmation_token":"` + token + `"}}`,
	}, "\n") + "\n")
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	want := strings.Join([]string{
		`{"id":"inspect","result":{"pid":42,"verification_mode":"verified_identity","process_name":"listener","executable":` + strconv.Quote(executable) + `,"listen_addr":"127.0.0.1:19099","recovery":{"action":"force_terminate"},"confirmation_token":"opaque-confirmation","expires_at":"2026-07-18T01:02:33Z"}}`,
		`{"id":"terminate","result":{"termination":"process_terminated","port_state":"released"}}`,
	}, "\n") + "\n"
	if forceCalls != 1 || output.String() != want {
		t.Fatalf("force calls=%d response=%s, want %s", forceCalls, output.String(), want)
	}
}

func TestOccupantHandlerOmitsUnverifiedProcessMetadata(t *testing.T) {
	expiresAt := time.Date(2026, 7, 22, 12, 0, 30, 0, time.UTC)
	manager := newWithDependencies(Config{}, dependencies{occupant: &fakeOccupant{
		inspect: func(context.Context) (occupant.Inspection, error) {
			return occupant.Inspection{
				PID: 4242, VerificationMode: occupant.VerificationModeWindowsPIDOnly,
				ListenAddr: "127.0.0.1:19099", Recovery: occupant.Recovery{Action: occupant.RecoveryActionForceTerminate}, ConfirmationToken: "token", ExpiresAt: &expiresAt,
			}, nil
		},
	}})
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(`{"id":"inspect","method":"router.inspect_occupant"}`+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	want := `{"id":"inspect","result":{"pid":4242,"verification_mode":"windows_pid_only","listen_addr":"127.0.0.1:19099","recovery":{"action":"force_terminate"},"confirmation_token":"token","expires_at":"2026-07-22T12:00:30Z"}}` + "\n"
	if output.String() != want {
		t.Fatalf("response = %s, want %s", output.String(), want)
	}
}

func TestOccupantHandlerMapsBlockedServiceExactly(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{occupant: &fakeOccupant{
		inspect: func(context.Context) (occupant.Inspection, error) {
			return occupant.Inspection{
				PID: 8, VerificationMode: occupant.VerificationModeWindowsPIDOnly, ListenAddr: "127.0.0.1:19099",
				Recovery:   occupant.Recovery{Action: occupant.RecoveryActionManualStopRequired, Reason: occupant.RecoveryReasonServiceManaged},
				Supervisor: &occupant.Supervisor{Kind: occupant.SupervisorWindowsService, Scope: occupant.SupervisorScopeSystem, Identifiers: []string{"Svc"}},
			}, nil
		},
	}})
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(`{"id":"inspect","method":"router.inspect_occupant"}`+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	want := `{"id":"inspect","result":{"pid":8,"verification_mode":"windows_pid_only","listen_addr":"127.0.0.1:19099","recovery":{"action":"manual_stop_required","reason":"service_managed"},"supervisor":{"kind":"windows_service","scope":"system","identifiers":["Svc"]}}}` + "\n"
	if output.String() != want {
		t.Fatalf("response = %s, want %s", output.String(), want)
	}
}

func TestProtectedStatePID(t *testing.T) {
	dir := t.TempDir()
	desktopPath := filepath.Join(dir, "desktop-state.json")
	cliPath := filepath.Join(dir, "cli-state.json")
	if err := state.Write(desktopPath, state.RouterState{PID: 4101}); err != nil {
		t.Fatal(err)
	}
	if err := state.Write(cliPath, state.RouterState{PID: 4102}); err != nil {
		t.Fatal(err)
	}

	isProtectedPID := protectedStatePID(desktopPath, cliPath)
	for _, pid := range []int{4101, 4102} {
		if !isProtectedPID(pid) {
			t.Errorf("PID %d was not protected", pid)
		}
	}
	if isProtectedPID(4103) || isProtectedPID(0) {
		t.Fatal("unmanaged or non-positive PID was protected")
	}
}

func TestProtectedStatePIDSkipsReadErrors(t *testing.T) {
	dir := t.TempDir()
	unreadablePath := filepath.Join(dir, "invalid-state.json")
	cliPath := filepath.Join(dir, "cli-state.json")
	if err := os.WriteFile(unreadablePath, []byte("not-json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := state.Write(cliPath, state.RouterState{PID: 4202}); err != nil {
		t.Fatal(err)
	}

	if protectedStatePID(unreadablePath, filepath.Join(dir, "missing-state.json"))(4201) {
		t.Fatal("state read errors alone protected the PID")
	}
	if !protectedStatePID(unreadablePath, cliPath)(4202) {
		t.Fatal("read error prevented protection from a later readable state")
	}
}

func TestOccupantHandlersRejectInvalidShapesAndMapSanitizedErrors(t *testing.T) {
	tokenCanary := "confirmation-token-canary"
	pathCanary := "/sensitive/full/path/listener"
	manager := newWithDependencies(Config{}, dependencies{occupant: &fakeOccupant{
		inspect: func(context.Context) (occupant.Inspection, error) {
			return occupant.Inspection{}, fmt.Errorf("%w: %s %s", occupant.ErrChanged, tokenCanary, pathCanary)
		},
		forceTerminate: func(context.Context, string) (occupant.Result, error) {
			return occupant.Result{}, fmt.Errorf("%w: %s %s", occupant.ErrTerminationFailed, tokenCanary, pathCanary)
		},
	}})
	requests := []string{
		`{"id":"inspect-shape","method":"router.inspect_occupant","params":{"extra":true}}`,
		`{"id":"missing","method":"router.force_terminate_occupant"}`,
		`{"id":"blank","method":"router.force_terminate_occupant","params":{"confirmation_token":" "}}`,
		`{"id":"extra","method":"router.force_terminate_occupant","params":{"confirmation_token":"x","pid":42}}`,
		`{"id":"inspect-error","method":"router.inspect_occupant"}`,
		`{"id":"terminate-error","method":"router.force_terminate_occupant","params":{"confirmation_token":"x"}}`,
	}
	var output bytes.Buffer
	if err := manager.Serve(context.Background(), strings.NewReader(strings.Join(requests, "\n")+"\n"), &output); err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	for index := 0; index < 4; index++ {
		if !strings.Contains(lines[index], `"code":"INVALID_PARAMS"`) {
			t.Fatalf("response %d = %s", index, lines[index])
		}
	}
	if !strings.Contains(lines[4], `"code":"OCCUPANT_CHANGED"`) || !strings.Contains(lines[5], `"code":"OCCUPANT_TERMINATION_FAILED"`) {
		t.Fatalf("mapped responses = %s", output.String())
	}
	if strings.Contains(output.String(), tokenCanary) || strings.Contains(output.String(), pathCanary) {
		t.Fatalf("error exposed internal detail: %s", output.String())
	}
}

func TestOccupantHandlerMapsPermissionDeniedExactly(t *testing.T) {
	manager := newWithDependencies(Config{}, dependencies{occupant: &fakeOccupant{
		forceTerminate: func(context.Context, string) (occupant.Result, error) {
			return occupant.Result{}, fmt.Errorf("%w: sensitive native error", occupant.ErrPermissionDenied)
		},
	}})
	var output bytes.Buffer
	request := `{"id":"terminate","method":"router.force_terminate_occupant","params":{"confirmation_token":"token"}}` + "\n"
	if err := manager.Serve(context.Background(), strings.NewReader(request), &output); err != nil {
		t.Fatal(err)
	}
	want := `{"id":"terminate","error":{"code":"OCCUPANT_PERMISSION_DENIED","message":"permission to terminate port occupant was denied"}}` + "\n"
	if output.String() != want {
		t.Fatalf("response = %s, want %s", output.String(), want)
	}
}

func TestMapOccupantErrorCoversStableCodes(t *testing.T) {
	tests := map[error]protocol.ErrorCode{
		occupant.ErrNotFound:            protocol.CodeOccupantNotFound,
		occupant.ErrNotOwned:            protocol.CodeOccupantNotOwned,
		occupant.ErrIdentityUnavailable: protocol.CodeOccupantIdentityUnavailable,
		occupant.ErrChanged:             protocol.CodeOccupantChanged,
		occupant.ErrProtected:           protocol.CodeOccupantProtected,
		occupant.ErrPermissionDenied:    protocol.CodeOccupantPermissionDenied,
		occupant.ErrTerminationFailed:   protocol.CodeOccupantTerminationFailed,
		occupant.ErrPortReleaseTimeout:  protocol.CodePortReleaseTimeout,
		occupant.ErrConfirmationExpired: protocol.CodeConfirmationExpired,
	}
	for input, code := range tests {
		if got := mapOccupantError(input); got.Code != code {
			t.Errorf("mapOccupantError(%v) = %q, want %q", input, got.Code, code)
		}
	}
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
				discoverStatus: func(context.Context) discovery.Result { return found },
				lifecycle:      lifecycleManager,
			})
			manager.failure = &routerFailure{identity: process.Identity{PID: 91}}
			input := strings.NewReader("{\"id\":\"start\",\"method\":\"router.start\",\"params\":{\"owner\":\"desktop\"}}\n{\"id\":\"status\",\"method\":\"router.status\"}\n")
			var output bytes.Buffer
			if err := manager.Serve(context.Background(), input, &output); err != nil {
				t.Fatal(err)
			}
			if startCalls != 1 || reclaimCalls != 1 {
				t.Fatalf("start calls = %d, reclaim calls = %d", startCalls, reclaimCalls)
			}
			if manager.failure != nil {
				t.Fatal("successful reclaim retained prior failure")
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
		discoverStatus: func(context.Context) discovery.Result {
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

func TestDesktopRouterStartLatchesLaunchedFailureAndSuccessfulRestartClearsIt(t *testing.T) {
	recentOutput := make([]string, defaultLogLines+2)
	for index := range recentOutput {
		recentOutput[index] = fmt.Sprintf("startup line %d", index)
	}
	recentOutput = append(recentOutput,
		"Authorization: Bearer startup-secret-canary",
		strings.Repeat("x", maxLogLineBytes+20),
		"safe startup ending",
	)
	startCalls := 0
	lifecycleManager := &fakeLifecycle{start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
		startCalls++
		if startCalls == 1 {
			return state.RouterState{}, &lifecycle.Error{
				Code: protocol.CodeRouterStartFailed, Err: errors.New("internal startup-secret-canary"),
				Launched: true, RecentOutput: strings.Join(recentOutput, "\n"),
			}
		}
		return state.RouterState{PID: 92, Owner: "desktop", ProcessStartedAt: "restart", ProcessExecutable: "/router"}, nil
	}}
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle:      lifecycleManager,
	})

	if _, gotErr := manager.routerStart(context.Background(), json.RawMessage(`{"owner":"desktop"}`)); gotErr == nil || gotErr.Code != protocol.CodeRouterStartFailed || gotErr.Message != "router could not be started" {
		t.Fatalf("start error = %+v", gotErr)
	}
	value, gotErr := manager.routerStatus(context.Background(), json.RawMessage(`{}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	failed := value.(protocol.RouterStatusResult)
	if failed.State != "start_failed" || failed.LastError != "desktop-owned router failed during startup" {
		t.Fatalf("failed status = %+v", failed)
	}
	if len(failed.RecentLogs) != defaultLogLines || failed.RecentLogs[len(failed.RecentLogs)-1] != "safe startup ending" {
		t.Fatalf("recent logs = %d, ending = %q", len(failed.RecentLogs), failed.RecentLogs[len(failed.RecentLogs)-1])
	}
	for _, line := range failed.RecentLogs {
		if strings.Contains(line, "startup-secret-canary") || len(line) > maxLogLineBytes+len("[truncated]") {
			t.Fatalf("unsafe startup log = %q", line)
		}
	}
	manager.failureMu.Lock()
	manager.failure.identity = process.Identity{PID: 92, StartedAt: "restart", Executable: "/router"}
	manager.failureMu.Unlock()

	if _, gotErr := manager.routerStart(context.Background(), json.RawMessage(`{"owner":"desktop"}`)); gotErr != nil {
		t.Fatal(gotErr)
	}
	value, gotErr = manager.routerStatus(context.Background(), json.RawMessage(`{}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	cleared := value.(protocol.RouterStatusResult)
	if cleared.State != string(discovery.Absent) || cleared.LastError != "" || len(cleared.RecentLogs) != 0 {
		t.Fatalf("status after successful restart = %+v", cleared)
	}
}

func TestDesktopRouterStartRetainsOriginalLaunchedFailureAcrossReclaim(t *testing.T) {
	lifecycleManager := &fakeLifecycle{
		start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			return state.RouterState{}, &lifecycle.Error{
				Code: protocol.CodeRouterStateStale, Err: errors.New("original detail"), Launched: true, RecentOutput: "original startup output",
			}
		},
		reclaim: func() (state.RouterState, *lifecycle.Error) {
			return state.RouterState{}, &lifecycle.Error{Code: protocol.CodeRouterAlreadyRunning, Err: errors.New("reclaim detail"), Launched: true, RecentOutput: "reclaim output"}
		},
	}
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Stale} },
		lifecycle:      lifecycleManager,
	})

	if _, gotErr := manager.routerStart(context.Background(), json.RawMessage(`{"owner":"desktop"}`)); gotErr == nil || gotErr.Code != protocol.CodeRouterAlreadyRunning || gotErr.Message != "router is already running" {
		t.Fatalf("start error = %+v", gotErr)
	}
	value, gotErr := manager.routerStatus(context.Background(), json.RawMessage(`{}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	failed := value.(protocol.RouterStatusResult)
	if fmt.Sprint(failed.RecentLogs) != fmt.Sprint([]string{"original startup output"}) {
		t.Fatalf("recent logs = %v", failed.RecentLogs)
	}
}

func TestDesktopRouterStartLatchesSafePreLaunchDiagnostic(t *testing.T) {
	lifecycleManager := &fakeLifecycle{start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
		return state.RouterState{}, &lifecycle.Error{
			Code:         protocol.CodeRouterStartFailed,
			Err:          errors.New(`CreateProcess C:\secret\router.exe: access denied canary`),
			Stage:        lifecycle.StartupStageProcessLaunch,
			OSErrorCode:  5,
			RecentOutput: "reason=upstream_probe_failed\nAuthorization: Bearer hidden-canary",
		}
	}}
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle:      lifecycleManager,
	})

	if _, gotErr := manager.routerStart(context.Background(), json.RawMessage(`{"owner":"desktop"}`)); gotErr == nil {
		t.Fatal("routerStart() unexpectedly succeeded")
	}
	value, gotErr := manager.routerStatus(context.Background(), json.RawMessage(`{}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	status := value.(protocol.RouterStatusResult)
	want := "stage=process_launch code=ROUTER_START_FAILED os_error=5"
	if status.State != "start_failed" || status.LastError != want || fmt.Sprint(status.RecentLogs) != fmt.Sprint([]string{want, "reason=upstream_probe_failed", "Authorization: [REDACTED]"}) {
		t.Fatalf("status = %+v", status)
	}
	value, gotErr = manager.routerLogs(context.Background(), json.RawMessage(`{"limit":10}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	logs := value.(protocol.RouterLogsResult).Lines
	if fmt.Sprint(logs) != fmt.Sprint(status.RecentLogs) {
		t.Fatalf("logs = %v, status logs = %v", logs, status.RecentLogs)
	}
	value, gotErr = manager.routerLogs(context.Background(), json.RawMessage(`{"limit":1}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	if logs := value.(protocol.RouterLogsResult).Lines; fmt.Sprint(logs) != fmt.Sprint([]string{want}) {
		t.Fatalf("single diagnostic log = %v", logs)
	}
	serialized := fmt.Sprintf("%+v %v", status, logs)
	for _, unsafe := range []string{"secret", "router.exe", "access denied", "canary"} {
		if strings.Contains(serialized, unsafe) {
			t.Fatalf("diagnostic exposed %q: %s", unsafe, serialized)
		}
	}
}

func TestStateReconcileFailureRemainsObservableWithoutBlockingAbsentAutoStart(t *testing.T) {
	lifecycleManager := &fakeLifecycle{
		start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			return state.RouterState{}, &lifecycle.Error{
				Code: protocol.CodeRouterStateStale, Err: errors.New("safe state conflict"),
				Stage: lifecycle.StartupStageStateReconcile,
			}
		},
		reclaim: func() (state.RouterState, *lifecycle.Error) {
			return state.RouterState{}, &lifecycle.Error{Code: protocol.CodeRouterStateStale, Err: errors.New("not reclaimable")}
		},
	}
	manager := newWithDependencies(Config{RouterPath: os.Args[0]}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle:      lifecycleManager,
	})

	if _, gotErr := manager.routerStart(context.Background(), json.RawMessage(`{"owner":"desktop"}`)); gotErr == nil {
		t.Fatal("routerStart() unexpectedly succeeded")
	}
	status, gotErr := manager.routerStatus(context.Background(), json.RawMessage(`{}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	if got := status.(protocol.RouterStatusResult); got.State != "start_failed" || !strings.Contains(got.LastError, "stage=state_reconcile") {
		t.Fatalf("status = %+v", got)
	}
	if !manager.absentStartOK() {
		t.Fatal("non-launched state reconciliation failure blocked safe absent auto-start")
	}
}

func TestStartupDiagnosticOmitsMissingOSErrorCode(t *testing.T) {
	got := startupDiagnostic(&lifecycle.Error{
		Code:  protocol.CodeRouterStartFailed,
		Stage: lifecycle.StartupStageLogOpen,
	})
	if got != "stage=log_open code=ROUTER_START_FAILED" {
		t.Fatalf("diagnostic = %q", got)
	}
	got = startupDiagnostic(&lifecycle.Error{
		Code:  protocol.CodeRouterStartFailed,
		Stage: lifecycle.StartupStage(`C:\secret\canary`),
	})
	if got != "stage=unknown code=ROUTER_START_FAILED" {
		t.Fatalf("unknown-stage diagnostic = %q", got)
	}
}

func TestRouterLogsMergesOnlyDiskSuffixMemoryPrefixOverlapAndAppliesLimit(t *testing.T) {
	dir := t.TempDir()
	logPath := filepath.Join(dir, "router.log")
	if err := os.WriteFile(logPath, []byte("disk-only\nrepeat\nrepeat\nshared-1\nshared-2\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	lifecycleManager := &fakeLifecycle{recent: "shared-1\nshared-2\nrepeat\nrepeat\nmemory-only\n"}
	manager := newWithDependencies(Config{Paths: managerpaths.Paths{DesktopLogFile: logPath}}, dependencies{
		discoverStatus: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.DesktopOwned, Owner: "desktop", State: state.RouterState{LogPath: logPath}}
		},
		lifecycle: lifecycleManager,
	})

	value, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":6}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	want := []string{"repeat", "shared-1", "shared-2", "repeat", "repeat", "memory-only"}
	if got := value.(protocol.RouterLogsResult).Lines; fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("lines = %v, want %v", got, want)
	}
}

func TestRouterLogsNeverMixesDesktopRecentOutputIntoTrustedCLILog(t *testing.T) {
	dir := t.TempDir()
	cliLogPath := filepath.Join(dir, "cli.log")
	if err := os.WriteFile(cliLogPath, []byte("cli-old\ncli-new\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	desktopLogPath := filepath.Join(dir, "desktop.log")
	if err := os.WriteFile(desktopLogPath, []byte("stale-desktop-disk\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	lifecycleManager := &fakeLifecycle{recent: "stale-desktop-1\nstale-desktop-2\n"}
	found := discovery.Result{
		Classification: discovery.ExternalCompatible, Owner: "cli",
		State: state.RouterState{Owner: "cli", LogPath: cliLogPath},
	}
	manager := newWithDependencies(Config{Paths: managerpaths.Paths{DesktopLogFile: desktopLogPath}}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return found },
		lifecycle:      lifecycleManager,
	})

	value, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":2}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	want := []string{"cli-old", "cli-new"}
	if got := value.(protocol.RouterLogsResult).Lines; fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("CLI lines = %v, want %v", got, want)
	}

	found.State.LogPath = dir
	if _, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":2}`)); gotErr == nil || gotErr.Code != protocol.CodeRouterStateStale {
		t.Fatalf("unreadable CLI log error = %+v", gotErr)
	}

	found.State.LogPath = ""
	value, gotErr = manager.routerLogs(context.Background(), json.RawMessage(`{"limit":2}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	if got := value.(protocol.RouterLogsResult).Lines; len(got) != 0 {
		t.Fatalf("missing CLI log returned desktop lines: %v", got)
	}
}

func TestRouterLogsUsesScopedStartupFailureInsteadOfHistoricalDesktopOutput(t *testing.T) {
	dir := t.TempDir()
	lifecycleManager := &fakeLifecycle{
		recent: "historical desktop output",
		start: func(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
			return state.RouterState{}, &lifecycle.Error{
				Code: protocol.CodeRouterStartFailed, Err: errors.New("startup failed"), Launched: true,
				RecentOutput: "scoped startup output\nAuthorization: Bearer scoped-secret",
			}
		},
	}
	manager := newWithDependencies(Config{RouterPath: os.Args[0], Paths: managerpaths.Paths{DesktopLogFile: dir}}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Stale} },
		lifecycle:      lifecycleManager,
	})

	if _, gotErr := manager.routerStart(context.Background(), json.RawMessage(`{"owner":"desktop"}`)); gotErr == nil {
		t.Fatal("routerStart() unexpectedly succeeded")
	}
	value, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":10}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	want := []string{"scoped startup output", "Authorization: [REDACTED]"}
	if got := value.(protocol.RouterLogsResult).Lines; fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("failure lines = %v, want %v", got, want)
	}
}

func TestRouterLogsFallsBackToMemoryWhenDiskIsUnreadable(t *testing.T) {
	dir := t.TempDir()
	manager := newWithDependencies(Config{Paths: managerpaths.Paths{DesktopLogFile: dir}}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle:      &fakeLifecycle{recent: "memory fallback"},
	})

	value, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":10}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	if got := value.(protocol.RouterLogsResult).Lines; fmt.Sprint(got) != fmt.Sprint([]string{"memory fallback"}) {
		t.Fatalf("lines = %v", got)
	}
}

func TestRouterLogsFallsBackToMemoryWhenDiskIsEmpty(t *testing.T) {
	dir := t.TempDir()
	logPath := filepath.Join(dir, "router.log")
	if err := os.WriteFile(logPath, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	manager := newWithDependencies(Config{Paths: managerpaths.Paths{DesktopLogFile: logPath}}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle:      &fakeLifecycle{recent: "memory fallback"},
	})

	value, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":10}`))
	if gotErr != nil {
		t.Fatal(gotErr)
	}
	if got := value.(protocol.RouterLogsResult).Lines; fmt.Sprint(got) != fmt.Sprint([]string{"memory fallback"}) {
		t.Fatalf("lines = %v", got)
	}
}

func TestRouterLogsReturnsStaleWhenDiskIsUnreadableWithoutMemory(t *testing.T) {
	dir := t.TempDir()
	manager := newWithDependencies(Config{Paths: managerpaths.Paths{DesktopLogFile: dir}}, dependencies{
		discoverStatus: func(context.Context) discovery.Result { return discovery.Result{Classification: discovery.Absent} },
		lifecycle:      &fakeLifecycle{},
	})

	if _, gotErr := manager.routerLogs(context.Background(), json.RawMessage(`{"limit":10}`)); gotErr == nil || gotErr.Code != protocol.CodeRouterStateStale {
		t.Fatalf("routerLogs() error = %+v", gotErr)
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
		discoverStatus: func(context.Context) discovery.Result {
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
