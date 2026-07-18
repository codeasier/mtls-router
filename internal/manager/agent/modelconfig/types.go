// Package modelconfig defines the authoritative, key-free Agent model
// configuration document.
package modelconfig

import "fmt"

const (
	Version        = 1
	MaxConfigSize  = 2 << 20
	MaxSafeInteger = int64(9007199254740991)
)

type Agent string

const (
	Claude   Agent = "claude"
	OpenCode Agent = "opencode"
	Codex    Agent = "codex"
)

type Config struct {
	Version  int             `json:"version"`
	Claude   *ClaudeConfig   `json:"claude,omitempty"`
	OpenCode *OpenCodeConfig `json:"opencode,omitempty"`
	Codex    *CodexConfig    `json:"codex,omitempty"`
}

type Model struct {
	Model string  `json:"model"`
	Name  *string `json:"name,omitempty"`
}

type ClaudeRole struct {
	InheritPrimary bool
	Selection      *Model
}

func (r ClaudeRole) MarshalJSON() ([]byte, error) {
	if r.InheritPrimary && r.Selection == nil {
		return []byte(`{"inherit_primary":true}`), nil
	}
	if !r.InheritPrimary && r.Selection != nil {
		return marshalJCS(r.Selection)
	}
	return nil, fmt.Errorf("invalid Claude role")
}

type ClaudeConfig struct {
	Primary Model             `json:"primary"`
	Haiku   ClaudeRole        `json:"haiku"`
	Sonnet  ClaudeRole        `json:"sonnet"`
	Opus    ClaudeRole        `json:"opus"`
	Extra   map[string]string `json:"extra,omitempty"`
}

type OpenCodeConfig struct {
	DefaultModel string                         `json:"default_model"`
	Models       map[string]OpenCodeModelConfig `json:"models"`
}

type OpenCodeModelConfig struct {
	Name        *string        `json:"name,omitempty"`
	Reasoning   *bool          `json:"reasoning,omitempty"`
	Attachment  *bool          `json:"attachment,omitempty"`
	ToolCall    *bool          `json:"tool_call,omitempty"`
	Temperature *bool          `json:"temperature,omitempty"`
	Limit       *Limit         `json:"limit,omitempty"`
	Modalities  *Modalities    `json:"modalities,omitempty"`
	Interleaved any            `json:"interleaved,omitempty"`
	Options     map[string]any `json:"options,omitempty"`
	Extra       map[string]any `json:"extra,omitempty"`
}

type Limit struct {
	Context int64  `json:"context"`
	Input   *int64 `json:"input,omitempty"`
	Output  int64  `json:"output"`
}

type Modalities struct {
	Input  []string `json:"input,omitempty"`
	Output []string `json:"output,omitempty"`
}

type InterleavedField struct {
	Field string `json:"field"`
}

type CodexConfig struct {
	Model                 string         `json:"model"`
	ReasoningEffort       *string        `json:"reasoning_effort,omitempty"`
	ReasoningSummary      *string        `json:"reasoning_summary,omitempty"`
	Verbosity             *string        `json:"verbosity,omitempty"`
	ContextWindow         *int64         `json:"context_window,omitempty"`
	AutoCompactTokenLimit *int64         `json:"auto_compact_token_limit,omitempty"`
	Extra                 map[string]any `json:"extra,omitempty"`
}

// ValidationError is safe for protocol validation details. It contains no
// rejected value.
type ValidationError struct {
	Path string
	Rule string
}

func (e *ValidationError) Error() string { return "invalid model config at " + e.Path + ": " + e.Rule }

func invalid(path, rule string) error { return &ValidationError{Path: path, Rule: rule} }
