package proxy

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

const (
	imageBearer     = "Bearer sk-image-fixture-9a8b7c6d5e"
	imagePrompt     = "A serene mountain landscape at sunset with warm golden light"
	imageEditPrompt = "Make the sky more dramatic with deeper orange and purple tones"
)

type imageFixtureRequest struct {
	method string
	uri    string
	body   string
}

func loadImageFixture(t *testing.T, name string) []byte {
	t.Helper()
	return loadFixture(t, name)
}

func newImageContractMTLSServer(t *testing.T, handler http.HandlerFunc) (*httptest.Server, string, string, string) {
	t.Helper()
	caPEM, _, ca, caKey := testCertificate(t, "image-contract-ca", true, nil, nil)
	clientCertPEM, clientKeyPEM, _, _ := testCertificate(t, "image-contract-client", false, ca, caKey)
	serverCertPEM, serverKeyPEM, _, _ := testCertificate(t, "server", false, ca, caKey)
	server := httptest.NewUnstartedServer(handler)
	configureTestMTLS(t, server, caPEM, serverCertPEM, serverKeyPEM)
	server.StartTLS()
	t.Cleanup(server.Close)
	return server, clientCertPEM, clientKeyPEM, caPEM
}

func newImageReverseProxy(t *testing.T, upstreamURL *url.URL, clientCertPEM, clientKeyPEM, caPEM string, logger *slog.Logger) http.Handler {
	t.Helper()
	transport, err := NewMTLSTransport(clientCertPEM, clientKeyPEM, caPEM)
	if err != nil {
		t.Fatal(err)
	}
	rp := New(Options{Upstream: upstreamURL, Transport: transport, ErrorLog: logger})
	return contractAccessLog(rp, logger)
}

// TestImageModelsCatalogContract verifies GET /v1/models/image preserves
// Authorization, slash model IDs, path, and bounded JSON response.
func TestImageModelsCatalogContract(t *testing.T) {
	catalogBody := string(loadImageFixture(t, "models-image-response.json"))
	var hit bool
	upstream, clientCertPEM, clientKeyPEM, caPEM := newImageContractMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hit = true
		if r.Method != http.MethodGet {
			t.Errorf("method = %s, want GET", r.Method)
		}
		if r.URL.Path != "/v1/models/image" {
			t.Errorf("path = %q, want /v1/models/image", r.URL.Path)
		}
		if got := r.Header.Get("Authorization"); got != imageBearer {
			t.Errorf("Authorization = %q, want %q", got, imageBearer)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, catalogBody)
	}))
	upstreamURL, _ := url.Parse(upstream.URL)

	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	handler := newImageReverseProxy(t, upstreamURL, clientCertPEM, clientKeyPEM, caPEM, logger)
	downstream := httptest.NewServer(handler)
	t.Cleanup(downstream.Close)

	req, err := http.NewRequest(http.MethodGet, downstream.URL+"/v1/models/image", nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", imageBearer)
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d, body = %q", resp.StatusCode, body)
	}
	if !hit {
		t.Fatal("upstream was not hit")
	}

	var catalog modelsListFixture
	if err := json.Unmarshal(body, &catalog); err != nil {
		t.Fatalf("response is not valid JSON: %v", err)
	}
	ids := make(map[string]bool, len(catalog.Data))
	for _, m := range catalog.Data {
		ids[m.ID] = true
		if !strings.Contains(m.ID, "/") {
			t.Errorf("model ID %q does not contain slash", m.ID)
		}
	}
	if !ids[presetModelCX] {
		t.Errorf("preset model %q missing from proxied response", presetModelCX)
	}
	if !ids[presetModelAG] {
		t.Errorf("preset model %q missing from proxied response", presetModelAG)
	}

	time.Sleep(50 * time.Millisecond)
	out := logs.String()
	if !strings.Contains(out, "path=/v1/models/image") {
		t.Errorf("logs missing path=/v1/models/image: %s", out)
	}
	if strings.Contains(out, imageBearer) || strings.Contains(out, strings.TrimPrefix(imageBearer, "Bearer ")) {
		t.Errorf("logs leaked Authorization: %s", out)
	}
}

