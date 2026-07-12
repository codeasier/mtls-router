package agent

import (
	"os"
	"path/filepath"
	"testing"
)

func TestClaudePathsRespectsConfigDir(t *testing.T) {
	home := t.TempDir()
	override := filepath.Join(t.TempDir(), "claude-profile")
	if got := ClaudePaths(home, override).ConfigPath; got != filepath.Join(override, "settings.json") {
		t.Fatalf("Claude path = %q", got)
	}
	if got := ClaudePaths(home, "").ConfigPath; got != filepath.Join(home, ".claude", "settings.json") {
		t.Fatalf("default Claude path = %q", got)
	}
}

func TestOpenCodePathsRespectsOverrideAndFallbackOrder(t *testing.T) {
	home := t.TempDir()
	override := filepath.Join(t.TempDir(), "custom.jsonc")
	if got := OpenCodePaths(home, override).ConfigPath; got != override {
		t.Fatalf("override path = %q", got)
	}

	dir := filepath.Join(home, ".config", "opencode")
	jsonPath := filepath.Join(dir, "opencode.json")
	jsoncPath := filepath.Join(dir, "opencode.jsonc")
	if got := OpenCodePaths(home, "").ConfigPath; got != jsonPath {
		t.Fatalf("missing fallback = %q", got)
	}
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(jsoncPath, []byte("{}"), 0600); err != nil {
		t.Fatal(err)
	}
	if got := OpenCodePaths(home, "").ConfigPath; got != jsoncPath {
		t.Fatalf("JSONC fallback = %q", got)
	}
	if err := os.WriteFile(jsonPath, []byte("{}"), 0600); err != nil {
		t.Fatal(err)
	}
	if got := OpenCodePaths(home, "").ConfigPath; got != jsonPath {
		t.Fatalf("JSON preference = %q", got)
	}
}

func TestCodexPathsRespectsHomeOverride(t *testing.T) {
	home := t.TempDir()
	override := filepath.Join(t.TempDir(), "codex-profile")
	got := CodexPaths(home, override)
	if got.ConfigPath != filepath.Join(override, "config.toml") || got.AuthPath != filepath.Join(override, "auth.json") {
		t.Fatalf("override paths = %#v", got)
	}
	got = CodexPaths(home, "")
	if got.ConfigPath != filepath.Join(home, ".codex", "config.toml") || got.AuthPath != filepath.Join(home, ".codex", "auth.json") {
		t.Fatalf("default paths = %#v", got)
	}
}
