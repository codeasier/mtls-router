// mtls-router forwards plain local HTTP requests to an mTLS upstream.
package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/codeasier/mtls-router/internal/background"
	"github.com/codeasier/mtls-router/internal/config"
	"github.com/codeasier/mtls-router/internal/health"
	mlog "github.com/codeasier/mtls-router/internal/log"
	"github.com/codeasier/mtls-router/internal/proxy"
)

var (
	clientCertPEM string
	clientKeyPEM  string
	upstreamCAPEM string
	upstreamURL   string
	version       = "dev"
)

func main() {
	if err := run(); err != nil {
		slog.Error("fatal", "err", err)
		os.Exit(1)
	}
}

func run() error {
	if handled, err := handleMetaFlags(os.Args[1:]); handled || err != nil {
		return err
	}

	defaults := config.Defaults{
		ListenAddr:  "127.0.0.1:19099",
		UpstreamURL: upstreamURL,
		TLSMin:      "tls1.2",
		Timeout:     10 * time.Second,
		Debug:       false,
	}

	cfg, err := config.Load(defaults, os.Args[1:])
	if err != nil {
		return err
	}
	if err := cfg.Validate(); err != nil {
		return err
	}
	if cfg.Backend {
		return startBackend(cfg.LogPath)
	}

	level := slog.LevelInfo
	if cfg.Debug {
		level = slog.LevelDebug
	}
	writer, closeLog, err := logWriter(cfg.LogPath)
	if err != nil {
		return err
	}
	defer closeLog()
	logger := slog.New(slog.NewTextHandler(writer, &slog.HandlerOptions{Level: level}))
	slog.SetDefault(logger)

	transport, err := proxy.NewMTLSTransport(clientCertPEM, clientKeyPEM, upstreamCAPEM, proxy.WithTLSMin(cfg.TLSMin))
	if err != nil {
		return err
	}

	if err := health.Probe(health.ProbeOptions{
		UpstreamURL: cfg.UpstreamURL,
		ClientCert:  clientCertPEM,
		ClientKey:   clientKeyPEM,
		UpstreamCA:  upstreamCAPEM,
		Timeout:     cfg.Timeout,
	}); err != nil {
		return err
	}

	parsedUpstream, err := url.Parse(cfg.UpstreamURL)
	if err != nil {
		return err
	}
	reverseProxy := proxy.New(proxy.Options{
		Upstream:  parsedUpstream,
		Transport: transport,
		ErrorLog:  logger,
	})
	handler := withAccessLog(proxy.WrapHandler(reverseProxy), logger)

	server := &http.Server{
		Addr:              cfg.ListenAddr,
		Handler:           handler,
		ReadHeaderTimeout: 10 * time.Second,
	}

	errCh := make(chan error, 1)
	go func() {
		logger.Info("listening", "addr", cfg.ListenAddr, "upstream", cfg.UpstreamURL)
		if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
		}
		close(errCh)
	}()

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
	defer signal.Stop(sigCh)

	select {
	case sig := <-sigCh:
		logger.Info("shutdown signal received", "signal", sig.String())
	case err := <-errCh:
		if err != nil {
			return err
		}
		return nil
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	return server.Shutdown(ctx)
}

func startBackend(logPath string) error {
	exePath, err := os.Executable()
	if err != nil {
		return err
	}
	if logPath == "" {
		logPath = background.DefaultLogPath(exePath)
	}
	childArgs := background.ChildArgs(os.Args[1:], logPath)
	pid, err := background.Start(exePath, childArgs, logPath)
	if err != nil {
		return err
	}
	fmt.Fprintf(os.Stdout, "mtls-router started in background, pid=%d, log=%s\n", pid, logPath)
	return nil
}

func logWriter(logPath string) (io.Writer, func(), error) {
	if logPath == "" {
		return os.Stderr, func() {}, nil
	}
	f, err := background.OpenLogFile(logPath)
	if err != nil {
		return nil, nil, err
	}
	return f, func() { _ = f.Close() }, nil
}

func withAccessLog(next http.Handler, logger *slog.Logger) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		recorder := &mlog.ResponseRecorder{ResponseWriter: w}
		next.ServeHTTP(recorder, r)
		mlog.AccessLog(logger, r, recorder, start, nil)
	})
}

func handleMetaFlags(args []string) (bool, error) {
	for _, arg := range args {
		switch arg {
		case "-version", "--version":
			fmt.Fprintf(os.Stdout, "mtls-router %s\n", version)
			return true, nil
		case "-help", "--help", "-h":
			printUsage()
			return true, nil
		}
	}
	return false, nil
}

func printUsage() {
	fmt.Fprintf(os.Stdout, "mtls-router %s\n\n", version)
	fmt.Fprintln(os.Stdout, "Usage: mtls-router [flags]")
	fmt.Fprintln(os.Stdout, "Flags:")
	fmt.Fprintln(os.Stdout, "  -listen string    listen address (default 127.0.0.1:19099)")
	fmt.Fprintln(os.Stdout, "  -upstream string  upstream URL")
	fmt.Fprintln(os.Stdout, "  -tls-min string   minimum TLS version: tls1.2 or tls1.3")
	fmt.Fprintln(os.Stdout, "  -timeout duration upstream probe timeout")
	fmt.Fprintln(os.Stdout, "  -debug            enable debug logging")
	fmt.Fprintln(os.Stdout, "  -backend         run in background")
	fmt.Fprintln(os.Stdout, "  -log string      log file path")
	fmt.Fprintln(os.Stdout, "  -version          print version and exit")
	fmt.Fprintln(os.Stdout, "  -help, -h         print help and exit")
}
