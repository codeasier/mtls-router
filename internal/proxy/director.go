package proxy

import (
	"net/http"
	"net/url"
)

func NewDirector(upstream *url.URL) func(*http.Request) {
	return func(r *http.Request) {
		r.URL.Scheme = upstream.Scheme
		r.URL.Host = upstream.Host
		r.Host = upstream.Host
	}
}
