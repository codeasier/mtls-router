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
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/lifecycle"
	"github.com/codeasier/mtls-router/internal/manager/occupant"
	managerpaths "github.com/codeasier/mtls-router/internal/manager/paths"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
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
	preview func(context.Context, []agent.Kind) (agent.Preview, error)
	write   func(context.Context, agent.WriteRequest) (agent.WriteResult, error)
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

func (f *fakeAgent) Write(ctx context.Context, request agent.WriteRequest) (agent.WriteResult, error) {
	return f.write(ctx, request)
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
			Version: "router-v1", PID: 91, DeploymentID: "prod-a", ManagementProtocolVersion: "1",
		},
		Health: discovery.Health{Status: "ok"},
		State: state.RouterState{
			PID: 91, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099", LogPath: logPath,
			RouterVersion: "router-v1", DeploymentID: "prod-a", ManagementProtocolVersion: "1",
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
			return protocol.ManagerInfoResult{Version: "manager-v1", Commit: "abc123", BuildDate: "2026-07-12T00:00:00Z", Target: "test/test", DeploymentID: "prod-a", ManagementProtocolVersion: "1"}
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
		`{"id":"10","method":"agent.preview","params":{"agents":["claude"]}}`,
		`{"id":"11","method":"agent.write","params":{"agents":["claude"],"revision_token":"revision","api_key":"` + integrationKey + `"}}`,
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
	wantIDs := []string{"1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"}
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
		`{"id":"5","method":"agent.write","params":{"agents":["claude","claude"],"revision_token":"r","api_key":"secret"}}`,
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

func TestOccupantHandlersExposeSafeResultAndSubmitOnlyToken(t *testing.T) {
	expiresAt := time.Date(2026, 7, 18, 1, 2, 33, 0, time.UTC)
	token := "opaque-confirmation"
	executable := filepath.Join(t.TempDir(), "listener")
	forceCalls := 0
	manager := newWithDependencies(Config{}, dependencies{occupant: &fakeOccupant{
		inspect: func(context.Context) (occupant.Inspection, error) {
			return occupant.Inspection{PID: 42, ProcessName: "listener", Executable: executable, ListenAddr: "127.0.0.1:19099", ConfirmationToken: token, ExpiresAt: expiresAt}, nil
		},
		forceTerminate: func(_ context.Context, got string) (occupant.Result, error) {
			forceCalls++
			if got != token {
				t.Fatalf("token = %q", got)
			}
			return occupant.Result{State: "absent"}, nil
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
	if forceCalls != 1 || !strings.Contains(output.String(), `"process_name":"listener"`) || !strings.Contains(output.String(), `"state":"absent"`) {
		t.Fatalf("force calls=%d output=%s", forceCalls, output.String())
	}
	if strings.Contains(output.String(), "started_at") || strings.Contains(output.String(), "user") || strings.Contains(output.String(), "socket") {
		t.Fatalf("inspection exposed internal identity: %s", output.String())
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

func TestMapOccupantErrorCoversStableCodes(t *testing.T) {
	tests := map[error]protocol.ErrorCode{
		occupant.ErrNotFound:            protocol.CodeOccupantNotFound,
		occupant.ErrNotOwned:            protocol.CodeOccupantNotOwned,
		occupant.ErrIdentityUnavailable: protocol.CodeOccupantIdentityUnavailable,
		occupant.ErrChanged:             protocol.CodeOccupantChanged,
		occupant.ErrProtected:           protocol.CodeOccupantProtected,
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