// TestImageGenerationContract verifies POST /v1/images/generations generation
// request body, headers, and b64_json response are not rewritten or buffered.
func TestImageGenerationContract(t *testing.T) {
	for _, tc := range []struct {
		name         string
		requestFile  string
		responseFile string
	}{
		{"generation-cx", "generation-cx-request.json", "generation-cx-response.json"},
		{"generation-ag", "generation-ag-request.json", "generation-ag-response.json"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			reqBody := string(loadImageFixture(t, tc.requestFile))
			respBody := loadImageFixture(t, tc.responseFile)
			var upstreamBody string
			var upstreamAuth string
			var upstreamCT string
			upstream, clientCertPEM, clientKeyPEM, caPEM := newImageContractMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Method != http.MethodPost {
					t.Errorf("method = %s, want POST", r.Method)
				}
				if r.URL.Path != "/v1/images/generations" {
					t.Errorf("path = %q, want /v1/images/generations", r.URL.Path)
				}
				body, err := io.ReadAll(r.Body)
				if err != nil {
					t.Fatalf("read upstream body: %v", err)
				}
				upstreamBody = string(body)
				upstreamAuth = r.Header.Get("Authorization")
				upstreamCT = r.Header.Get("Content-Type")
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write(respBody)
			}))
			upstreamURL, _ := url.Parse(upstream.URL)

			var logs bytes.Buffer
			logger := slog.New(slog.NewTextHandler(&logs, nil))
			handler := newImageReverseProxy(t, upstreamURL, clientCertPEM, clientKeyPEM, caPEM, logger)
			downstream := httptest.NewServer(handler)
			t.Cleanup(downstream.Close)

			req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/images/generations", strings.NewReader(reqBody))
			if err != nil {
				t.Fatal(err)
			}
			req.Header.Set("Authorization", imageBearer)
			req.Header.Set("Content-Type", "application/json")
			resp, err := downstream.Client().Do(req)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			body, err := io.ReadAll(resp.Body)
			if err != nil {
				t.Fatal(err)
			}
			if resp.StatusCode != http.StatusOK {
				t.Fatalf("status = %d, body = %q", resp.StatusCode, body)
			}
			if upstreamBody != reqBody {
				t.Errorf("upstream body was rewritten:\ngot  %q\nwant %q", upstreamBody, reqBody)
			}
			if upstreamAuth != imageBearer {
				t.Errorf("upstream Authorization = %q, want %q", upstreamAuth, imageBearer)
			}
			if upstreamCT != "application/json" {
				t.Errorf("upstream Content-Type = %q, want application/json", upstreamCT)
			}
			if !bytes.Equal(body, respBody) {
				t.Errorf("response body was rewritten:\ngot  %q\nwant %q", body, respBody)
			}
			out := logs.String()
			for _, secret := range []string{
				imageBearer,
				strings.TrimPrefix(imageBearer, "Bearer "),
				imagePrompt,
			} {
				if strings.Contains(out, secret) {
					t.Errorf("logs leaked %q: %s", secret, out)
				}
			}
		})
	}
}

// TestImageEditContract verifies POST /v1/images/generations with a data URI
// image field (edit mode) passes the JSON body and response unchanged.
func TestImageEditContract(t *testing.T) {
	for _, tc := range []struct {
		name         string
		requestFile  string
		responseFile string
	}{
		{"edit-cx", "edit-cx-request.json", "edit-cx-response.json"},
		{"edit-ag", "edit-ag-request.json", "edit-ag-response.json"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			reqBody := string(loadImageFixture(t, tc.requestFile))
			respBody := loadImageFixture(t, tc.responseFile)

			var reqBody2 struct {
				Image string `json:"image"`
			}
			if err := json.Unmarshal([]byte(reqBody), &reqBody2); err != nil {
				t.Fatal(err)
			}
			if !strings.HasPrefix(reqBody2.Image, "data:image/png;base64,") {
				t.Fatalf("edit request image is not a data URI: %q", reqBody2.Image[:min(len(reqBody2.Image), 40)])
			}

			upstream, clientCertPEM, clientKeyPEM, caPEM := newImageContractMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				body, err := io.ReadAll(r.Body)
				if err != nil {
					t.Fatalf("read upstream body: %v", err)
				}
				if string(body) != reqBody {
					t.Errorf("upstream body was rewritten:\ngot  %q\nwant %q", body, reqBody)
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write(respBody)
			}))
			upstreamURL, _ := url.Parse(upstream.URL)

			var logs bytes.Buffer
			logger := slog.New(slog.NewTextHandler(&logs, nil))
			handler := newImageReverseProxy(t, upstreamURL, clientCertPEM, clientKeyPEM, caPEM, logger)
			downstream := httptest.NewServer(handler)
			t.Cleanup(downstream.Close)

			req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/images/generations", strings.NewReader(reqBody))
			if err != nil {
				t.Fatal(err)
			}
			req.Header.Set("Authorization", imageBearer)
			req.Header.Set("Content-Type", "application/json")
			resp, err := downstream.Client().Do(req)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			body, err := io.ReadAll(resp.Body)
			if err != nil {
				t.Fatal(err)
			}
			if resp.StatusCode != http.StatusOK {
				t.Fatalf("status = %d, body = %q", resp.StatusCode, body)
			}
			if !bytes.Equal(body, respBody) {
				t.Errorf("response body was rewritten:\ngot  %q\nwant %q", body, respBody)
			}
			out := logs.String()
			for _, secret := range []string{
				imageBearer,
				strings.TrimPrefix(imageBearer, "Bearer "),
				imageEditPrompt,
				reqBody2.Image,
				strings.TrimPrefix(reqBody2.Image, "data:image/png;base64,"),
			} {
				if strings.Contains(out, secret) {
					t.Errorf("logs leaked secret: %s", out)
				}
			}
		})
	}
}

