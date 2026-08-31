package main

import (
	"bufio"
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

const subprocessKey = "sk-subprocess-sensitive-canary-4d8a"

func TestTrustedRouterHelperProcess(t *testing.T) {
	if os.Getenv("MTLS_TEST_ROUTER_HELPER") == "" {
		return
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		os.Exit(2)
	}
	ready := os.Getenv("MTLS_TEST_ROUTER_READY")
	requests := os.Getenv("MTLS_TEST_ROUTER_REQUESTS")
	catalogMode := os.Getenv("MTLS_TEST_ROUTER_CATALOG_MODE")
	var modelRequests atomic.Int64
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/version":
			_ = json.NewEncoder(w).Encode(map[string]any{"version": "fixture", "pid": os.Getpid(), "deployment_id": "deployment-test", "management_protocol_version": "4"})
		case "/health":
			_ = json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
		case "/v1/models":
			if r.Header.Get("Authorization") != "Bearer "+subprocessKey || r.URL.RawQuery != "" {
				w.WriteHeader(http.StatusUnauthorized)
				return
			}
			file, openErr := os.OpenFile(requests, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
			if openErr != nil {
				w.WriteHeader(http.StatusInternalServerError)
				return
			}
			_, _ = file.WriteString("models\n")
			_ = file.Close()
			requestNumber := modelRequests.Add(1)
			switch catalogMode {
			case "", "mixed":
				_, _ = w.Write([]byte(`{"data":[{"id":"provider/slash"},{"id":"model-a"}]}`))
			case "slash-only":
				_, _ = w.Write([]byte(`{"data":[{"id":"provider/slash"}]}`))
			case "safe-then-slash":
				if requestNumber == 1 {
					_, _ = w.Write([]byte(`{"data":[{"id":"model-a"}]}`))
				} else {
					_, _ = w.Write([]byte(`{"data":[{"id":"provider/slash"}]}`))
				}
			default:
				w.WriteHeader(http.StatusInternalServerError)
			}
		default:
			http.NotFound(w, r)
		}
	})
	server := &http.Server{Handler: handler}
	if err := os.WriteFile(ready, []byte(listener.Addr().String()), 0o600); err != nil {
		os.Exit(3)
	}
	if err := server.Serve(listener); err != nil && err != http.ErrServerClosed {
		os.Exit(4)
	}
	os.Exit(0)
}

func TestServeSubprocessContract(t *testing.T) {
	binary := buildManager(t)
	dataDir := t.TempDir()
	input := strings.Join([]string{
		`{"id":"first","method":"manager.info"}`,
		`{"api_key":"` + subprocessKey,
		`{"id":"sensitive","method":"agent.write","params":{"agents":["claude"],"revision_token":"missing","api_key":"` + subprocessKey + `"}}`,
		`{"id":"cleanup-preview-sensitive","method":"agent.cleanup.preview","params":{"agent":"opencode","api_key":"` + subprocessKey + `"}}`,
		`{"id":"cleanup-write-sensitive","method":"agent.cleanup.write","params":{"agent":"opencode","revision_token":"` + subprocessKey + `","approve_managed_overwrite":false}}`,
		`{"id":"last","method":"manager.info","params":{}}`,
	}, "\n") + "\n"
	stdout, stderr, err := runManager(binary, dataDir, input)
	if err != nil {
		t.Fatalf("serve failed: %v, stderr=%s", err, stderr)
	}
	if strings.Contains(stdout, subprocessKey) || strings.Contains(stderr, subprocessKey) {
		t.Fatalf("sensitive input leaked: stdout=%s stderr=%s", stdout, stderr)
	}
	lines := strings.Split(strings.TrimSpace(stdout), "\n")
	if len(lines) != 6 {
		t.Fatalf("response lines = %d: %s", len(lines), stdout)
	}
	var responses []protocol.Response
	for _, line := range lines {
		var response protocol.Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatalf("non-JSON protocol line %q: %v", line, err)
		}
		responses = append(responses, response)
	}
	if responses[0].ID == nil || *responses[0].ID != "first" || responses[0].Error != nil {
		t.Fatalf("first response = %#v", responses[0])
	}
	if responses[1].ID != nil || responses[1].Error == nil || responses[1].Error.Code != protocol.CodeInvalidRequest {
		t.Fatalf("malformed response = %#v", responses[1])
	}
	if responses[2].ID == nil || *responses[2].ID != "sensitive" || responses[2].Error == nil {
		t.Fatalf("sensitive response = %#v", responses[2])
	}
	if responses[3].ID == nil || *responses[3].ID != "cleanup-preview-sensitive" || responses[3].Error == nil || responses[3].Error.Code != protocol.CodeInvalidParams {
		t.Fatalf("cleanup preview response = %#v", responses[3])
	}
	if responses[4].ID == nil || *responses[4].ID != "cleanup-write-sensitive" || responses[4].Error == nil || responses[4].Error.Code != protocol.CodeAgentNotManaged {
		t.Fatalf("cleanup write response = %#v", responses[4])
	}
	if responses[5].ID == nil || *responses[5].ID != "last" || responses[5].Error != nil {
		t.Fatalf("last response = %#v", responses[5])
	}

	var info protocol.ManagerInfoResult
	if err := json.Unmarshal(responses[0].Result, &info); err != nil {
		t.Fatal(err)
	}
	if info.Version != "v9.8.7-test" || info.Commit != "abc123test" || info.BuildDate != "2026-07-12T00:00:00Z" || info.DeploymentID != "deployment-test" || info.ManagementProtocolVersion == "" || !strings.Contains(info.Target, "/") {
		t.Fatalf("manager info = %+v", info)
	}
	assertTreeExcludes(t, dataDir, subprocessKey)
}

