package main

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

const subprocessKey = "sk-subprocess-sensitive-canary-4d8a"

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
	command.Env = append(os.Environ(),
		"MTLS_ROUTER_DESKTOP_DATA_DIR="+dataDir,
		"MTLS_ROUTER_STATE_DIR="+filepath.Join(dataDir, "cli"),
		"MTLS_ROUTER_LOG_PATH="+filepath.Join(dataDir, "cli", "router.log"),
		"CLAUDE_CONFIG_DIR="+filepath.Join(dataDir, "claude"),
		"OPENCODE_CONFIG="+filepath.Join(dataDir, "opencode.json"),
		"CODEX_HOME="+filepath.Join(dataDir, "codex"),
	)
	command.Stdin = strings.NewReader(input)
	var stdout, stderr bytes.Buffer
	command.Stdout = &stdout
	command.Stderr = &stderr
	err := command.Run()
	return stdout.String(), stderr.String(), err
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
