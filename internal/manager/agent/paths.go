// Package agent detects supported agents and inspects their configuration.
package agent

import "path/filepath"

// Kind identifies a supported agent.
type Kind string

const (
	ClaudeCode Kind = "claude"
	OpenCode   Kind = "opencode"
	Codex      Kind = "codex"
)

// Paths contains the files used to configure an agent. AuthPath is set only
// for agents that keep authentication separately from their main config.
type Paths struct {
	ConfigPath string
	AuthPath   string
}

// ClaudePaths applies the CLAUDE_CONFIG_DIR semantics used by the setup
// scripts. An empty override selects the default under the user's home.
func ClaudePaths(home, configDir string) Paths {
	if configDir == "" {
		configDir = filepath.Join(home, ".claude")
	}
	return Paths{ConfigPath: filepath.Join(configDir, "settings.json")}
}

// OpenCodePaths applies the OPENCODE_CONFIG and JSON-before-JSONC fallback
// semantics used by the setup scripts.
func OpenCodePaths(home, configOverride string) Paths {
	if configOverride != "" {
		return Paths{ConfigPath: configOverride}
	}

	dir := filepath.Join(home, ".config", "opencode")
	jsonPath := filepath.Join(dir, "opencode.json")
	jsoncPath := filepath.Join(dir, "opencode.jsonc")
	if isRegularFile(jsonPath) {
		return Paths{ConfigPath: jsonPath}
	}
	if isRegularFile(jsoncPath) {
		return Paths{ConfigPath: jsoncPath}
	}
	return Paths{ConfigPath: jsonPath}
}

// CodexPaths applies CODEX_HOME semantics used by both setup scripts.
func CodexPaths(home, codexHome string) Paths {
	if codexHome == "" {
		codexHome = filepath.Join(home, ".codex")
	}
	return Paths{
		ConfigPath: filepath.Join(codexHome, "config.toml"),
		AuthPath:   filepath.Join(codexHome, "auth.json"),
	}
}
