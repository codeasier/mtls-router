package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
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
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/version":
			_ = json.NewEncoder(w).Encode(map[string]any{"version": "fixture", "pid": os.Getpid(), "deployment_id": "deployment-test", "management_protocol_version": "2"})
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
			_, _ = w.Write([]byte(`{"data":[{"id":"model-a"}]}`))
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
	if len(lines) != 4 {
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
	if responses[3].ID == nil || *responses[3].ID != "last" || responses[3].Error != nil {
		t.Fatalf("last response = %#v", responses[3])
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

func TestServeOnlyCommandAndExplicitFlagValidation(t *testing.T) {
	binary := buildManager(t)
	for _, args := range [][]string{
		nil,
		{"manager.info"},
		{"serve", "--unknown"},
		{"serve", "--desktop-session", "session-only"},
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
	binary := buildManager(t)
	dataDir := t.TempDir()
	ready := filepath.Join(dataDir, "router-ready")
	requestLog := filepath.Join(dataDir, "router-requests")
	router := exec.Command(os.Args[0], "-test.run=^TestTrustedRouterHelperProcess$")
	router.Env = append(os.Environ(), "MTLS_TEST_ROUTER_HELPER=1", "MTLS_TEST_ROUTER_READY="+ready, "MTLS_TEST_ROUTER_REQUESTS="+requestLog)
	if err := router.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = router.Process.Kill(); _ = router.Wait() })
	authority := waitForContent(t, ready)
	identity, err := process.Inspect(router.Process.Pid)
	if err != nil {
		t.Fatal(err)
	}
	cliDir := filepath.Join(dataDir, "cli")
	if err := os.MkdirAll(cliDir, 0o700); err != nil {
		t.Fatal(err)
	}
	value := state.RouterState{
		PID: router.Process.Pid, Owner: "cli", ListenAddr: "http://" + authority,
		BinaryPath: identity.Executable, ProcessStartedAt: identity.StartedAt, ProcessExecutable: identity.Executable,
		RouterVersion: "fixture", DeploymentID: "deployment-test", ManagementProtocolVersion: "2",
	}
	if err := state.Write(filepath.Join(cliDir, "setup-state.json"), value); err != nil {
		t.Fatal(err)
	}

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
	if writeResponse.Error != nil {
		t.Fatalf("write error = %+v", writeResponse.Error)
	}
	_ = stdin.Close()
	if err := command.Wait(); err != nil {
		t.Fatalf("manager exit: %v, stderr=%s", err, stderr.String())
	}
	if strings.Contains(stderr.String(), subprocessKey) {
		t.Fatalf("stderr leaked key: %s", stderr.String())
	}
	configured, err := os.ReadFile(filepath.Join(dataDir, "claude", "settings.json"))
	if err != nil || !strings.Contains(string(configured), subprocessKey) || !strings.Contains(string(configured), "model-a") {
		t.Fatalf("Claude output was not written correctly: %q, %v", configured, err)
	}
	requests, err := os.ReadFile(requestLog)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(requests), "models\n"); got != 2 {
		t.Fatalf("authenticated catalog requests = %d, want discovery plus write refresh; log=%q", got, requests)
	}
	assertTreeExcludesExcept(t, dataDir, subprocessKey, filepath.Join(dataDir, "claude", "settings.json"))
}

func buildManager(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	binary := filepath.Join(dir, "mtls-router-manager")
	command := exec.Command("go", "build", "-ldflags", strings.Join([]string{
		"-X github.com/codeasier/mtls-router/internal/version.Version=v9.8.7-test",
		"-X github.com/codeasier/mtls-router/internal/version.Commit=abc123test",
		"-X github.com/codeasier/mtls-router/internal/version.BuildDate=2026-07-12T00:00:00Z",
		"-X github.com/codeasier/mtls-router/internal/version.DeploymentID=deployment-test",
	}, " "), "-o", binary, ".")
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
