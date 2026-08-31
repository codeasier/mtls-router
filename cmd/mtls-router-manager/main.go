// mtls-router-manager is the stdin/stdout control plane for mtls-router.
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"

	"github.com/codeasier/mtls-router/internal/manager/app"
	"github.com/codeasier/mtls-router/internal/manager/modelcatalog"
	"github.com/codeasier/mtls-router/internal/manager/process"
)

func main() {
	if err := run(context.Background(), os.Args[1:], os.Stdin, os.Stdout, os.Stderr); err != nil {
		writeBootstrapFailure(os.Stderr, err)
		os.Exit(1)
	}
}

type processFailure struct {
	Stage string
	Code  string
	Err   error
}

func (e *processFailure) Error() string { return e.Err.Error() }
func (e *processFailure) Unwrap() error { return e.Err }

func processError(stage, code string, err error) error {
	return &processFailure{Stage: stage, Code: code, Err: err}
}

func writeBootstrapFailure(stderr io.Writer, err error) {
	failure := &processFailure{Stage: "unexpected_exit", Code: "MANAGER_PROTOCOL_FAILED"}
	var classified *processFailure
	if errors.As(err, &classified) {
		failure = classified
	}
	_, _ = fmt.Fprintf(stderr, `{"schema_version":1,"kind":"manager_bootstrap_failure","stage":%q,"code":%q}`+"\n", failure.Stage, failure.Code)
}

func run(ctx context.Context, args []string, input io.Reader, output, stderr io.Writer) error {
	if len(args) == 0 || args[0] != "serve" {
		return processError("handshake", "MANAGER_COMMAND_INVALID", errors.New("the only supported command is serve"))
	}

	flags := flag.NewFlagSet("serve", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	var config app.Config
	flags.StringVar(&config.RouterPath, "router-sidecar", "", "path to the mtls-router sidecar")
	flags.StringVar(&config.ListenAddr, "listen", "127.0.0.1:19099", "router localhost listen address")
	flags.StringVar(&config.DesktopSession, "desktop-session", "", "desktop session identifier")
	flags.StringVar(&config.InstallationID, "desktop-installation", "", "durable desktop installation identifier")
	flags.IntVar(&config.PackageGeneration, "package-generation", 0, "desktop package generation")
	flags.IntVar(&config.ParentIdentity.PID, "parent-pid", 0, "desktop parent process ID")
	flags.StringVar(&config.ParentIdentity.StartedAt, "parent-start", "", "desktop parent process start identity")
	flags.StringVar(&config.ParentIdentity.Executable, "parent-executable", "", "desktop parent executable identity")
	if err := flags.Parse(args[1:]); err != nil || flags.NArg() != 0 {
		return processError("handshake", "MANAGER_FLAGS_INVALID", errors.New("invalid serve flags"))
	}
	if err := validateDesktopFlags(config); err != nil {
		return processError("handshake", "MANAGER_FLAGS_INVALID", err)
	}
	simplify, err := modelcatalog.ParseSimplify()
	if err != nil {
		return processError("handshake", "MANAGER_CONFIG_INVALID", err)
	}
	config.Stderr = stderr

	manager, err := app.New(config, simplify)
	if err != nil {
		return processError("handshake", "MANAGER_INIT_FAILED", err)
	}
	if err := manager.Serve(ctx, input, output); err != nil {
		return processError("unexpected_exit", "MANAGER_PROTOCOL_FAILED", err)
	}
	return nil
}

func validateDesktopFlags(config app.Config) error {
	parentValues := 0
	if config.ParentIdentity.PID > 0 {
		parentValues++
	}
	if config.ParentIdentity.StartedAt != "" {
		parentValues++
	}
	if config.ParentIdentity.Executable != "" {
		parentValues++
	}
	if parentValues == 0 && config.DesktopSession == "" {
		return nil
	}
	if parentValues != 3 || config.DesktopSession == "" {
		return errors.New("desktop-session and complete parent identity must be supplied together")
	}
	if config.InstallationID == "" || config.PackageGeneration < 1 {
		return errors.New("desktop-installation and package-generation must be supplied with a desktop session")
	}
	if _, err := process.NormalizeExecutable(config.ParentIdentity.Executable); err != nil {
		return errors.New("parent executable identity is invalid")
	}
	return nil
}
