package mlog

import (
	"log/slog"
	"net/http"
	"time"
)

type ResponseRecorder struct {
	http.ResponseWriter
	Status int
	Bytes  int
}

func (r *ResponseRecorder) Unwrap() http.ResponseWriter {
	return r.ResponseWriter
}

func (r *ResponseRecorder) WriteHeader(status int) {
	if r.Status == 0 {
		r.Status = status
		r.ResponseWriter.WriteHeader(status)
	}
}

func (r *ResponseRecorder) Write(p []byte) (int, error) {
	if r.Status == 0 {
		r.Status = http.StatusOK
	}
	n, err := r.ResponseWriter.Write(p)
	r.Bytes += n
	return n, err
}

func AccessLog(logger *slog.Logger, req *http.Request, rec *ResponseRecorder, start time.Time, err error) {
	status := rec.Status
	if status == 0 {
		status = http.StatusOK
	}
	attrs := []any{"method", req.Method, "path", req.URL.RequestURI(), "status", status, "bytes", rec.Bytes, "latency", time.Since(start).String()}
	if err != nil {
		attrs = append(attrs, "error", err.Error())
	}
	logger.Info("access", attrs...)
}
