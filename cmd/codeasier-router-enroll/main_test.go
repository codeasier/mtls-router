package main

import (
	"bytes"
	"context"
	"strings"
	"testing"
)

func TestInvalidConfigurationDoesNotEchoArguments(t *testing.T) {
	for _, args := range [][]string{{"-unknown", "secret-canary"}, {"-listen", "secret-canary:9099"}, {"secret-canary"}} {
		var log bytes.Buffer
		if got := run(context.Background(), args, func(string) (string, bool) { return "", false }, &log); got != 2 || strings.Contains(log.String(), "secret-canary") {
			t.Fatalf("unsafe config result: %d %s", got, log.String())
		}
	}
}

func TestAdminDoesNotEchoInvalidSecretsOrArguments(t *testing.T) {
	for _, args := range [][]string{{"admin", "unknown", "secret-canary"}, {"admin", "revoke", "-unknown", "secret-canary"}, {"admin", "revoke", "--user", "secret-canary", "--device", "secret-canary"}} {
		var log bytes.Buffer
		if got := run(context.Background(), args, func(string) (string, bool) { return "", false }, &log); got != 2 || strings.Contains(log.String(), "secret-canary") {
			t.Fatalf("unsafe admin result: %d %s", got, log.String())
		}
	}
}
