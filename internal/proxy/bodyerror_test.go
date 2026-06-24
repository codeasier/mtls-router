package proxy

import (
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"strings"
	"testing"
)

type failingBody struct {
	reader *strings.Reader
	err    error
}

func (b *failingBody) Read(p []byte) (int, error) {
	if b.reader.Len() > 0 {
		return b.reader.Read(p)
	}
	return 0, b.err
}

func (b *failingBody) Close() error { return nil }

func TestNewBodyErrorHandlerReturnsBadRequestForClientBodyReadError(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.Copy(io.Discard, r.Body)
		w.WriteHeader(http.StatusNoContent)
	}))
	defer upstream.Close()

	rp := &httputil.ReverseProxy{
		Director: func(r *http.Request) {
			r.URL.Scheme = "http"
			r.URL.Host = strings.TrimPrefix(upstream.URL, "http://")
		},
		ErrorHandler: NewErrorHandler(),
	}
	req := httptest.NewRequest(http.MethodPost, "http://example.test", &failingBody{
		reader: strings.NewReader("partial body"),
		err:    errors.New("client upload failed"),
	})
	rr := httptest.NewRecorder()

	NewBodyErrorHandler(rp).ServeHTTP(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Fatalf("status=%d want=%d body=%q", rr.Code, http.StatusBadRequest, rr.Body.String())
	}
}
