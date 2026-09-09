package proxy

import (
	"bytes"
	"context"
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
)

const (
	workbenchPromptCanary = "workbench-prompt-canary-red-cube"
	workbenchB64Canary    = "workbench-b64-canary-NOT-A-REAL-IMAGE"
)

func TestWorkbenchCatalogsForwardSlashIDsAndAuthorization(t *testing.T) {
	models := testdataFile(t, "models.json")
	imageModels := testdataFile(t, "models_image.json")
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != contractBearer {
			t.Errorf("%s Authorization = %q", r.URL.Path, r.Header.Get("Authorization"))
		}
		switch r.URL.Path {
		case "/v1/models":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write(models)
		case "/v1/models/image":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write(imageModels)
		default:
			http.NotFound(w, r)
		}
	}))
	caPEM, certPEM, keyPEM := startContractUpstream(t, upstream)
	downstream, logs := newWorkbenchProxy(t, upstream.URL, caPEM, certPEM, keyPEM)

	for _, path := range []string{"/v1/models", "/v1/models/image"} {
		req, err := http.NewRequest(http.MethodGet, downstream.URL+path, nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Authorization", contractBearer)
		resp, err := downstream.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		body, err := io.ReadAll(resp.Body)
		_ = resp.Body.Close()
		if err != nil {
			t.Fatal(err)
		}
		if resp.StatusCode != http.StatusOK {
			t.Fatalf("%s status = %d", path, resp.StatusCode)
		}
		want := models
		if path == "/v1/models/image" {
			want = imageModels
		}
		if !bytes.Equal(body, want) {
			t.Fatalf("%s body rewritten", path)
		}
	}
	out := waitWorkbenchAccessLog(t, logs, "path=/v1/models/image")
	if !strings.Contains(out, "path=/v1/models") || !strings.Contains(out, "path=/v1/models/image") {
		t.Fatalf("access log missing catalog paths: %s", out)
	}
	if strings.Contains(out, contractBearer) || strings.Contains(out, strings.TrimPrefix(contractBearer, "Bearer ")) {
		t.Fatal("access log leaked Authorization")
	}
}

func TestWorkbenchChatSSEIsUnbufferedAndCancelStopsUpstream(t *testing.T) {
	first := "data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\n"
	cancelled := make(chan struct{})
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/v1/chat/completions" {
			http.NotFound(w, r)
			return
		}
		if r.Header.Get("Authorization") != contractBearer {
			t.Errorf("Authorization = %q", r.Header.Get("Authorization"))
		}
		body, _ := io.ReadAll(r.Body)
		if !bytes.Contains(body, []byte(workbenchPromptCanary)) {
			t.Errorf("upstream body missing prompt canary")
		}
		w.Header().Set("Content-Type", "text/event-stream; charset=utf-8")
		_, _ = io.WriteString(w, first)
		if err := http.NewResponseController(w).Flush(); err != nil {
			t.Errorf("flush: %v", err)
			return
		}
		select {
		case <-r.Context().Done():
			close(cancelled)
		case <-time.After(3 * time.Second):
			t.Error("upstream was not cancelled")
		}
	}))
	caPEM, certPEM, keyPEM := startContractUpstream(t, upstream)
	downstream, logs := newWorkbenchProxy(t, upstream.URL, caPEM, certPEM, keyPEM)

	ctx, cancel := context.WithCancel(context.Background())
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, downstream.URL+"/v1/chat/completions", strings.NewReader(`{"model":"gemini-3.8-flash","stream":true,"messages":[{"role":"user","content":"`+workbenchPromptCanary+`"}]}`))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", contractBearer)
	req.Header.Set("Content-Type", "application/json")
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	got := make([]byte, len(first))
	if _, err := io.ReadFull(resp.Body, got); err != nil {
		t.Fatal(err)
	}
	if string(got) != first {
		t.Fatalf("first SSE chunk = %q", got)
	}
	if resp.Header.Get("Content-Type") != "text/event-stream" {
		t.Fatalf("Content-Type = %q", resp.Header.Get("Content-Type"))
	}
	cancel()
	_ = resp.Body.Close()
	select {
	case <-cancelled:
	case <-time.After(3 * time.Second):
		t.Fatal("downstream cancel did not cancel upstream")
	}
	out := waitWorkbenchAccessLog(t, logs, "path=/v1/chat/completions")
	if strings.Contains(out, workbenchPromptCanary) {
		t.Fatal("access log leaked prompt")
	}
}

