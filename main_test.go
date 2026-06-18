package main

import (
	"bytes"
	"io"
	"os"
	"testing"
)

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
