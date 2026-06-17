package proxy

import (
	"net/http"
	"testing"
)

func TestNewModifyResponse(t *testing.T) {
	resp := &http.Response{Header: make(http.Header)}
	resp.Header.Set("Content-Type", "text/event-stream; charset=utf-8")
	if err := NewModifyResponse()(resp); err != nil {
		t.Fatal(err)
	}
	if resp.Header.Get("Content-Type") != "text/event-stream" || resp.Header.Get("Cache-Control") != "no-cache" || resp.Header.Get("X-Accel-Buffering") != "no" {
		t.Fatalf("SSE headers not forced: %v", resp.Header)
	}

	other := &http.Response{Header: make(http.Header)}
	other.Header.Set("Content-Type", "application/json")
	other.Header.Set("Cache-Control", "max-age=60")
	if err := NewModifyResponse()(other); err != nil {
		t.Fatal(err)
	}
	if other.Header.Get("Content-Type") != "application/json" || other.Header.Get("Cache-Control") != "max-age=60" {
		t.Fatalf("non-SSE headers changed: %v", other.Header)
	}
}
