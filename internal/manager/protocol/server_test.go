package protocol

import (
	"bytes"
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestDeadlinesCoverEveryMethod(t *testing.T) {
	want := map[Method]time.Duration{
		MethodManagerInfo: time.Second, MethodDiagnosticsCollect: 5 * time.Second,
		MethodRouterStatus: time.Second, MethodRouterStart: 20 * time.Second,
		MethodRouterMigrateLegacy: 27 * time.Second, MethodRouterStop: 7 * time.Second, MethodRouterHealth: 12 * time.Second,
		MethodRouterVersion: time.Second, MethodRouterLogs: 2 * time.Second,
		MethodRouterInspectOccupant: 2 * time.Second, MethodRouterForceTerminateOccupant: 3 * time.Second,
		MethodAgentDetect: 5 * time.Second, MethodAgentModels: 30 * time.Second,
		MethodAgentRender: 5 * time.Second, MethodAgentPreview: 5 * time.Second,
		MethodAgentWrite: 30 * time.Second, MethodAgentCleanupPreview: 5 * time.Second,
		MethodAgentCleanupWrite: 30 * time.Second,
	}
	got := Deadlines()
	if len(got) != len(want) {
		t.Fatalf("deadline count = %d, want %d", len(got), len(want))
	}
	for method, duration := range want {
		if got[method] != duration {
			t.Errorf("deadline for %q = %s, want %s", method, got[method], duration)
		}
	}
}

func TestErrorDetailsAreBoundedAndValidationOnly(t *testing.T) {
	for _, test := range []struct {
		name string
		err  *Error
		want bool
	}{
		{name: "valid", err: &Error{Code: CodeModelConfigInvalid, Message: "invalid model config", Details: &ErrorDetails{Path: "/opencode/models/model-a", Rule: "protected_path"}}, want: true},
		{name: "non-validation", err: &Error{Code: CodeModelDiscoveryFailed, Message: "failed", Details: &ErrorDetails{Path: "/x", Rule: "invalid_value"}}},
		{name: "unbounded path", err: &Error{Code: CodeInvalidParams, Message: "invalid", Details: &ErrorDetails{Path: "/" + strings.Repeat("x", 1024), Rule: "invalid_value"}}},
		{name: "unstable rule", err: &Error{Code: CodeInvalidParams, Message: "invalid", Details: &ErrorDetails{Path: "/x", Rule: "contains secret"}}},
	} {
		t.Run(test.name, func(t *testing.T) {
			response := errorResponse(nil, test.err)
			if (response.Error.Details != nil) != test.want {
				t.Fatalf("details = %#v, want present=%t", response.Error.Details, test.want)
			}
		})
	}
}

func TestV2AgentParamsRejectUnknownFields(t *testing.T) {
	for name, target := range map[string]any{
		"models": &AgentModelsParams{}, "render": &AgentConfigParams{}, "write": &AgentWriteParams{},
	} {
		t.Run(name, func(t *testing.T) {
			if err := DecodeParams(json.RawMessage(`{"agents":["claude"],"unknown":true}`), target); err == nil || err.Code != CodeInvalidParams {
				t.Fatalf("error = %#v", err)
			}
		})
	}
}

func TestV2AgentParamsRejectMixedShapes(t *testing.T) {
	tests := []struct {
		name   string
		raw    string
		target any
	}{
		{name: "mixed preview", raw: `{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1},"api_key":"v1-key"}`, target: &AgentConfigParams{}},
		{name: "mixed write", raw: `{"agents":["claude"],"catalog_token":"catalog","model_config":{"version":1},"revision_token":"revision","approve_managed_overwrite":false,"approve_codex_auth_change":false,"api_key":"key","config":{"claude":{}}}`, target: &AgentWriteParams{}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := DecodeParams(json.RawMessage(test.raw), test.target); err == nil || err.Code != CodeInvalidParams {
				t.Fatalf("error = %#v", err)
			}
		})
	}
}

