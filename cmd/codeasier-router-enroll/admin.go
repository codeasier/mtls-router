package main

import (
	"context"
	"encoding/json"
	"flag"
	"io"
	"time"

	"github.com/codeasier/mtls-router/internal/routerenroll"
)

func runAdmin(ctx context.Context, args []string, env func(string) (string, bool), output io.Writer) int {
	fail := func(code string) int { _ = json.NewEncoder(output).Encode(map[string]string{"code": code}); return 2 }
	if len(args) == 0 || (args[0] != "list" && args[0] != "revoke" && args[0] != "backup" && args[0] != "publication") {
		return fail("enroll_admin_request_invalid")
	}
	state, _ := env("MTLS_ENROLL_STATE_DIR")
	journal, _ := env("MTLS_ENROLL_JOURNAL_DIR")
	flags := flag.NewFlagSet("admin", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	flags.StringVar(&state, "state-dir", state, "private state directory")
	flags.StringVar(&journal, "journal-dir", journal, "independent private journal directory")
	cfg := routerenroll.LoadConfig(env)
	flags.StringVar(&cfg.IssuerCertFile, "issuer-cert-file", cfg.IssuerCertFile, "public issuing CA")
	flags.StringVar(&cfg.RootCRLFile, "root-crl-file", cfg.RootCRLFile, "offline root CRL")
	flags.StringVar(&cfg.GatewaysFile, "gateways-file", cfg.GatewaysFile, "private gateway configuration")
	var serial, device, user string
	flags.StringVar(&serial, "serial", "", "certificate serial")
	flags.StringVar(&device, "device", "", "stable device ID")
	flags.StringVar(&user, "user", "", "stable GitCode user ID")
	limit := flags.Int("limit", 100, "maximum public summaries, 1–100")
	outputFile := flags.String("output", "", "new private SQLite backup file")
	if flags.Parse(args[1:]) != nil || flags.NArg() != 0 || state == "" || journal == "" {
		return fail("enroll_admin_request_invalid")
	}
	var target routerenroll.RevocationTarget
	count := 0
	for _, candidate := range []routerenroll.RevocationTarget{{Kind: "serial", ID: serial}, {Kind: "device", ID: device}, {Kind: "user", ID: user}} {
		if candidate.ID != "" {
			target = candidate
			count++
		}
	}
	if (args[0] == "revoke" && count != 1) || (args[0] != "revoke" && count != 0) || (args[0] == "backup" && *outputFile == "") || (args[0] != "backup" && *outputFile != "") {
		return fail("enroll_admin_request_invalid")
	}
	admin, err := routerenroll.OpenAdmin(state, journal)
	if err != nil {
		return fail("enroll_admin_unavailable")
	}
	defer admin.Close()
	var result any
	if args[0] == "list" {
		result, err = admin.List(ctx, *limit)
	} else if args[0] == "backup" {
		result, err = admin.Backup(ctx, *outputFile)
	} else if args[0] == "publication" {
		result, err = admin.PublicationStatus(ctx, cfg.IssuerCertFile, cfg.RootCRLFile, cfg.GatewaysFile, time.Now())
	} else {
		result, err = admin.Revoke(ctx, target, time.Now())
	}
	if err != nil {
		return fail("enroll_admin_request_failed")
	}
	if json.NewEncoder(output).Encode(result) != nil {
		return 1
	}
	return 0
}

func runJournalInit(args []string, env func(string) (string, bool), output io.Writer) int {
	directory, _ := env("MTLS_ENROLL_JOURNAL_DIR")
	flags := flag.NewFlagSet("journal-init", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	flags.StringVar(&directory, "journal-dir", directory, "new independent private journal directory")
	if flags.Parse(args) != nil || flags.NArg() != 0 || directory == "" || routerenroll.InitJournal(directory) != nil {
		_ = json.NewEncoder(output).Encode(map[string]string{"code": "enroll_journal_init_failed"})
		return 2
	}
	_ = json.NewEncoder(output).Encode(map[string]string{"status": "initialized"})
	return 0
}