func TestWorkbenchImageGenerationForwardsBinaryJSONAndLogsStayClosed(t *testing.T) {
	png := testdataFile(t, "generation_binary.png")
	b64 := testdataFile(t, "generation_b64_json.json")
	editBody := `{"model":"cx/gpt-5.5-image","prompt":"` + workbenchPromptCanary + `","n":1,"size":"1024x1024","image":"data:image/png;base64,` + workbenchB64Canary + `","images":["data:image/png;base64,` + workbenchB64Canary + `"]}`

	cases := []struct {
		name     string
		wantBody []byte
		jsonResp bool
	}{
		{name: "binary", wantBody: png},
		{name: "b64_json", wantBody: b64, jsonResp: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var seenBody []byte
			upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/v1/images/generations" || r.URL.RawQuery != "response_format=binary" {
					t.Errorf("unexpected request %s %s", r.URL.Path, r.URL.RawQuery)
					http.NotFound(w, r)
					return
				}
				if r.Header.Get("Authorization") != contractBearer {
					t.Errorf("Authorization = %q", r.Header.Get("Authorization"))
				}
				body, _ := io.ReadAll(r.Body)
				seenBody = append([]byte(nil), body...)
				if tc.jsonResp {
					w.Header().Set("Content-Type", "application/json")
					_, _ = w.Write(b64)
					return
				}
				w.Header().Set("Content-Type", "application/octet-stream")
				_, _ = w.Write(png)
			}))
			caPEM, certPEM, keyPEM := startContractUpstream(t, upstream)
			downstream, logs := newWorkbenchProxy(t, upstream.URL, caPEM, certPEM, keyPEM)
			req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/images/generations?response_format=binary", strings.NewReader(editBody))
			if err != nil {
				t.Fatal(err)
			}
			req.Header.Set("Authorization", contractBearer)
			req.Header.Set("Content-Type", "application/json")
			resp, err := downstream.Client().Do(req)
			if err != nil {
				t.Fatal(err)
			}
			got, err := io.ReadAll(resp.Body)
			_ = resp.Body.Close()
			if err != nil {
				t.Fatal(err)
			}
			if resp.StatusCode != http.StatusOK || !bytes.Equal(got, tc.wantBody) {
				t.Fatalf("status=%d body rewritten or mismatched", resp.StatusCode)
			}
			if string(seenBody) != editBody {
				t.Fatal("generation JSON body was rewritten")
			}
			out := waitWorkbenchAccessLog(t, logs, "path=/v1/images/generations")
			if !strings.Contains(out, "path=/v1/images/generations") {
				t.Fatalf("missing generation path: %s", out)
			}
			for _, secret := range []string{
				contractBearer,
				strings.TrimPrefix(contractBearer, "Bearer "),
				workbenchPromptCanary,
				workbenchB64Canary,
				string(png),
				"response_format",
			} {
				if strings.Contains(out, secret) {
					t.Fatalf("access log leaked %q", secret)
				}
			}
		})
	}
}

func TestWorkbenchImageGenerationForwardsLargeB64JSONUnchanged(t *testing.T) {
	payload := bytes.Repeat([]byte("A"), 256*1024)
	body := append(append([]byte(`{"data":[{"b64_json":"`), payload...), []byte(`"}]}`)...)
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write(body)
	}))
	caPEM, certPEM, keyPEM := startContractUpstream(t, upstream)
	downstream, logs := newWorkbenchProxy(t, upstream.URL, caPEM, certPEM, keyPEM)
	req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/images/generations?response_format=binary", strings.NewReader(`{"model":"ag/gemini-3.1-flash-image","prompt":"`+workbenchPromptCanary+`","n":1,"size":"1024x1024"}`))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", contractBearer)
	req.Header.Set("Content-Type", "application/json")
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	got, err := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, body) {
		t.Fatal("large b64_json response was rewritten")
	}
	out := waitWorkbenchAccessLog(t, logs, "path=/v1/images/generations")
	if strings.Contains(out, workbenchPromptCanary) || strings.Contains(out, string(payload[:32])) {
		t.Fatal("access log leaked prompt or base64")
	}
}

func startContractUpstream(t *testing.T, server *httptest.Server) (caPEM, certPEM, keyPEM string) {
	t.Helper()
	caPEM, _, ca, caKey := testCertificate(t, "contract-ca-cert-canary", true, nil, nil)
	clientCertPEM, clientKeyPEM, _, _ := testCertificate(t, "contract-client-cert-canary", false, ca, caKey)
	serverCertPEM, serverKeyPEM, _, _ := testCertificate(t, "server", false, ca, caKey)
	configureTestMTLS(t, server, caPEM, serverCertPEM, serverKeyPEM)
	server.StartTLS()
	t.Cleanup(server.Close)
	return caPEM, clientCertPEM, clientKeyPEM
}

type workbenchLogBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *workbenchLogBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *workbenchLogBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

func newWorkbenchProxy(t *testing.T, upstreamURL, caPEM, certPEM, keyPEM string) (*httptest.Server, *workbenchLogBuffer) {
	t.Helper()
	transport, err := NewMTLSTransport(certPEM, keyPEM, caPEM)
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := url.Parse(upstreamURL)
	if err != nil {
		t.Fatal(err)
	}
	var logs workbenchLogBuffer
	var inflight sync.WaitGroup
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	reverseProxy := New(Options{Upstream: parsed, Transport: transport, ErrorLog: logger})
	access := contractAccessLog(reverseProxy, logger)
	downstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		inflight.Add(1)
		defer inflight.Done()
		access.ServeHTTP(w, r)
	}))
	t.Cleanup(func() {
		downstream.Close()
		inflight.Wait()
	})
	return downstream, &logs
}

func waitWorkbenchAccessLog(t *testing.T, logs *workbenchLogBuffer, needle string) string {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		out := logs.String()
		if strings.Contains(out, needle) {
			return out
		}
		time.Sleep(10 * time.Millisecond)
	}
	return logs.String()
}

func TestWorkbenchTestdataFilesArePresent(t *testing.T) {
	for _, name := range []string{
		"metadata.json",
		"models.json",
		"models_image.json",
		"chat_sse_with_image_block.txt",
		"generation_binary.png",
		"generation_b64_json.json",
		"generation_url_only.json",
	} {
		if _, err := os.Stat(filepath.Join("testdata", name)); err != nil {
			t.Errorf("%s: %v", name, err)
		}
	}
}

func TestWorkbenchChatSSEFixtureLinesStayIntactThroughProxy(t *testing.T) {
	fixture := testdataFile(t, "chat_sse_with_image_block.txt")
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write(fixture)
	}))
	caPEM, certPEM, keyPEM := startContractUpstream(t, upstream)
	downstream, _ := newWorkbenchProxy(t, upstream.URL, caPEM, certPEM, keyPEM)
	req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/chat/completions", strings.NewReader(`{"stream":true}`))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", contractBearer)
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	got, err := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, fixture) {
		t.Fatal("SSE fixture data lines were rewritten")
	}
}
