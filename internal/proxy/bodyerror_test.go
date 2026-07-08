package proxy

import (
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
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

func TestNewReturnsBadRequestForClientBodyReadError(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.Copy(io.Discard, r.Body)
		w.WriteHeader(http.StatusNoContent)
	}))
	defer upstream.Close()

	upstreamURL, err := url.Parse(upstream.URL)
	if err != nil {
		t.Fatal(err)
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	defer transport.CloseIdleConnections()
	rp := New(Options{Upstream: upstreamURL, Transport: transport})
	req := httptest.NewRequest(http.MethodPost, "http://example.test", &failingBody{
		reader: strings.NewReader("partial body"),
		err:    errors.New("client upload failed"),
	})
	rr := httptest.NewRecorder()

	rp.ServeHTTP(rr, req)

	if rr.Code != http.StatusBadRequest {
		t.Fatalf("status=%d want=%d body=%q", rr.Code, http.StatusBadRequest, rr.Body.String())
	}
}
