package proxy

import (
	"bytes"
	"io"
	"log/slog"
	"net/http"
	"net/http/httputil"
	"net/url"
)

type Options struct {
	Upstream  *url.URL
	Transport *http.Transport
	ErrorLog  *slog.Logger
}

func New(opts Options) *httputil.ReverseProxy {
	rp := &httputil.ReverseProxy{
		Director:       NewDirector(opts.Upstream),
		ModifyResponse: NewModifyResponse(),
		ErrorHandler:   NewErrorHandler(),
		FlushInterval:  -1,
		Transport:      opts.Transport,
	}
	if opts.ErrorLog != nil {
		rp.ErrorLog = slog.NewLogLogger(opts.ErrorLog.Handler(), slog.LevelError)
	}
	return rp
}

func WrapHandler(rp *httputil.ReverseProxy) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ctx := r.Context()
		if r.Body != nil {
			buf := make([]byte, sniffLimit)
			n, err := io.ReadFull(r.Body, buf)
			if err != nil && err != io.ErrUnexpectedEOF && err != io.EOF {
				http.Error(w, http.StatusText(http.StatusBadRequest), http.StatusBadRequest)
				return
			}
			data := buf[:n]
			r.Body = replayReadCloser{Reader: io.MultiReader(bytes.NewReader(data), r.Body), close: r.Body.Close}
			if containsStreamTrue(data) {
				ctx = contextWithStream(ctx)
			}
		}
		rp.ServeHTTP(w, r.WithContext(ctx))
	})
}
