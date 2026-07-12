package proxy

import (
	"log"
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
	director := NewDirector(opts.Upstream)
	rp := &httputil.ReverseProxy{
		Director: func(r *http.Request) {
			wrapBodyReadErrors(r)
			director(r)
		},
		ModifyResponse: NewModifyResponse(),
		ErrorHandler:   NewErrorHandler(opts.ErrorLog),
		FlushInterval:  -1,
		Transport:      opts.Transport,
		ErrorLog:       log.New(sanitizedProxyLogWriter{logger: opts.ErrorLog}, "", 0),
	}
	return rp
}

type sanitizedProxyLogWriter struct {
	logger *slog.Logger
}

func (w sanitizedProxyLogWriter) Write(p []byte) (int, error) {
	if w.logger != nil {
		w.logger.Error("proxy stream failed")
	}
	return len(p), nil
}