func TestManagerSubprocessCleanupNeedsNoRouterOrAPIKey(t *testing.T) {
	binary := buildManager(t)
	dataDir := t.TempDir()
	authority, requestLog := startTrustedRouter(t, dataDir, "")
	openCodePath := filepath.Join(dataDir, "opencode.json")
	if err := os.WriteFile(openCodePath, []byte(`{"theme":"keep"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	send, finish := startManagerSession(t, binary, dataDir, authority)

	modelsResponse := send(map[string]any{"id": "models", "method": "agent.models", "params": map[string]any{"owner": "cli", "agents": []string{"opencode"}, "api_key": subprocessKey}})
	if modelsResponse.Error != nil {
		t.Fatalf("models error = %+v", modelsResponse.Error)
	}
	var models protocol.AgentModelsResult
	if err := json.Unmarshal(modelsResponse.Result, &models); err != nil {
		t.Fatal(err)
	}
	config := json.RawMessage(`{"version":1,"opencode":{"default_model":"model-a","models":{"model-a":{}}}}`)
	previewResponse := send(map[string]any{"id": "preview", "method": "agent.preview", "params": map[string]any{"agents": []string{"opencode"}, "catalog_token": models.CatalogToken, "model_config": config}})
	if previewResponse.Error != nil {
		t.Fatalf("preview error = %+v", previewResponse.Error)
	}
	var preview protocol.AgentPreviewResult
	if err := json.Unmarshal(previewResponse.Result, &preview); err != nil {
		t.Fatal(err)
	}
	writeResponse := send(map[string]any{"id": "write", "method": "agent.write", "params": map[string]any{
		"agents": []string{"opencode"}, "catalog_token": models.CatalogToken, "model_config": config, "revision_token": preview.RevisionToken,
		"approve_managed_overwrite": preview.ManagedConfigDrift, "approve_codex_auth_change": preview.RequiresCodexAuthApproval, "api_key": subprocessKey,
	}})
	if writeResponse.Error != nil {
		t.Fatalf("write error = %+v", writeResponse.Error)
	}
	stopTrustedRouter(t, dataDir, authority)

	cleanupPreviewResponse := send(map[string]any{"id": "cleanup-preview", "method": "agent.cleanup.preview", "params": map[string]any{"agent": "opencode"}})
	if cleanupPreviewResponse.Error != nil {
		t.Fatalf("cleanup preview error = %+v", cleanupPreviewResponse.Error)
	}
	var cleanupPreview protocol.AgentCleanupPreviewResult
	if err := json.Unmarshal(cleanupPreviewResponse.Result, &cleanupPreview); err != nil {
		t.Fatal(err)
	}
	if cleanupPreview.Agent != "opencode" || cleanupPreview.RevisionToken == "" || len(cleanupPreview.Files) != 1 {
		t.Fatalf("cleanup preview = %#v", cleanupPreview)
	}
	cleanupWriteResponse := send(map[string]any{"id": "cleanup-write", "method": "agent.cleanup.write", "params": map[string]any{
		"agent": "opencode", "revision_token": cleanupPreview.RevisionToken, "approve_managed_overwrite": cleanupPreview.ManagedConfigDrift,
	}})
	if cleanupWriteResponse.Error != nil {
		t.Fatalf("cleanup write error = %+v", cleanupWriteResponse.Error)
	}
	finish()

	cleaned, err := os.ReadFile(openCodePath)
	if err != nil || !strings.Contains(string(cleaned), `"theme":"keep"`) || strings.Contains(string(cleaned), "mtls-router") || strings.Contains(string(cleaned), subprocessKey) {
		t.Fatalf("cleaned OpenCode config = %q, %v", cleaned, err)
	}
	sidecar := filepath.Join(dataDir, "agent-transactions", "last-applied-model-config.json")
	if _, err := os.Stat(sidecar); !os.IsNotExist(err) {
		t.Fatalf("final sidecar still exists: %v", err)
	}
	assertModelRequestCount(t, requestLog, 2)
	assertTreeExcludes(t, filepath.Join(dataDir, "agent-transactions"), subprocessKey)
}

func TestServeOnlyCommandAndExplicitFlagValidation(t *testing.T) {
	binary := buildManager(t)
	for _, args := range [][]string{
		nil,
		{"manager.info"},
		{"serve", "--unknown"},
		{"serve", "--desktop-session", "session-only"},
		{"serve", "--desktop-session", "session", "--parent-pid", "1", "--parent-start", "start", "--parent-executable", "/bin/true"},
		{"serve", "--listen", "0.0.0.0:19099"},
	} {
		command := exec.Command(binary, args...)
		var stdout, stderr bytes.Buffer
		command.Stdout = &stdout
		command.Stderr = &stderr
		if err := command.Run(); err == nil {
			t.Fatalf("args %#v unexpectedly succeeded", args)
		}
		if stdout.Len() != 0 {
			t.Fatalf("args %#v wrote non-protocol stdout: %s", args, stdout.String())
		}
	}
}

func TestManagerSubprocessModelsPreviewRefreshWriteAcrossRequests(t *testing.T) {
	for _, test := range []struct {
		name, catalogMode, model, simplify string
		wantSimplify                       bool
	}{
		{name: "default filters mixed catalog", model: "model-a", wantSimplify: true},
		{name: "mixed-case false retains slash-only catalog", catalogMode: "slash-only", model: "provider/slash", simplify: "fAlSe"},
	} {
		t.Run(test.name, func(t *testing.T) {
			var binary string
			if test.simplify != "" {
				binary = buildManagerWithSimplify(t, test.simplify)
			} else {
				binary = buildManager(t)
			}
			dataDir := t.TempDir()
			authority, requestLog := startTrustedRouter(t, dataDir, test.catalogMode)
			send, finish := startManagerSession(t, binary, dataDir, authority)

			modelsResponse := send(map[string]any{"id": "models", "method": "agent.models", "params": map[string]any{"owner": "cli", "agents": []string{"claude"}, "api_key": subprocessKey}})
			if modelsResponse.Error != nil {
				t.Fatalf("models error = %+v", modelsResponse.Error)
			}
			var models protocol.AgentModelsResult
			if err := json.Unmarshal(modelsResponse.Result, &models); err != nil {
				t.Fatal(err)
			}
			if got := strings.Join(models.Models, ","); got != test.model {
				t.Fatalf("models = %q, want %q", got, test.model)
			}
			tokenModels, tokenSimplify := catalogTokenClaims(t, models.CatalogToken)
			if got := strings.Join(tokenModels, ","); got != test.model {
				t.Fatalf("token models = %q, want %q", got, test.model)
			}
			if tokenSimplify != test.wantSimplify {
				t.Fatalf("token simplify = %t, want %t", tokenSimplify, test.wantSimplify)
			}
			config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"` + test.model + `"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
			previewResponse := send(map[string]any{"id": "preview", "method": "agent.preview", "params": map[string]any{"agents": []string{"claude"}, "catalog_token": models.CatalogToken, "model_config": config}})
			if previewResponse.Error != nil {
				t.Fatalf("preview error = %+v", previewResponse.Error)
			}
			var preview protocol.AgentPreviewResult
			if err := json.Unmarshal(previewResponse.Result, &preview); err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(string(preview.ModelConfig), test.model) {
				t.Fatalf("preview model config = %s, want %q", preview.ModelConfig, test.model)
			}
			writeResponse := send(map[string]any{"id": "write", "method": "agent.write", "params": map[string]any{
				"agents": []string{"claude"}, "catalog_token": models.CatalogToken, "model_config": config, "revision_token": preview.RevisionToken,
				"approve_managed_overwrite": preview.ManagedConfigDrift, "approve_codex_auth_change": preview.RequiresCodexAuthApproval, "api_key": subprocessKey,
			}})
			if writeResponse.Error != nil {
				t.Fatalf("write error = %+v", writeResponse.Error)
			}
			finish()

			configured, err := os.ReadFile(filepath.Join(dataDir, "claude", "settings.json"))
			if err != nil || !strings.Contains(string(configured), subprocessKey) || !strings.Contains(string(configured), test.model) {
				t.Fatalf("Claude output was not written correctly: %q, %v", configured, err)
			}
			assertModelRequestCount(t, requestLog, 2)
			assertTreeExcludesExcept(t, dataDir, subprocessKey, filepath.Join(dataDir, "claude", "settings.json"))
		})
	}
}

func TestManagerSubprocessWriteRefreshRejectsCatalogFilteredToEmpty(t *testing.T) {
	binary := buildManager(t)
	dataDir := t.TempDir()
	authority, requestLog := startTrustedRouter(t, dataDir, "safe-then-slash")
	send, finish := startManagerSession(t, binary, dataDir, authority)

	modelsResponse := send(map[string]any{"id": "models", "method": "agent.models", "params": map[string]any{"owner": "cli", "agents": []string{"claude"}, "api_key": subprocessKey}})
	if modelsResponse.Error != nil {
		t.Fatalf("models error = %+v", modelsResponse.Error)
	}
	var models protocol.AgentModelsResult
	if err := json.Unmarshal(modelsResponse.Result, &models); err != nil {
		t.Fatal(err)
	}
	config := json.RawMessage(`{"version":1,"claude":{"primary":{"model":"model-a"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`)
	previewResponse := send(map[string]any{"id": "preview", "method": "agent.preview", "params": map[string]any{"agents": []string{"claude"}, "catalog_token": models.CatalogToken, "model_config": config}})
	if previewResponse.Error != nil {
		t.Fatalf("preview error = %+v", previewResponse.Error)
	}
	var preview protocol.AgentPreviewResult
	if err := json.Unmarshal(previewResponse.Result, &preview); err != nil {
		t.Fatal(err)
	}
	writeResponse := send(map[string]any{"id": "write", "method": "agent.write", "params": map[string]any{
		"agents": []string{"claude"}, "catalog_token": models.CatalogToken, "model_config": config, "revision_token": preview.RevisionToken,
		"approve_managed_overwrite": preview.ManagedConfigDrift, "approve_codex_auth_change": preview.RequiresCodexAuthApproval, "api_key": subprocessKey,
	}})
	if writeResponse.Error == nil || writeResponse.Error.Code != protocol.CodeModelCatalogEmpty {
		t.Fatalf("write response = %#v", writeResponse)
	}
	finish()
	assertModelRequestCount(t, requestLog, 2)
	for _, path := range []string{
		filepath.Join(dataDir, "claude", "settings.json"),
		filepath.Join(dataDir, "agent-transactions", "agent-write-journal.json"),
		filepath.Join(dataDir, "agent-transactions", "last-applied-model-config.json"),
	} {
		if _, err := os.Stat(path); !os.IsNotExist(err) {
			t.Fatalf("failed refresh created %s: %v", path, err)
		}
	}
	entries, err := os.ReadDir(filepath.Join(dataDir, "agent-transactions"))
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if entry.Name() != "agent-operation.lock" && entry.Name() != "token-signing-key.json" {
			t.Fatalf("failed refresh left transaction artifact %s", entry.Name())
		}
	}
	assertTreeExcludes(t, dataDir, subprocessKey)
}

func TestManagerSubprocessReturnsPresetWithoutCanaryLeaks(t *testing.T) {
	const presetCanary = "preset-subprocess-canary"
	presetJSON := `{"version":1,"claude":{"primary":{"model":"model-a","name":"` + presetCanary + `"},"haiku":{"inherit_primary":true},"sonnet":{"inherit_primary":true},"opus":{"inherit_primary":true}}}`
	binary := buildManagerWithPreset(t, base64.StdEncoding.EncodeToString([]byte(presetJSON)))
	dataDir := t.TempDir()
	authority, _ := startTrustedRouter(t, dataDir, "")
	input := `{"id":"models","method":"agent.models","params":{"owner":"cli","agents":["claude"],"api_key":"` + subprocessKey + `"}}` + "\n"
	command := exec.Command(binary, "serve", "--listen", authority)
	command.Env = managerEnv(dataDir)
	command.Stdin = strings.NewReader(input)
	var stdout, stderr bytes.Buffer
	command.Stdout, command.Stderr = &stdout, &stderr
	if err := command.Run(); err != nil {
		t.Fatalf("manager failed: %v stderr=%s", err, stderr.String())
	}
	if strings.Contains(stdout.String(), subprocessKey) || strings.Contains(stderr.String(), subprocessKey) || strings.Contains(stderr.String(), presetCanary) {
		t.Fatalf("canary leaked: stdout=%s stderr=%s", stdout.String(), stderr.String())
	}
	var response protocol.Response
	if err := json.Unmarshal(bytes.TrimSpace(stdout.Bytes()), &response); err != nil {
		t.Fatal(err)
	}
	var result protocol.AgentModelsResult
	if response.Error != nil || json.Unmarshal(response.Result, &result) != nil || !strings.Contains(string(result.Preset.ModelConfig), presetCanary) {
		t.Fatalf("preset response = %#v stdout=%s", response, stdout.String())
	}
	assertTreeExcludes(t, dataDir, presetCanary)
	assertTreeExcludes(t, dataDir, subprocessKey)
}

func TestManagerSubprocessRejectsMalformedPresetBeforeServing(t *testing.T) {
	binary := buildManagerWithPreset(t, "malformed-encoded-preset-canary%%")
	stdout, stderr, err := runManager(binary, t.TempDir(), `{"id":"info","method":"manager.info"}`+"\n")
	if err == nil || stdout != "" {
		t.Fatalf("err=%v stdout=%q stderr=%q", err, stdout, stderr)
	}
	if stderr != "{\"schema_version\":1,\"kind\":\"manager_bootstrap_failure\",\"stage\":\"handshake\",\"code\":\"MANAGER_INIT_FAILED\"}\n" || strings.Contains(stderr, "malformed-encoded-preset-canary") {
		t.Fatalf("unsanitized startup failure: %s", stderr)
	}
}

func TestManagerSubprocessRejectsInvalidLinkedSimplifyBeforeRecoveryOrServing(t *testing.T) {
	const simplifyCanary = "invalid-linked-simplify-canary-6f2d"
	binary := buildManagerWithSimplify(t, simplifyCanary)
	dataDir := t.TempDir()
	stateDir := filepath.Join(dataDir, "agent-transactions")
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(stateDir, "agent-write-journal.json"), []byte(`invalid-recovery-canary`), 0o600); err != nil {
		t.Fatal(err)
	}

	stdout, stderr, err := runManager(binary, dataDir, `{"id":"info","method":"manager.info"}`+"\n")
	if err == nil || stdout != "" {
		t.Fatalf("err=%v stdout=%q stderr=%q", err, stdout, stderr)
	}
	if stderr != "{\"schema_version\":1,\"kind\":\"manager_bootstrap_failure\",\"stage\":\"handshake\",\"code\":\"MANAGER_CONFIG_INVALID\"}\n" || strings.Contains(stderr, simplifyCanary) || strings.Contains(stderr, "Agent transaction recovery") {
		t.Fatalf("unsanitized or late startup failure: %s", stderr)
	}
}

func buildManager(t *testing.T) string {
	return buildManagerWithPreset(t, "")
}

func buildManagerWithPreset(t *testing.T, encodedPreset string) string {
	return buildManagerWithLinkedValues(t, encodedPreset, nil)
}

func buildManagerWithSimplify(t *testing.T, simplify string) string {
	return buildManagerWithLinkedValues(t, "", &simplify)
}

func buildManagerWithLinkedValues(t *testing.T, encodedPreset string, simplify *string) string {
	t.Helper()
	dir := t.TempDir()
	binary := filepath.Join(dir, "mtls-router-manager")
	linkedValues := []string{
		"-X github.com/codeasier/mtls-router/internal/version.Version=v9.8.7-test",
		"-X github.com/codeasier/mtls-router/internal/version.Commit=abc123test",
		"-X github.com/codeasier/mtls-router/internal/version.BuildDate=2026-07-12T00:00:00Z",
		"-X github.com/codeasier/mtls-router/internal/version.DeploymentID=deployment-test",
		"-X github.com/codeasier/mtls-router/internal/manager/preset.Encoded=" + encodedPreset,
	}
	if simplify != nil {
		linkedValues = append(linkedValues, "-X github.com/codeasier/mtls-router/internal/manager/modelcatalog.Simplify="+*simplify)
	}
	command := exec.Command("go", "build", "-ldflags", strings.Join(linkedValues, " "), "-o", binary, ".")
	command.Dir = "."
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("build manager: %v\n%s", err, output)
	}
	return binary
}

func runManager(binary, dataDir, input string) (string, string, error) {
	command := exec.Command(binary, "serve")
	command.Env = managerEnv(dataDir)
	command.Stdin = strings.NewReader(input)
	var stdout, stderr bytes.Buffer
	command.Stdout = &stdout
	command.Stderr = &stderr
	err := command.Run()
	return stdout.String(), stderr.String(), err
}

func managerEnv(dataDir string) []string {
	return append(os.Environ(),
		"MTLS_ROUTER_DESKTOP_DATA_DIR="+dataDir,
		"MTLS_ROUTER_STATE_DIR="+filepath.Join(dataDir, "cli"),
		"MTLS_ROUTER_LOG_PATH="+filepath.Join(dataDir, "cli", "router.log"),
		"CLAUDE_CONFIG_DIR="+filepath.Join(dataDir, "claude"),
		"OPENCODE_CONFIG="+filepath.Join(dataDir, "opencode.json"),
		"CODEX_HOME="+filepath.Join(dataDir, "codex"),
	)
}

func startTrustedRouter(t *testing.T, dataDir, catalogMode string) (string, string) {
	t.Helper()
	ready := filepath.Join(dataDir, "router-ready")
	requestLog := filepath.Join(dataDir, "router-requests")
	router := exec.Command(os.Args[0], "-test.run=^TestTrustedRouterHelperProcess$")
	router.Env = append(os.Environ(),
		"MTLS_TEST_ROUTER_HELPER=1",
		"MTLS_TEST_ROUTER_READY="+ready,
		"MTLS_TEST_ROUTER_REQUESTS="+requestLog,
		"MTLS_TEST_ROUTER_CATALOG_MODE="+catalogMode,
	)
	if err := router.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = router.Process.Kill()
		_ = router.Wait()
	})
	authority := waitForContent(t, ready)
	identity, err := process.Inspect(router.Process.Pid)
	if err != nil {
		t.Fatal(err)
	}
	cliDir := filepath.Join(dataDir, "cli")
	if err := os.MkdirAll(cliDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := state.Write(filepath.Join(cliDir, "setup-state.json"), state.RouterState{
		PID: router.Process.Pid, Owner: "cli", ListenAddr: "http://" + authority,
		BinaryPath: identity.Executable, ProcessStartedAt: identity.StartedAt, ProcessExecutable: identity.Executable,
		RouterVersion: "fixture", DeploymentID: "deployment-test", ManagementProtocolVersion: "4",
	}); err != nil {
		t.Fatal(err)
	}
	return authority, requestLog
}

func stopTrustedRouter(t *testing.T, dataDir, authority string) {
	t.Helper()
	value, err := state.Read(filepath.Join(dataDir, "cli", "setup-state.json"))
	if err != nil {
		t.Fatal(err)
	}
	process, err := os.FindProcess(value.PID)
	if err != nil {
		t.Fatal(err)
	}
	if err := process.Kill(); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		connection, dialErr := net.DialTimeout("tcp", authority, 50*time.Millisecond)
		if dialErr != nil {
			return
		}
		_ = connection.Close()
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("trusted router %s did not stop", authority)
}

func startManagerSession(t *testing.T, binary, dataDir, authority string) (func(any) protocol.Response, func()) {
	t.Helper()
	command := exec.Command(binary, "serve", "--listen", authority)
	command.Env = managerEnv(dataDir)
	stdin, err := command.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := command.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	var stderr bytes.Buffer
	command.Stderr = &stderr
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	finished := false
	t.Cleanup(func() {
		if !finished {
			_ = command.Process.Kill()
			_ = command.Wait()
		}
	})
	reader := bufio.NewReader(stdout)
	send := func(request any) protocol.Response {
		t.Helper()
		if err := json.NewEncoder(stdin).Encode(request); err != nil {
			t.Fatal(err)
		}
		line, err := reader.ReadString('\n')
		if err != nil {
			t.Fatalf("read manager response: %v, stderr=%s", err, stderr.String())
		}
		var response protocol.Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatalf("response %q: %v", line, err)
		}
		if strings.Contains(line, subprocessKey) {
			t.Fatalf("response leaked key: %s", line)
		}
		return response
	}
	finish := func() {
		t.Helper()
		if finished {
			return
		}
		_ = stdin.Close()
		if err := command.Wait(); err != nil {
			t.Fatalf("manager exit: %v, stderr=%s", err, stderr.String())
		}
		finished = true
		if strings.Contains(stderr.String(), subprocessKey) {
			t.Fatalf("stderr leaked key: %s", stderr.String())
		}
	}
	return send, finish
}

func catalogTokenClaims(t *testing.T, token string) ([]string, bool) {
	t.Helper()
	parts := strings.SplitN(token, ".", 2)
	if len(parts) != 2 {
		t.Fatalf("invalid catalog token shape")
	}
	payload, err := base64.RawURLEncoding.Strict().DecodeString(parts[0])
	if err != nil {
		t.Fatal(err)
	}
	var claims struct {
		Models   []string `json:"models"`
		Simplify bool     `json:"simplify"`
	}
	if err := json.Unmarshal(payload, &claims); err != nil {
		t.Fatal(err)
	}
	return claims.Models, claims.Simplify
}

func assertModelRequestCount(t *testing.T, path string, want int) {
	t.Helper()
	requests, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(requests), "models\n"); got != want {
		t.Fatalf("authenticated catalog requests = %d, want %d; log=%q", got, want, requests)
	}
}

func waitForContent(t *testing.T, path string) string {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		content, err := os.ReadFile(path)
		if err == nil && strings.TrimSpace(string(content)) != "" {
			return strings.TrimSpace(string(content))
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", path)
	return ""
}

func assertTreeExcludesExcept(t *testing.T, root, canary, allowed string) {
	t.Helper()
	allowed = filepath.Clean(allowed)
	err := filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() || filepath.Clean(path) == allowed {
			return nil
		}
		content, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if strings.Contains(string(content), canary) {
			return fmt.Errorf("sensitive input persisted in %s", path)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func assertTreeExcludes(t *testing.T, root, canary string) {
	t.Helper()
	err := filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() {
			return nil
		}
		content, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if strings.Contains(string(content), canary) {
			t.Fatalf("sensitive input persisted in %s", path)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