func TestAgentCleanupParamsAreStrictAndKeyFree(t *testing.T) {
	for _, test := range []struct {
		name   string
		raw    string
		target any
	}{
		{name: "preview agents", raw: `{"agent":"opencode","agents":["opencode"]}`, target: &AgentCleanupParams{}},
		{name: "preview api key", raw: `{"agent":"opencode","api_key":"secret"}`, target: &AgentCleanupParams{}},
		{name: "preview catalog", raw: `{"agent":"opencode","catalog_token":"catalog"}`, target: &AgentCleanupParams{}},
		{name: "preview model config", raw: `{"agent":"opencode","model_config":{}}`, target: &AgentCleanupParams{}},
		{name: "preview flow", raw: `{"agent":"opencode","flow_id":"flow"}`, target: &AgentCleanupParams{}},
		{name: "preview unknown", raw: `{"agent":"opencode","unknown":true}`, target: &AgentCleanupParams{}},
		{name: "write agents", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"agents":["opencode"]}`, target: &AgentCleanupWriteParams{}},
		{name: "write api key", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"api_key":"secret"}`, target: &AgentCleanupWriteParams{}},
		{name: "write catalog", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"catalog_token":"catalog"}`, target: &AgentCleanupWriteParams{}},
		{name: "write model config", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"model_config":{}}`, target: &AgentCleanupWriteParams{}},
		{name: "write flow", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"flow_id":"flow"}`, target: &AgentCleanupWriteParams{}},
		{name: "write unknown", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"unknown":true}`, target: &AgentCleanupWriteParams{}},
	} {
		t.Run(test.name, func(t *testing.T) {
			if err := DecodeParams(json.RawMessage(test.raw), test.target); err == nil || err.Code != CodeInvalidParams {
				t.Fatalf("error = %#v", err)
			}
		})
	}

	var preview AgentCleanupParams
	if err := DecodeParams(json.RawMessage(`{"agent":"opencode"}`), &preview); err != nil || preview.Agent != "opencode" {
		t.Fatalf("preview params = %#v, error = %#v", preview, err)
	}
	var write AgentCleanupWriteParams
	if err := DecodeParams(json.RawMessage(`{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false}`), &write); err != nil || write.Agent != "opencode" || write.RevisionToken != "revision" || write.ApproveManagedOverwrite == nil || *write.ApproveManagedOverwrite {
		t.Fatalf("write params = %#v, error = %#v", write, err)
	}
	var missingApproval AgentCleanupWriteParams
	if err := DecodeParams(json.RawMessage(`{"agent":"opencode","revision_token":"revision"}`), &missingApproval); err != nil || missingApproval.ApproveManagedOverwrite != nil {
		t.Fatalf("missing approval params = %#v, error = %#v", missingApproval, err)
	}
}

func TestAgentCleanupParamsRejectDuplicateKeys(t *testing.T) {
	for _, test := range []struct {
		name   string
		raw    string
		target any
	}{
		{name: "preview duplicate agent", raw: `{"agent":"claude","agent":"opencode"}`, target: &AgentCleanupParams{}},
		{name: "preview duplicate approval", raw: `{"agent":"opencode","approve_managed_overwrite":false,"approve_managed_overwrite":true}`, target: &AgentCleanupParams{}},
		{name: "write duplicate agent", raw: `{"agent":"claude","agent":"opencode","revision_token":"revision","approve_managed_overwrite":false}`, target: &AgentCleanupWriteParams{}},
		{name: "write duplicate approval", raw: `{"agent":"opencode","revision_token":"revision","approve_managed_overwrite":false,"approve_managed_overwrite":true}`, target: &AgentCleanupWriteParams{}},
	} {
		t.Run(test.name, func(t *testing.T) {
			if err := DecodeParams(json.RawMessage(test.raw), test.target); err == nil || err.Code != CodeInvalidParams {
				t.Fatalf("error = %#v", err)
			}
		})
	}
}

func TestDecodeParamsRejectsNestedDuplicateKeys(t *testing.T) {
	var params AgentConfigParams
	raw := json.RawMessage(`{"agents":["opencode"],"catalog_token":"catalog","model_config":{"version":1,"opencode":{"models":{"model-a":{"name":"first","name":"second"}}}}}`)
	if err := DecodeParams(raw, &params); err == nil || err.Code != CodeInvalidParams {
		t.Fatalf("error = %#v", err)
	}
}