// TestImageLargeB64ResponseNotRewritten verifies a large b64_json response
// passes through the proxy without being rewritten or truncated.
func TestImageLargeB64ResponseNotRewritten(t *testing.T) {
	largeB64 := base64.StdEncoding.EncodeToString(bytes.Repeat([]byte{0x89, 0x50, 0x4e, 0x47}, 8192))
	largeResponse := fmt.Sprintf(`{"created":1719500000,"data":[{"b64_json":"%s"}]}`, largeB64)
	reqBody := string(loadImageFixture(t, "generation-cx-request.json"))

	upstream, clientCertPEM, clientKeyPEM, caPEM := newImageContractMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, largeResponse)
	}))
	upstreamURL, _ := url.Parse(upstream.URL)

	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	handler := newImageReverseProxy(t, upstreamURL, clientCertPEM, clientKeyPEM, caPEM, logger)
	downstream := httptest.NewServer(handler)
	t.Cleanup(downstream.Close)

	req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/images/generations", strings.NewReader(reqBody))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", imageBearer)
	req.Header.Set("Content-Type", "application/json")
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d, body = %q", resp.StatusCode, body)
	}
	if string(body) != largeResponse {
		t.Errorf("large response was rewritten or truncated: got %d bytes, want %d bytes", len(body), len(largeResponse))
	}
}

// TestImageDownstreamCancelDoesNotHang verifies that canceling the downstream
// request does not cause the proxy to leak secrets in logs.
func TestImageDownstreamCancelDoesNotHang(t *testing.T) {
	unblockUpstream := make(chan struct{})
	upstream, clientCertPEM, clientKeyPEM, caPEM := newImageContractMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		select {
		case <-r.Context().Done():
		case <-unblockUpstream:
		}
	}))
	upstreamURL, _ := url.Parse(upstream.URL)

	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	handler := newImageReverseProxy(t, upstreamURL, clientCertPEM, clientKeyPEM, caPEM, logger)
	downstream := httptest.NewServer(handler)
	t.Cleanup(downstream.Close)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	reqBody := `{"model":"cx/gpt-5.5-image","prompt":"cancel-test-prompt","n":1}`
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, downstream.URL+"/v1/images/generations", strings.NewReader(reqBody))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", imageBearer)
	req.Header.Set("Content-Type", "application/json")

	go func() {
		resp, err := downstream.Client().Do(req)
		if resp != nil {
			resp.Body.Close()
		}
		_ = err
	}()

	time.Sleep(200 * time.Millisecond)
	cancel()
	close(unblockUpstream)

	out := logs.String()
	for _, secret := range []string{
		imageBearer,
		strings.TrimPrefix(imageBearer, "Bearer "),
		"cancel-test-prompt",
	} {
		if strings.Contains(out, secret) {
			t.Errorf("logs leaked %q: %s", secret, out)
		}
	}
}

