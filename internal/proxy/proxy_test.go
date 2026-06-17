package proxy

import (
	"io"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"strings"
	"sync/atomic"
	"testing"
)

type blockingBody struct {
	first   strings.Reader
	blocked atomic.Bool
}

func newBlockingBody(prefix string) *blockingBody {
	return &blockingBody{first: *strings.NewReader(prefix)}
}

func (b *blockingBody) Read(p []byte) (int, error) {
	if b.first.Len() > 0 {
		return b.first.Read(p)
	}
	b.blocked.Store(true)
	return 0, io.ErrUnexpectedEOF
}

func (b *blockingBody) Close() error { return nil }

func TestWrapHandlerDoesNotReadPastSniffWindow(t *testing.T) {
	body := newBlockingBody(strings.Repeat("x", sniffLimit))
	req, err := http.NewRequest(http.MethodPost, "http://example.test", body)
	if err != nil {
		t.Fatal(err)
	}

	rp := &httputil.ReverseProxy{Director: func(r *http.Request) {}}
	WrapHandler(rp).ServeHTTP(httptest.NewRecorder(), req)

	if body.blocked.Load() {
		t.Fatal("WrapHandler read beyond the sniff window before proxying")
	}
}
