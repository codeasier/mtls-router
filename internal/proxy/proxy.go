package proxy

import (
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
