package main

import (
	"bytes"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/health"
	"github.com/codeasier/mtls-router/internal/proxy"
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

func registerManagementRoutes(mux *http.ServeMux) {
	mux.Handle("/version", routermeta.VersionHandler(routermeta.InfoProviderFunc(func() map[string]any {
		return map[string]any{"route": "version"}
	})))
	mux.Handle("/health", routermeta.HealthHandler(health.ProbeFunc(func() error { return nil })))
}

func registerProxyRoute(mux *http.ServeMux) {
	mux.Handle("/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("proxy"))
	}))
}

// Precedence comes from ServeMux pattern specificity, not registration order,
// so both orders must behave identically. Exercising both keeps the invariant
// from being misread as "register the management routes first".
func TestManagementRoutesTakePrecedenceOverProxyRoute(t *testing.T) {
	orders := []struct {
		name     string
		register func(*http.ServeMux)
	}{
		{name: "management registered first", register: func(mux *http.ServeMux) {
			registerManagementRoutes(mux)
			registerProxyRoute(mux)
		}},
		{name: "proxy registered first", register: func(mux *http.ServeMux) {
			registerProxyRoute(mux)
			registerManagementRoutes(mux)
		}},
	}
	tests := []struct {
		path string
		want string
	}{
		{path: "/version", want: "version"},
		{path: "/health", want: "ok"},
		{path: "/anything-else", want: "proxy"},
	}
	for _, order := range orders {
		t.Run(order.name, func(t *testing.T) {
			mux := http.NewServeMux()
			order.register(mux)
			for _, tt := range tests {
				t.Run(tt.path, func(t *testing.T) {
					rec := httptest.NewRecorder()
					mux.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, tt.path, nil))
					if !bytes.Contains(rec.Body.Bytes(), []byte(tt.want)) {
						t.Fatalf("body = %q, want to contain %q", rec.Body.String(), tt.want)
					}
				})
			}
		})
	}
}

func TestStartupLogsSanitizeUpstreamAndFailureDetails(t *testing.T) {
	const (
		userCanary     = "auth-user-canary"
		passwordCanary = "sk-password-canary"
		pathCanary     = "private-path-canary"
		queryCanary    = "sk-query-canary"
		errorCanary    = "upstream-error-canary"
	)
	upstream, err := url.Parse("https://" + userCanary + ":" + passwordCanary + "@upstream.example:8443/" + pathCanary + "?api_key=" + queryCanary)
	if err != nil {
		t.Fatal(err)
	}
	logPath := filepath.Join(t.TempDir(), "router.log")
	writer, closeLog, err := logWriter(logPath)
	if err != nil {
		t.Fatal(err)
	}
	logger := slog.New(slog.NewTextHandler(writer, nil))

	logListening(logger, "127.0.0.1:19099", upstream)
	logRunFailure(logger, errors.New("probe failed: "+upstream.String()+": "+errorCanary))
	closeLog()

	rawLog, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatal(err)
	}
	out := string(rawLog)
	for _, want := range []string{"msg=listening", "addr=127.0.0.1:19099", "upstream=https://upstream.example:8443", "msg=fatal", "reason=router_failure"} {
		if !strings.Contains(out, want) {
			t.Fatalf("startup log missing %q: %s", want, out)
		}
	}
	for _, canary := range []string{userCanary, passwordCanary, pathCanary, queryCanary, errorCanary, "api_key"} {
		if strings.Contains(out, canary) {
			t.Fatalf("startup log leaked %q: %s", canary, out)
		}
	}
}

func TestProxyStreamsFirstChunkThroughAccessLog(t *testing.T) {
	tests := []struct {
		name        string
		contentType string
		firstChunk  string
		secondChunk string
	}{
		{
			name:        "SSE",
			contentType: "text/event-stream; charset=utf-8",
			firstChunk:  "data: first\n\n",
			secondChunk: "data: second\n\n",
		},
		{
			name:        "chunked",
			contentType: "text/plain",
			firstChunk:  "first chunk\n",
			secondChunk: "second chunk\n",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			releaseUpstream := make(chan struct{})
			upstreamFinished := make(chan struct{})
			var releaseOnce sync.Once
			release := func() { releaseOnce.Do(func() { close(releaseUpstream) }) }
			defer release()

			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				defer close(upstreamFinished)
				w.Header().Set("Content-Type", tt.contentType)
				_, _ = io.WriteString(w, tt.firstChunk)
				if err := http.NewResponseController(w).Flush(); err != nil {
					return
				}
				<-releaseUpstream
				_, _ = io.WriteString(w, tt.secondChunk)
			}))
			t.Cleanup(upstream.Close)

			upstreamURL, err := url.Parse(upstream.URL)
			if err != nil {
				t.Fatal(err)
			}
			logger := slog.New(slog.NewTextHandler(io.Discard, nil))
			reverseProxy := proxy.New(proxy.Options{
				Upstream:  upstreamURL,
				Transport: http.DefaultTransport.(*http.Transport).Clone(),
				ErrorLog:  logger,
			})
			downstream := httptest.NewServer(withAccessLog(reverseProxy, logger))
			t.Cleanup(downstream.Close)

			type firstChunkResult struct {
				response *http.Response
				chunk    string
				err      error
			}
			resultCh := make(chan firstChunkResult, 1)
			go func() {
				resp, err := downstream.Client().Get(downstream.URL)
				if err != nil {
					resultCh <- firstChunkResult{err: err}
					return
				}
				buf := make([]byte, len(tt.firstChunk))
				_, err = io.ReadFull(resp.Body, buf)
				resultCh <- firstChunkResult{response: resp, chunk: string(buf), err: err}
			}()

			var result firstChunkResult
			select {
			case result = <-resultCh:
			case <-time.After(2 * time.Second):
				release()
				select {
				case result = <-resultCh:
					t.Fatalf("first chunk did not arrive before upstream release: %v", result.err)
				case <-time.After(2 * time.Second):
					t.Fatal("downstream request did not finish after upstream release")
				}
			}
			if result.response != nil {
				t.Cleanup(func() { _ = result.response.Body.Close() })
			}
			if result.err != nil {
				t.Fatalf("read first chunk: %v", result.err)
			}
			if result.chunk != tt.firstChunk {
				t.Fatalf("first chunk = %q, want %q", result.chunk, tt.firstChunk)
			}
			select {
			case <-upstreamFinished:
				t.Fatal("upstream completed before the first chunk was observed")
			default:
			}

			if tt.name == "SSE" {
				if got := result.response.Header.Get("Content-Type"); got != "text/event-stream" {
					t.Fatalf("Content-Type = %q, want text/event-stream", got)
				}
				if got := result.response.Header.Get("Cache-Control"); got != "no-cache" {
					t.Fatalf("Cache-Control = %q, want no-cache", got)
				}
				if got := result.response.Header.Get("X-Accel-Buffering"); got != "no" {
					t.Fatalf("X-Accel-Buffering = %q, want no", got)
				}
			}

			release()
			rest, err := io.ReadAll(result.response.Body)
			if err != nil {
				t.Fatalf("read remaining response: %v", err)
			}
			if string(rest) != tt.secondChunk {
				t.Fatalf("remaining response = %q, want %q", rest, tt.secondChunk)
			}
			select {
			case <-upstreamFinished:
			case <-time.After(2 * time.Second):
				t.Fatal("upstream did not finish after release")
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
