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
	"github.com/codeasier/mtls-router/internal/routermeta"
	"github.com/codeasier/mtls-router/internal/version"
)

var (
	clientCertPEM string
	clientKeyPEM  string
	upstreamCAPEM string
	upstreamURL   string
)

type runFailure struct {
	reason   string
	err      error
	reported bool
}

func (e *runFailure) Error() string { return e.reason }
func (e *runFailure) Unwrap() error { return e.err }

func failRun(reason string, err error) error {
	if err == nil {
		return nil
	}
	return &runFailure{reason: reason, err: err}
}

func main() {
	if err := run(); err != nil {
		logRunFailure(slog.Default(), err)
		os.Exit(1)
	}
}

// Startup errors can embed configured URLs and raw transport details. Emit only
// a closed reason code that remains useful in desktop diagnostics.
func logRunFailure(logger *slog.Logger, err error) {
	reason := "router_failure"
	var failure *runFailure
	if errors.As(err, &failure) {
		if failure.reported {
			return
		}
		reason = failure.reason
		failure.reported = true
	}
	logger.Error("fatal", "reason", reason)
}

func run() (runErr error) {
	if handled, err := handleMetaFlags(os.Args[1:]); handled || err != nil {
		return failRun("arguments_invalid", err)
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
		return failRun("config_invalid", err)
	}
	if err := cfg.Validate(); err != nil {
		return failRun("config_invalid", err)
	}
	if cfg.Backend {
		return failRun("backend_start_failed", startBackend(cfg.LogPath))
	}

	level := slog.LevelInfo
	if cfg.Debug {
		level = slog.LevelDebug
	}
	writer, closeLog, err := logWriter(cfg.LogPath)
	if err != nil {
		return failRun("log_open_failed", err)
	}
	logger := slog.New(slog.NewTextHandler(writer, &slog.HandlerOptions{Level: level}))
	slog.SetDefault(logger)
	defer func() {
		// Report while the configured writer is still open. main() observes the
		// reported marker and does not emit a duplicate after closeLog.
		if runErr != nil {
			logRunFailure(logger, runErr)
		}
		closeLog()
	}()

	transport, err := proxy.NewMTLSTransport(clientCertPEM, clientKeyPEM, upstreamCAPEM, proxy.WithTLSMin(cfg.TLSMin))
	if err != nil {
		return failRun("tls_material_invalid", err)
	}

	probeOptions := health.ProbeOptions{
		UpstreamURL: cfg.UpstreamURL,
		ClientCert:  clientCertPEM,
		ClientKey:   clientKeyPEM,
		UpstreamCA:  upstreamCAPEM,
		TLSMin:      cfg.TLSMin,
		Timeout:     cfg.Timeout,
	}
	prober, err := health.NewProber(probeOptions)
	if err != nil {
		return failRun("probe_setup_failed", err)
	}
	defer prober.Close()
	if err := prober.Probe(); err != nil {
		return failRun("upstream_probe_failed", err)
	}

	parsedUpstream, err := url.Parse(cfg.UpstreamURL)
	if err != nil {
		return failRun("config_invalid", err)
	}
	reverseProxy := proxy.New(proxy.Options{
		Upstream:  parsedUpstream,
		Transport: transport,
		ErrorLog:  logger,
	})

	startedAt := time.Now().UTC().Format(time.RFC3339)
	mux := http.NewServeMux()
	mux.Handle("/version", routermeta.VersionHandler(routermeta.InfoProviderFunc(func() map[string]any {
		return map[string]any{"started_at": startedAt}
	})))
	mux.Handle("/health", routermeta.HealthHandler(prober.Probe))
	mux.Handle("/", withAccessLog(reverseProxy, logger))

	server := &http.Server{
		Addr:              cfg.ListenAddr,
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
	}

	errCh := make(chan error, 1)
	go func() {
		logListening(logger, cfg.ListenAddr, parsedUpstream)
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
			return failRun("listen_failed", err)
		}
		return nil
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	return failRun("shutdown_failed", server.Shutdown(ctx))
}

func logListening(logger *slog.Logger, addr string, upstream *url.URL) {
	origin := (&url.URL{Scheme: upstream.Scheme, Host: upstream.Host}).String()
	logger.Info("listening", "addr", addr, "upstream", origin)
}

func startBackend(logPath string) error {
	exePath, err := os.Executable()
	if err != nil {
		return err
	}
	if logPath == "" {
		logPath, err = background.PrepareSessionLogPath(background.DefaultLogPath(exePath), time.Now())
		if err != nil {
			return err
		}
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
	writer, closeLog, err := background.OpenBoundedLogWriter(logPath, background.DefaultMaxLogBytes)
	if err != nil {
		return nil, nil, err
	}
	return writer, closeLog, nil
}

func withAccessLog(next http.Handler, logger *slog.Logger) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		recorder := &mlog.ResponseRecorder{ResponseWriter: w}
		next.ServeHTTP(recorder, r)
		mlog.AccessLog(logger, r, recorder, start)
	})
}

func handleMetaFlags(args []string) (bool, error) {
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch arg {
		case "-version", "--version":
			fmt.Fprintf(os.Stdout, "mtls-router %s\n", version.Version)
			return true, nil
		case "-help", "--help", "-h":
			printUsage()
			return true, nil
		case "-listen", "--listen", "-upstream", "--upstream", "-tls-min", "--tls-min", "-timeout", "--timeout", "-log", "--log":
			i++
		default:
			if len(arg) > 0 && arg[0] != '-' {
				return false, fmt.Errorf("unexpected argument %q; mtls-router only accepts flags", arg)
			}
		}
	}
	return false, nil
}

func printUsage() {
	fmt.Fprintf(os.Stdout, "mtls-router %s\n\n", version.Version)
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
