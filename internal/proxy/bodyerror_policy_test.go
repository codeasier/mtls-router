package proxy

import (
	"net/http"
	"net/http/httputil"
	"net/url"
	"testing"
)

func TestNewKeepsReverseProxyDirectlyUsableAsHandler(t *testing.T) {
	upstream, err := url.Parse("https://upstream.example.test")
	if err != nil {
		t.Fatal(err)
	}
	rp := New(Options{Upstream: upstream})

	if _, ok := any(rp).(*httputil.ReverseProxy); !ok {
		t.Fatalf("handler type = %T, want *httputil.ReverseProxy", rp)
	}
	if _, ok := any(rp).(http.Handler); !ok {
		t.Fatal("reverse proxy must remain directly usable as an http.Handler")
	}
}
