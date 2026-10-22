package main

import (
	"context"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/codeasier/mtls-router/internal/routerenroll"
)

// The SSH account is restricted to this fixed subcommand. Its config contains
// public trust files and a gateway ID, never any enrollment signing/token key.
func runGatewayLoad(ctx context.Context, args []string, env func(string) (string, bool), input io.Reader, output io.Writer) int {
	get := func(key, fallback string) string {
		if value, ok := env(key); ok {
			return value
		}
		return fallback
	}
	cfg := routerenroll.GatewayLoadConfig{ID: get("MTLS_GATEWAY_ID", ""), StateDir: get("MTLS_GATEWAY_STATE_DIR", "/var/lib/codeasier-router-gateway"), IssuerCertFile: get("MTLS_GATEWAY_ISSUER_CERT_FILE", "/etc/codeasier-router-gateway/issuing-cert.pem"), RootCAFile: get("MTLS_GATEWAY_ROOT_CA_FILE", "/etc/codeasier-router-gateway/root-cert.pem"), NginxBinary: get("MTLS_GATEWAY_NGINX_BINARY", "/usr/sbin/nginx")}
	pidFile := get("MTLS_GATEWAY_NGINX_PID_FILE", "/run/nginx.pid")
	flags := flag.NewFlagSet("gateway-load", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	flags.StringVar(&cfg.ID, "id", cfg.ID, "gateway stable ID")
	flags.StringVar(&cfg.StateDir, "state-dir", cfg.StateDir, "private gateway state directory")
	flags.StringVar(&cfg.IssuerCertFile, "issuer-cert-file", cfg.IssuerCertFile, "pinned issuing CA")
	flags.StringVar(&cfg.RootCAFile, "root-ca-file", cfg.RootCAFile, "pinned offline root")
	flags.StringVar(&cfg.NginxBinary, "nginx-binary", cfg.NginxBinary, "absolute nginx executable")
	flags.StringVar(&pidFile, "nginx-pid-file", pidFile, "absolute nginx master PID file")
	if flags.Parse(args) != nil || flags.NArg() != 0 {
		return 2
	}
	bundle, err := routerenroll.DecodeGatewayBundle(input)
	if err != nil {
		return 1
	}
	ack, err := routerenroll.ApplyGatewayBundle(ctx, cfg, bundle, routerenroll.NginxRuntime{Binary: cfg.NginxBinary, PidFile: pidFile}, time.Now())
	if err != nil {
		return 1
	}
	if json.NewEncoder(output).Encode(ack) != nil {
		return 1
	}
	return 0
}
