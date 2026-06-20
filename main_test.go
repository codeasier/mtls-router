package main

import (
	"bytes"
	"io"
	"os"
	"testing"
)

func TestSetupPowerShellScriptHasUtf8Bom(t *testing.T) {
	data, err := os.ReadFile("setup.ps1")
	if err != nil {
		t.Fatal(err)
	}
	bom := []byte{0xEF, 0xBB, 0xBF}
	if !bytes.HasPrefix(data, bom) {
		t.Fatal("setup.ps1 must include a UTF-8 BOM for Windows PowerShell 5.1")
	}
}

func TestHandleMetaFlagsIgnoresConfigFlags(t *testing.T) {
	output := captureStdout(t, func() {
		handled, err := handleMetaFlags([]string{"-debug"})
		if err != nil {
			t.Fatal(err)
		}
		if handled {
			t.Fatal("config flag should not be handled as a meta flag")
		}
	})
	if output != "" {
		t.Fatalf("unexpected output for config flag: %q", output)
	}
}

func TestHandleMetaFlagsIgnoresBackendAndLogFlags(t *testing.T) {
	output := captureStdout(t, func() {
		handled, err := handleMetaFlags([]string{"--backend", "--log", "/tmp/mtls-router.log"})
		if err != nil {
			t.Fatal(err)
		}
		if handled {
			t.Fatal("backend and log flags should not be handled as meta flags")
		}
	})
	if output != "" {
		t.Fatalf("unexpected output for runtime flags: %q", output)
	}
}

func TestHandleMetaFlagsRejectsUnexpectedPositionalArgs(t *testing.T) {
	output := captureStdout(t, func() {
		handled, err := handleMetaFlags([]string{"print-config"})
		if err == nil {
			t.Fatal("expected positional arg to be rejected")
		}
		if handled {
			t.Fatal("unexpected positional arg should not be handled successfully")
		}
	})
	if output != "" {
		t.Fatalf("unexpected output for positional arg: %q", output)
	}
}

func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	old := os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	os.Stdout = w
	defer func() { os.Stdout = old }()

	fn()

	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	if _, err := io.Copy(&buf, r); err != nil {
		t.Fatal(err)
	}
	return buf.String()
}