func TestProtocolResultJSONExactShapes(t *testing.T) {
	expiresAt := time.Date(2026, 7, 25, 0, 0, 30, 0, time.UTC)
	tests := []struct {
		name  string
		value any
		keys  []string
		want  string
	}{
		{name: "forceable occupant", value: RouterOccupantInspectionResult{
			PID: 7, VerificationMode: "windows_pid_only", ListenAddr: "127.0.0.1:19099",
			Recovery: RouterOccupantRecovery{Action: "force_terminate"}, ConfirmationToken: "token", ExpiresAt: &expiresAt,
		}, want: `{"pid":7,"verification_mode":"windows_pid_only","listen_addr":"127.0.0.1:19099","recovery":{"action":"force_terminate"},"confirmation_token":"token","expires_at":"2026-07-25T00:00:30Z"}`},
		{name: "blocked occupant", value: RouterOccupantInspectionResult{
			PID: 8, VerificationMode: "windows_pid_only", ListenAddr: "127.0.0.1:19099",
			Recovery:   RouterOccupantRecovery{Action: "manual_stop_required", Reason: "service_managed"},
			Supervisor: &RouterOccupantSupervisor{Kind: "windows_service", Scope: "system", Identifiers: []string{"Svc"}},
		}, want: `{"pid":8,"verification_mode":"windows_pid_only","listen_addr":"127.0.0.1:19099","recovery":{"action":"manual_stop_required","reason":"service_managed"},"supervisor":{"kind":"windows_service","scope":"system","identifiers":["Svc"]}}`},
		{name: "occupant termination", value: RouterOccupantTerminationResult{Termination: "process_terminated", PortState: "released"}, want: `{"termination":"process_terminated","port_state":"released"}`},
		{name: "models", value: AgentModelsResult{}, keys: []string{"api_base_url", "catalog_token", "existing", "models", "preset", "router_base_url"}},
		{name: "models existing", value: AgentModelsExisting{}, keys: []string{"drifted_agents", "model_config", "unavailable_models"}},
		{name: "models preset", value: AgentModelsPreset{ModelConfig: json.RawMessage(`{}`), UnavailableAgents: map[string]AgentPresetUnavailable{}}, keys: []string{"model_config", "unavailable_agents"}},
		{name: "render", value: AgentRenderResult{}, keys: []string{"fragments", "model_config"}},
		{name: "preview", value: AgentPreviewResult{}, keys: []string{"drifted_agents", "files", "fragments", "managed_collisions", "managed_config_drift", "model_config", "requires_codex_auth_approval", "revision_token"}},
		{name: "cleanup state", value: AgentCleanupState{Managed: true, Available: false, Reason: "writes_disabled"}, want: `{"managed":true,"available":false,"reason":"writes_disabled"}`},
		{name: "cleanup preview", value: AgentCleanupPreviewResult{
			RevisionToken: "revision", Agent: "opencode", Files: []AgentFileEffect{{Path: "/config", Role: "config", Format: "json", Operation: "delete"}},
			RemovedPaths: []string{"model", "provider.mtls-router"}, ManagedConfigDrift: true,
			StateChange: &AgentFileEffect{Path: "/state", Role: "state", Format: "json", Operation: "delete"},
			StateBackup: &AgentFileEffect{Path: "/state", Role: "state", Format: "json", Operation: "backup"},
		}, want: `{"revision_token":"revision","agent":"opencode","files":[{"path":"/config","role":"config","format":"json","operation":"delete"}],"removed_paths":["model","provider.mtls-router"],"managed_config_drift":true,"state_change":{"path":"/state","role":"state","format":"json","operation":"delete"},"state_backup":{"path":"/state","role":"state","format":"json","operation":"backup"}}`},
		{name: "write", value: AgentWriteResult{}, keys: []string{"agents", "transaction_id"}},
		{name: "cleanup write paths only", value: AgentWriteResult{TransactionID: "transaction", Agents: []AgentWriteStatus{{Agent: "opencode", Success: true, Changed: []string{"/config"}, Backups: []string{"/config.bak"}}}, StateChange: &AgentFileEffect{Path: "/state", Role: "state", Format: "json", Operation: "delete"}, StateBackup: &AgentFileEffect{Path: "/state.bak", Role: "state", Format: "json", Operation: "backup"}}, want: `{"transaction_id":"transaction","agents":[{"agent":"opencode","success":true,"changed":["/config"],"backups":["/config.bak"]}],"state_change":{"path":"/state","role":"state","format":"json","operation":"delete"},"state_backup":{"path":"/state.bak","role":"state","format":"json","operation":"backup"}}`},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			encoded, err := json.Marshal(test.value)
			if err != nil {
				t.Fatal(err)
			}
			if test.want != "" {
				if string(encoded) != test.want {
					t.Fatalf("shape = %s, want %s", encoded, test.want)
				}
				return
			}
			var object map[string]json.RawMessage
			if err := json.Unmarshal(encoded, &object); err != nil {
				t.Fatal(err)
			}
			if len(object) != len(test.keys) {
				t.Fatalf("shape = %s, want keys %v", encoded, test.keys)
			}
			for _, key := range test.keys {
				if _, ok := object[key]; !ok {
					t.Fatalf("shape = %s, missing %q", encoded, key)
				}
			}
		})
	}
}

func TestServeRejectsRequestOverFourMiBWithProtocolResponse(t *testing.T) {
	server := NewServer(nil)
	input := strings.NewReader(strings.Repeat("x", maxRequestSize+1) + "\n")
	var output bytes.Buffer
	if err := server.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	var response Response
	if err := json.Unmarshal(bytes.TrimSpace(output.Bytes()), &response); err != nil {
		t.Fatalf("response is not protocol JSON: %q: %v", output.String(), err)
	}
	if response.ID != nil || response.Error == nil || response.Error.Code != CodeInvalidRequest {
		t.Fatalf("response = %#v", response)
	}
}

