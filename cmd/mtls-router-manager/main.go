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
		fmt.Fprintln(os.Stderr, "mtls-router-manager:", err)
		os.Exit(1)
	}
}

func run(ctx context.Context, args []string, input io.Reader, output, stderr io.Writer) error {
	if len(args) == 0 || args[0] != "serve" {
		return errors.New("the only supported command is serve")
	}

	flags := flag.NewFlagSet("serve", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	var config app.Config
	flags.StringVar(&config.RouterPath, "router-sidecar", "", "path to the mtls-router sidecar")
	flags.StringVar(&config.ListenAddr, "listen", "127.0.0.1:19099", "router localhost listen address")
	flags.StringVar(&config.DesktopSession, "desktop-session", "", "desktop session identifier")
	flags.IntVar(&config.ParentIdentity.PID, "parent-pid", 0, "desktop parent process ID")
	flags.StringVar(&config.ParentIdentity.StartedAt, "parent-start", "", "desktop parent process start identity")
	flags.StringVar(&config.ParentIdentity.Executable, "parent-executable", "", "desktop parent executable identity")
	if err := flags.Parse(args[1:]); err != nil || flags.NArg() != 0 {
		return errors.New("invalid serve flags")
	}
	if err := validateDesktopFlags(config); err != nil {
		return err
	}
	simplify, err := modelcatalog.ParseSimplify()
	if err != nil {
		return err
	}
	config.Stderr = stderr

	manager, err := app.New(config, simplify)
	if err != nil {
		return err
	}
	return manager.Serve(ctx, input, output)
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
	if _, err := process.NormalizeExecutable(config.ParentIdentity.Executable); err != nil {
		return errors.New("parent executable identity is invalid")
	}
	return nil
}
