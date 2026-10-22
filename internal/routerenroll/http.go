package routerenroll

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"strconv"
	"time"
)

const (
	DefaultListen   = "127.0.0.1:19101"
	MaxBodyBytes    = 16 * 1024
	ShutdownTimeout = 5 * time.Second
)

var (
	ErrListen        = errors.New("enroll_listen_failed")
	ErrListenInvalid = errors.New("enroll_listen_invalid")
)

func ValidateListen(address string) error {
	host, port, err := net.SplitHostPort(address)
	ip, parseErr := netip.ParseAddr(host)
	n, portErr := strconv.Atoi(port)
	if err != nil || parseErr != nil || !ip.IsLoopback() || ip.Zone() != "" || portErr != nil || n < 1 || n > 65535 {
		return ErrListenInvalid
	}
	return nil
}

// NewHTTPServer is for bounded enrollment data, never for model streaming.
func NewHTTPServer(address string, handler http.Handler, logger *slog.Logger) (*http.Server, error) {
	if err := ValidateListen(address); err != nil {
		return nil, err
	}
	if logger == nil {
		logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	}
	return &http.Server{
		Addr: address, Handler: boundedHandler(handler, logger),
		ReadHeaderTimeout: 5 * time.Second, ReadTimeout: 10 * time.Second,
		WriteTimeout: 20 * time.Second, IdleTimeout: 30 * time.Second, MaxHeaderBytes: 16 * 1024,
		// net/http's default diagnostics can contain peer-controlled data.
		ErrorLog: slog.NewLogLogger(slog.NewTextHandler(io.Discard, nil), slog.LevelError),
	}, nil
}

func Serve(ctx context.Context, server *http.Server) error {
	if ValidateListen(server.Addr) != nil {
		return ErrListenInvalid
	}
	listener, err := net.Listen("tcp", server.Addr)
	if err != nil {
		return ErrListen
	}
	return serveListener(ctx, server, listener)
}

func serveListener(ctx context.Context, server *http.Server, listener net.Listener) error {
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	select {
	case err := <-done:
		if errors.Is(err, http.ErrServerClosed) {
			return nil
		}
		return errors.New("enroll_serve_failed")
	case <-ctx.Done():
		shutdown, cancel := context.WithTimeout(context.Background(), ShutdownTimeout)
		defer cancel()
		if server.Shutdown(shutdown) != nil {
			_ = server.Close()
			<-done
			return errors.New("enroll_shutdown_timeout")
		}
		<-done
		return nil
	}
}

var endpointMethods = map[string]string{
	"/enroll/ready":        "GET",
	"/enroll/native/start": "POST", "/enroll/native/verify": "GET",
	"/enroll/native/authorize": "POST", "/enroll/gitcode/callback": "GET",
	"/enroll/native/approve": "POST", "/enroll/native/poll": "POST",
	"/enroll/native/cancel": "POST",
	RenewPath:               "POST", RenewRecoverPath: "POST", DeviceRevokePath: "POST",
}

// NewHandler exposes exact enrollment paths. Unimplemented operations remain
// closed until their durable services are assembled; they cannot issue tokens.
func NewHandler(ready func(context.Context) bool) http.Handler {
	return newHandler(ready, nil)
}
func newHandler(ready func(context.Context) bool, routes map[string]http.HandlerFunc) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		method, exists := endpointMethods[r.URL.Path]
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("Referrer-Policy", "no-referrer")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		if !exists {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		if r.Method != method {
			w.Header().Set("Allow", method)
			w.WriteHeader(http.StatusMethodNotAllowed)
			return
		}
		// Business credentials never authorize or enter the enrollment service.
		if r.Header.Get("Authorization") != "" || r.Header.Get("X-Api-Key") != "" {
			WriteError(w, InvalidRequest)
			return
		}
		if r.URL.Path == "/enroll/ready" && ready != nil && ready(r.Context()) {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		if handler := routes[r.URL.Path]; handler != nil {
			handler(w, r)
			return
		}
		WriteError(w, ServiceUnavailable)
	})
}

func decodeJSON(w http.ResponseWriter, r *http.Request, target any) bool {
	if r.Header.Get("Content-Type") != "application/json" {
		WriteError(w, InvalidRequest)
		return false
	}
	data, err := io.ReadAll(io.LimitReader(r.Body, MaxBodyBytes+1))
	if err != nil || len(data) > MaxBodyBytes {
		WriteError(w, InvalidRequest)
		return false
	}
	defer clear(data)
	// Each endpoint is a single object. Reject duplicate keys instead of relying
	// on the JSON decoder's last-value-wins behavior across trust boundaries.
	keys := json.NewDecoder(bytes.NewReader(data))
	first, err := keys.Token()
	if err != nil || first != json.Delim('{') {
		WriteError(w, InvalidRequest)
		return false
	}
	seen := map[string]bool{}
	for keys.More() {
		key, err := keys.Token()
		name, ok := key.(string)
		if err != nil || !ok || seen[name] {
			WriteError(w, InvalidRequest)
			return false
		}
		seen[name] = true
		var raw json.RawMessage
		if keys.Decode(&raw) != nil {
			WriteError(w, InvalidRequest)
			return false
		}
	}
	if _, err := keys.Token(); err != nil {
		WriteError(w, InvalidRequest)
		return false
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if decoder.Decode(target) != nil || decoder.Decode(new(any)) != io.EOF {
		WriteError(w, InvalidRequest)
		return false
	}
	return true
}

type accessWriter struct {
	http.ResponseWriter
	status, bytes int
}

func (w *accessWriter) WriteHeader(status int) {
	if w.status == 0 {
		w.status = status
		w.ResponseWriter.WriteHeader(status)
	}
}
func (w *accessWriter) Write(value []byte) (int, error) {
	if w.status == 0 {
		w.WriteHeader(http.StatusOK)
	}
	n, err := w.ResponseWriter.Write(value)
	w.bytes += n
	return n, err
}

func boundedHandler(next http.Handler, logger *slog.Logger) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		started := time.Now()
		wrapped := &accessWriter{ResponseWriter: w}
		r.Body = http.MaxBytesReader(wrapped, r.Body, MaxBodyBytes)
		defer func() {
			if recover() != nil && wrapped.status == 0 {
				WriteError(wrapped, ServiceUnavailable)
			}
			status := wrapped.status
			if status == 0 {
				status = 200
			}
			logger.Info("enroll_access", "method", safeMethod(r.Method), "path", routeLabel(r.URL.Path), "status", status, "bytes", wrapped.bytes, "latency", time.Since(started))
		}()
		next.ServeHTTP(wrapped, r)
	})
}
func routeLabel(path string) string {
	if _, exists := endpointMethods[path]; exists {
		return path
	}
	return "<unknown>"
}
func safeMethod(method string) string {
	switch method {
	case "GET", "POST", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS":
		return method
	}
	return "<unknown>"
}
