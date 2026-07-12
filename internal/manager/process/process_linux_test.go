//go:build linux

package process

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestReplacedLinuxExecutableRemainsGenuine(t *testing.T) {
	dir := t.TempDir()
	binary := filepath.Join(dir, "fake-router")
	self, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(self)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(binary, data, 0o700); err != nil {
		t.Fatal(err)
	}
	cmd := helperCommand(binary)
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer cmd.Process.Kill()
	identity, err := Inspect(cmd.Process.Pid)
	if err != nil {
		t.Fatal(err)
	}
	replacement := binary + ".new"
	if err := os.WriteFile(replacement, data, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(replacement, binary); err != nil {
		t.Fatal(err)
	}
	status, err := Validate(identity, binary)
	if err != nil {
		t.Fatal(err)
	}
	if status != StatusGenuine {
		t.Fatalf("status after replacement = %q, want genuine", status)
	}
}

func helperCommand(binary string) *exec.Cmd {
	cmd := exec.Command(binary, "-test.run=TestProcessHelper")
	cmd.Env = append(os.Environ(), "MTLS_ROUTER_PROCESS_HELPER=1")
	return cmd
}
