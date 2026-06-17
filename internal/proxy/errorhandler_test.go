package proxy

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestNewErrorHandler(t *testing.T) {
	r := httptest.NewRequest(http.MethodGet, "/", nil)
	for _, tc := range []struct {
		err  error
		want int
	}{{context.DeadlineExceeded, http.StatusGatewayTimeout}, {errors.New("x509 private key secret upstream.internal"), http.StatusBadGateway}} {
		rr := httptest.NewRecorder()
		NewErrorHandler()(rr, r, tc.err)
		if rr.Code != tc.want {
			t.Fatalf("status=%d want=%d", rr.Code, tc.want)
		}
		if ct := rr.Header().Get("Content-Type"); ct != "application/json" {
			t.Fatalf("content-type=%q", ct)
		}
		body := rr.Body.String()
		if !strings.Contains(body, "error") || strings.Contains(body, "private key") || strings.Contains(body, "upstream.internal") {
			t.Fatalf("unsafe JSON body: %q", body)
		}
	}
}
