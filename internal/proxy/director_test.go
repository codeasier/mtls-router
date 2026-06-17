package proxy

import (
	"io"
	"net/http"
	"net/url"
	"strings"
	"testing"
)

func TestNewDirector(t *testing.T) {
	upstream, _ := url.Parse("https://upstream.example:8443")
	req, _ := http.NewRequest(http.MethodPost, "http://local.test/v1/chat?x=1", io.NopCloser(strings.NewReader("body")))
	req.Header.Set("Authorization", "Bearer token")

	NewDirector(upstream)(req)

	if req.URL.Scheme != "https" || req.URL.Host != "upstream.example:8443" || req.Host != "upstream.example:8443" {
		t.Fatalf("upstream not rewritten: url=%s host=%s", req.URL.String(), req.Host)
	}
	if req.URL.Path != "/v1/chat" || req.URL.RawQuery != "x=1" {
		t.Fatalf("path/query changed: %s", req.URL.String())
	}
	if req.Header.Get("Authorization") != "Bearer token" {
		t.Fatal("header not preserved")
	}
	b, _ := io.ReadAll(req.Body)
	if string(b) != "body" {
		t.Fatal("body changed")
	}
}
