package proxy

import (
	"fmt"
	"io"
	"net/http"
	"net/http/httputil"
)

type clientBodyReadError struct {
	err error
}

func (e clientBodyReadError) Error() string {
	return fmt.Sprintf("client body read error: %v", e.err)
}

func (e clientBodyReadError) Unwrap() error {
	return e.err
}

type bodyErrorReadCloser struct {
	io.ReadCloser
}

func (b bodyErrorReadCloser) Read(p []byte) (int, error) {
	n, err := b.ReadCloser.Read(p)
	if err != nil && err != io.EOF {
		err = clientBodyReadError{err: err}
	}
	return n, err
}

func NewBodyErrorHandler(rp *httputil.ReverseProxy) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Body != nil {
			r.Body = bodyErrorReadCloser{ReadCloser: r.Body}
		}
		rp.ServeHTTP(w, r)
	})
}