// TestImageUpstreamFailureSanitized verifies transport/upstream errors return
// sanitized errors and don't leak prompt, key, base64, or response body.
func TestImageUpstreamFailureSanitized(t *testing.T) {
	reqBody := string(loadImageFixture(t, "generation-cx-request.json"))
	largeB64 := base64.StdEncoding.EncodeToString(bytes.Repeat([]byte{0x89, 0x50, 0x4e, 0x47}, 512))
	failureSecrets := []string{
		imageBearer,
		strings.TrimPrefix(imageBearer, "Bearer "),
		imagePrompt,
		largeB64,
	}

	caPEM, _, ca, caKey := testCertificate(t, "image-contract-ca", true, nil, nil)
	clientCertPEM, clientKeyPEM, _, _ := testCertificate(t, "image-contract-client", false, ca, caKey)
	serverCertPEM, serverKeyPEM, _, _ := testCertificate(t, "server", false, ca, caKey)
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, fmt.Sprintf(`{"error":"%s"}`, largeB64))
	}))
	configureTestMTLS(t, upstream, caPEM, serverCertPEM, serverKeyPEM)
	upstream.StartTLS()
	t.Cleanup(upstream.Close)

	transport, err := NewMTLSTransport(clientCertPEM, clientKeyPEM, caPEM)
	if err != nil {
		t.Fatal(err)
	}
	upstreamURL, _ := url.Parse(upstream.URL)
	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	rp := New(Options{Upstream: upstreamURL, Transport: transport, ErrorLog: logger})
	rp.Transport = contractRoundTripper{
		base:       transport,
		failureErr: errors.New(strings.Join(failureSecrets, " ")),
	}

	downstream := httptest.NewServer(contractAccessLog(rp, logger))
	t.Cleanup(downstream.Close)

	req, err := http.NewRequest(http.MethodPost, downstream.URL+"/v1/failure", strings.NewReader(reqBody))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", imageBearer)
	req.Header.Set("Content-Type", "application/json")
	resp, err := downstream.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()

	if resp.StatusCode != http.StatusBadGateway {
		t.Fatalf("status = %d, want 502, body = %q", resp.StatusCode, body)
	}
	if string(body) != "{\"error\":\"Bad Gateway\"}\n" {
		t.Errorf("failure body = %q, want {\"error\":\"Bad Gateway\"}", body)
	}

	out := logs.String()
	for _, secret := range failureSecrets {
		if strings.Contains(out, secret) {
			t.Errorf("logs leaked secret: %s", out)
		}
		if strings.Contains(string(body), secret) {
			t.Errorf("error response leaked secret: %s", body)
		}
	}
}

// TestImageAccessLogSanitization verifies access logs for image requests
// contain only method, path, status, bytes, latency - not key, prompt,
// base64, or image bytes.
func TestImageAccessLogSanitization(t *testing.T) {
	catalogBody := string(loadImageFixture(t, "models-image-response.json"))
	reqBody := string(loadImageFixture(t, "generation-cx-request.json"))
	respBody := string(loadImageFixture(t, "generation-cx-response.json"))

	var reqCount int
	upstream, clientCertPEM, clientKeyPEM, caPEM := newImageContractMTLSServer(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		reqCount++
		w.Header().Set("Content-Type", "application/json")
		if r.Method == http.MethodGet {
			_, _ = io.WriteString(w, catalogBody)
			return
		}
		_, _ = io.WriteString(w, respBody)
	}))
	upstreamURL, _ := url.Parse(upstream.URL)

	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	handler := newImageReverseProxy(t, upstreamURL, clientCertPEM, clientKeyPEM, caPEM, logger)
	downstream := httptest.NewServer(handler)
	t.Cleanup(downstream.Close)

	req1, _ := http.NewRequest(http.MethodGet, downstream.URL+"/v1/models/image", nil)
	req1.Header.Set("Authorization", imageBearer)
	resp1, err := downstream.Client().Do(req1)
	if err != nil {
		t.Fatal(err)
	}
	io.Copy(io.Discard, resp1.Body)
	resp1.Body.Close()

	req2, _ := http.NewRequest(http.MethodPost, downstream.URL+"/v1/images/generations", strings.NewReader(reqBody))
	req2.Header.Set("Authorization", imageBearer)
	req2.Header.Set("Content-Type", "application/json")
	resp2, err := downstream.Client().Do(req2)
	if err != nil {
		t.Fatal(err)
	}
	io.Copy(io.Discard, resp2.Body)
	resp2.Body.Close()

	time.Sleep(50 * time.Millisecond)
	out := logs.String()
	for _, wantPath := range []string{"path=/v1/models/image", "path=/v1/images/generations"} {
		if !strings.Contains(out, wantPath) {
			t.Errorf("logs missing %q: %s", wantPath, out)
		}
	}
	b64Value := fixtureB64PNG
	for _, secret := range []string{
		imageBearer,
		strings.TrimPrefix(imageBearer, "Bearer "),
		imagePrompt,
		b64Value,
		reqBody,
		respBody,
		"b64_json",
	} {
		if strings.Contains(out, secret) {
			t.Errorf("logs leaked %q: %s", secret, out)
		}
	}
}
