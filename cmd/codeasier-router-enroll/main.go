// codeasier-router-enroll is the single-writer, loopback certificate service.
package main

import (
	"context"
	"flag"
	"io"
	"log/slog"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/codeasier/mtls-router/internal/routerenroll"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	output := io.Writer(os.Stderr)
	if len(os.Args) > 1 && os.Args[1] == "gateway-load" {
		output = os.Stdout
	}
	os.Exit(run(ctx, os.Args[1:], os.LookupEnv, output))
}

func run(ctx context.Context, args []string, env func(string) (string, bool), output io.Writer) int {
	if len(args) > 0 && args[0] == "journal-init" {
		return runJournalInit(args[1:], env, output)
	}
	if len(args) > 0 && args[0] == "admin" {
		return runAdmin(ctx, args[1:], env, output)
	}
	if len(args) > 0 && args[0] == "gateway-load" {
		return runGatewayLoad(ctx, args[1:], env, os.Stdin, output)
	}
	logger := slog.New(slog.NewJSONHandler(output, nil))
	cfg := routerenroll.LoadConfig(env)
	flags := flag.NewFlagSet("codeasier-router-enroll", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	flags.StringVar(&cfg.Listen, "listen", cfg.Listen, "loopback listen address")
	flags.StringVar(&cfg.GatewaySocket, "gateway-socket", cfg.GatewaySocket, "protected mTLS gateway Unix socket")
	flags.StringVar(&cfg.RootCRLFile, "root-crl-file", cfg.RootCRLFile, "offline root signed CRL file")
	flags.StringVar(&cfg.GatewaysFile, "gateways-file", cfg.GatewaysFile, "private two-gateway SSH configuration")
	flags.StringVar(&cfg.StateDir, "state-dir", cfg.StateDir, "private state directory")
	flags.StringVar(&cfg.JournalDir, "journal-dir", cfg.JournalDir, "independent private journal directory")
	flags.StringVar(&cfg.TokenKeyFile, "token-key-file", cfg.TokenKeyFile, "private token encryption key file")
	flags.StringVar(&cfg.ClientID, "client-id", cfg.ClientID, "GitCode application ID")
	flags.StringVar(&cfg.ClientSecretFile, "client-secret-file", cfg.ClientSecretFile, "private OAuth secret file")
	flags.StringVar(&cfg.OrgPath, "org-path", cfg.OrgPath, "server organization path")
	flags.StringVar(&cfg.ExpectedOrgID, "org-id", cfg.ExpectedOrgID, "expected stable organization ID")
	flags.StringVar(&cfg.IssuerCertFile, "issuer-cert-file", cfg.IssuerCertFile, "restricted issuing CA certificate file")
	flags.StringVar(&cfg.IssuerKeyFile, "issuer-key-file", cfg.IssuerKeyFile, "private issuing CA key file")
	flags.StringVar(&cfg.RootCAFile, "root-ca-file", cfg.RootCAFile, "offline root public certificate file")
	if flags.Parse(args) != nil || flags.NArg() != 0 || cfg.Validate() != nil {
		logger.Error("enroll_config_invalid")
		return 2
	}
	store, err := routerenroll.OpenStore(cfg.StateDir, cfg.TokenKeyFile, cfg.JournalDir)
	if err != nil {
		logger.Error("enroll_storage_unavailable")
		return 1
	}
	defer store.Close()
	provider, err := routerenroll.NewGitCode(cfg)
	if err != nil {
		logger.Error("enroll_provider_config_invalid")
		return 2
	}
	issuer, err := routerenroll.LoadIssuer(cfg.IssuerCertFile, cfg.IssuerKeyFile, cfg.RootCAFile, time.Now())
	if err != nil {
		logger.Error("enroll_issuer_invalid")
		return 2
	}
	if _, err := store.ActivatePolicy(ctx, cfg); err != nil {
		logger.Error("enroll_policy_unavailable")
		return 1
	}
	service := routerenroll.NewService(store, provider, issuer)
	service.SetLogger(logger)
	targets, err := routerenroll.ReadGatewayTargets(cfg.GatewaysFile)
	if err != nil {
		logger.Error("enroll_gateways_invalid")
		return 2
	}
	publisher := &routerenroll.Publisher{Store: store, Issuer: issuer, RootCRLFile: cfg.RootCRLFile, Targets: targets, Loader: routerenroll.SSHGatewayLoader{}}
	if _, err := publisher.Publish(ctx, time.Now()); err != nil {
		logger.Error("enroll_crl_unavailable")
		return 1
	}
	service.SetPublisher(publisher)
	server, err := routerenroll.NewHTTPServer(cfg.Listen, service.Handler(), logger)
	if err != nil {
		logger.Error("enroll_config_invalid")
		return 2
	}
	ctx, cancel := context.WithCancel(ctx)
	maintenance := make(chan struct{})
	go func() { defer close(maintenance); service.RunMaintenance(ctx, logger) }()
	err = routerenroll.ServeControl(ctx, server, cfg.GatewaySocket, service.GatewayHandler(), logger)
	cancel()
	<-maintenance
	if err != nil {
		logger.Error("enroll_service_failed")
		return 1
	}
	return 0
}
