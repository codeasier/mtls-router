package main

import (
	"bytes"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"

	"github.com/codeasier/mtls-router/internal/health"
	"github.com/codeasier/mtls-router/internal/routermeta"
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

func TestManagementRoutesTakePrecedenceOverProxyRoute(t *testing.T) {
	mux := http.NewServeMux()
	mux.Handle("/version", routermeta.VersionHandler(routermeta.InfoProviderFunc(func() map[string]any {
		return map[string]any{"route": "version"}
	})))
	mux.Handle("/health", routermeta.HealthHandler(health.ProbeFunc(func(health.ProbeOptions) error { return nil })))
	mux.Handle("/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("proxy"))
	}))

	tests := []struct {
		path string
		want string
	}{
		{path: "/version", want: "version"},
		{path: "/health", want: "ok"},
		{path: "/anything-else", want: "proxy"},
	}
	for _, tt := range tests {
		t.Run(tt.path, func(t *testing.T) {
			rec := httptest.NewRecorder()
			mux.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, tt.path, nil))
			if !bytes.Contains(rec.Body.Bytes(), []byte(tt.want)) {
				t.Fatalf("body = %q, want to contain %q", rec.Body.String(), tt.want)
			}
		})
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
