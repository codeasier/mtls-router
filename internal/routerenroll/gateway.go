package routerenroll

import (
	"context"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"os"
	"path/filepath"
)

// ServeControl binds both listeners before serving. The gateway listener is
// optional for enrollment-only deployments; no public HTTP signing fallback is
// created. A stale/occupied path is never removed to take over an unknown peer.
func ServeControl(ctx context.Context, public *http.Server, socket string, gateway http.Handler, logger *slog.Logger) error {
	if socket == "" {
		return Serve(ctx, public)
	}
	if ValidateListen(public.Addr) != nil || !filepath.IsAbs(socket) {
		return ErrListenInvalid
	}
	info, err := os.Lstat(filepath.Dir(socket))
	if err != nil || !info.IsDir() || !privateOwner(info) || info.Mode().Perm()&0027 != 0 {
		return ErrListenInvalid
	}
	if _, err := os.Lstat(socket); !errors.Is(err, os.ErrNotExist) {
		return ErrListen
	}
	tcp, err := net.Listen("tcp", public.Addr)
	if err != nil {
		return ErrListen
	}
	defer tcp.Close()
	unix, err := net.ListenUnix("unix", &net.UnixAddr{Name: socket, Net: "unix"})
	if err != nil {
		return ErrListen
	}
	defer unix.Close()
	if os.Chmod(socket, 0660) != nil {
		return ErrListen
	}
	private, err := NewHTTPServer(DefaultListen, gateway, logger)
	if err != nil {
		return err
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	results := make(chan error, 2)
	go func() { results <- serveListener(ctx, public, tcp) }()
	go func() { results <- serveListener(ctx, private, unix) }()
	first := <-results
	cancel()
	second := <-results
	if first != nil {
		return first
	}
	return second
}