func TestServeCorrelatesSequentialRequestsAndKeepsStdoutPure(t *testing.T) {
	var calls []string
	server := NewServer(map[Method]Handler{
		MethodManagerInfo: func(_ context.Context, params json.RawMessage) (any, *Error) {
			calls = append(calls, string(params))
			return ManagerInfoResult{Version: "v1", Target: "test"}, nil
		},
	})
	input := strings.NewReader("{\"id\":\"first\",\"method\":\"manager.info\"}\n{\"id\":\"second\",\"method\":\"manager.info\",\"params\":{}}\n")
	var output bytes.Buffer
	if err := server.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	if len(calls) != 2 || calls[0] != "{}" || calls[1] != "{}" {
		t.Fatalf("calls = %#v", calls)
	}

	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("response lines = %d, output %q", len(lines), output.String())
	}
	for index, line := range lines {
		var response Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatalf("response %d is not JSON: %v", index, err)
		}
		wantID := []string{"first", "second"}[index]
		if response.ID == nil || *response.ID != wantID || response.Error != nil {
			t.Fatalf("response %d = %#v", index, response)
		}
	}
}

func TestServeMalformedAndMissingIDsUseNullID(t *testing.T) {
	server := NewServer(nil)
	input := strings.NewReader("not-json\n{\"method\":\"manager.info\"}\n{\"id\":42,\"method\":\"manager.info\"}\n")
	var output bytes.Buffer
	if err := server.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(strings.TrimSpace(output.String()), "\n") {
		var response Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatal(err)
		}
		if response.ID != nil || response.Error == nil || response.Error.Code != CodeInvalidRequest {
			t.Fatalf("response = %#v", response)
		}
	}
}

func TestServeUnknownMethodPreservesValidID(t *testing.T) {
	server := NewServer(nil)
	var output bytes.Buffer
	if err := server.Serve(context.Background(), strings.NewReader("{\"id\":\"request-1\",\"method\":\"other\"}\n"), &output); err != nil {
		t.Fatal(err)
	}
	var response Response
	if err := json.Unmarshal(output.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if response.ID == nil || *response.ID != "request-1" || response.Error == nil || response.Error.Code != CodeUnknownMethod {
		t.Fatalf("response = %#v", response)
	}
}

func TestServeReturnsStableHandlerError(t *testing.T) {
	server := NewServer(map[Method]Handler{
		MethodRouterStart: func(context.Context, json.RawMessage) (any, *Error) {
			return nil, &Error{Code: CodePortOccupied, Message: "local port is occupied"}
		},
	})
	var output bytes.Buffer
	if err := server.Serve(context.Background(), strings.NewReader("{\"id\":\"x\",\"method\":\"router.start\",\"params\":{\"owner\":\"desktop\"}}\n"), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), `"code":"PORT_OCCUPIED"`) {
		t.Fatalf("output = %q", output.String())
	}
}

func TestServeAppliesMethodDeadline(t *testing.T) {
	server := NewServer(map[Method]Handler{
		MethodRouterStatus: func(ctx context.Context, _ json.RawMessage) (any, *Error) {
			<-ctx.Done()
			return nil, nil
		},
	})
	server.Deadlines[MethodRouterStatus] = 10 * time.Millisecond
	var output bytes.Buffer
	if err := server.Serve(context.Background(), strings.NewReader("{\"id\":\"x\",\"method\":\"router.status\"}\n"), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), `"code":"OPERATION_TIMEOUT"`) {
		t.Fatalf("output = %q", output.String())
	}
}

func TestDecodeParamsPreservesUnknownAndTrailingRejection(t *testing.T) {
	for _, raw := range []string{
		`{"owner":"desktop","extra":true}`,
		`{"owner":"desktop"} {"owner":"cli"}`,
	} {
		var params RouterStartParams
		if err := DecodeParams(json.RawMessage(raw), &params); err == nil || err.Code != CodeInvalidParams {
			t.Fatalf("DecodeParams(%s) error = %#v", raw, err)
		}
	}
}

func TestForceTerminateOccupantParamsRemainStrictlyTokenOnly(t *testing.T) {
	for _, raw := range []string{
		`{"confirmation_token":"token","pid":4242}`,
		`{"confirmation_token":"token","executable":"listener.exe"}`,
		`{"confirmation_token":"token","confirmed":true}`,
	} {
		var params RouterForceTerminateOccupantParams
		err := DecodeParams(json.RawMessage(raw), &params)
		if err == nil || err.Code != CodeInvalidParams {
			t.Errorf("DecodeParams(%s) error = %#v", raw, err)
		}
	}

	var params RouterForceTerminateOccupantParams
	if err := DecodeParams(json.RawMessage(`{"confirmation_token":"token"}`), &params); err != nil {
		t.Fatalf("token-only request rejected: %#v", err)
	}
	if params.ConfirmationToken != "token" {
		t.Fatalf("confirmation token = %q", params.ConfirmationToken)
	}
}
