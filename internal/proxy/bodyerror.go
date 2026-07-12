package proxy

import (
	"fmt"
	"io"
	"net/http"
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

func wrapBodyReadErrors(r *http.Request) {
	if r.Body != nil {
		r.Body = bodyErrorReadCloser{ReadCloser: r.Body}
	}
}
