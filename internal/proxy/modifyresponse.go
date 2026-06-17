package proxy

import (
	"mime"
	"net/http"
)

func NewModifyResponse() func(*http.Response) error {
	return func(resp *http.Response) error {
		mediaType, _, err := mime.ParseMediaType(resp.Header.Get("Content-Type"))
		if err == nil && mediaType == "text/event-stream" {
			resp.Header.Set("Content-Type", "text/event-stream")
			resp.Header.Set("Cache-Control", "no-cache")
			resp.Header.Set("X-Accel-Buffering", "no")
		}
		return nil
	}
}
