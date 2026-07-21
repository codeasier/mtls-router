package agent

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"unicode"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const (
	RedactedAPIKey = "<redacted-api-key>"
	maxRenderSize  = 2 << 20
)

// Fragment is one key-redacted manager-owned Agent configuration fragment.
type Fragment struct {
	Agent   Kind   `json:"agent"`
	Role    string `json:"role"`
	Path    string `json:"path"`
	Format  Format `json:"format"`
	Content string `json:"content"`
}

// RenderResult contains the normalized canonical model config and no existing
// user file content.
type RenderResult struct {
	ModelConfig json.RawMessage `json:"model_config"`
	Fragments   []Fragment      `json:"fragments"`
}

type renderInput struct {
	config        *modelconfig.Config
	routerBaseURL string
	apiBaseURL    string
	key           string
	ownRootModel  bool
}

func (d Detector) targetStates() (map[Kind]State, error) {
	home := d.HomeDir
	if home == "" {
		var err error
		home, err = os.UserHomeDir()
		if err != nil {
			return nil, err
		}
	}
	getenv := d.Getenv
	if getenv == nil {
		getenv = os.Getenv
	}
	claude := ClaudePaths(home, getenv("CLAUDE_CONFIG_DIR"))
	opencode := OpenCodePaths(home, getenv("OPENCODE_CONFIG"))
	codex := CodexPaths(home, getenv("CODEX_HOME"))
	format := FormatJSON
	if filepath.Ext(opencode.ConfigPath) == ".jsonc" {
		format = FormatJSONC
	}
	return map[Kind]State{
		ClaudeCode: {Agent: ClaudeCode, Path: claude.ConfigPath, Format: FormatJSON},
		OpenCode:   {Agent: OpenCode, Path: opencode.ConfigPath, Format: format},
		Codex:      {Agent: Codex, Path: codex.ConfigPath, AuthPath: codex.AuthPath, Format: FormatTOML},
	}, nil
}

// Render verifies the catalog scope and renders managed fragments without
// reading or changing Agent configuration files.
func (s *Service) Render(ctx context.Context, selected []Kind, catalogToken string, rawConfig json.RawMessage) (RenderResult, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	lock, err := acquireTransactionLock(ctx, s.stateDir)
	if err != nil {
		return RenderResult{}, err
	}
	defer lock.release()
	if err := contextError(ctx); err != nil {
		return RenderResult{}, err
	}
	if err := s.ensureExistingSigner(); err != nil {
		return RenderResult{}, err
	}
	claims, err := s.verifyCatalogToken(catalogToken)
	if err != nil {
		return RenderResult{}, err
	}
	normalized, err := normalizeSelection(selected)
	if err != nil {
		return RenderResult{}, err
	}
	if !sameModelAgents(normalized, claims.Agents) {
		return RenderResult{}, operationError(CodeModelCatalogStale, "model catalog Agent scope changed")
	}
	config, err := modelconfig.Decode(rawConfig, claims.Agents, claims.Models)
	if err != nil {
		return RenderResult{}, operationError(CodeModelConfigInvalid, "canonical model configuration is invalid")
	}
	canonical, err := modelconfig.Canonical(config)
	if err != nil {
		return RenderResult{}, operationError(CodeModelConfigInvalid, "canonical model configuration is invalid")
	}
	apiBaseURL, err := apiURL(claims.RouterBaseURL)
	if err != nil {
		return RenderResult{}, operationError(CodeModelCatalogStale, "model catalog router address is invalid")
	}
	paths, err := s.detector.targetStates()
	if err != nil {
		return RenderResult{}, operationError(CodeConfigInvalid, "Agent target paths are unavailable")
	}
	input := renderInput{config: config, routerBaseURL: claims.RouterBaseURL, apiBaseURL: apiBaseURL, key: RedactedAPIKey}
	fragments, err := renderFragments(normalized, paths, input)
	if err != nil {
		return RenderResult{}, err
	}
	return RenderResult{ModelConfig: canonical, Fragments: fragments}, nil
}

func renderFragments(selected []Kind, states map[Kind]State, input renderInput) ([]Fragment, error) {
	fragments := make([]Fragment, 0, len(selected)+1)
	total := len(input.configString())
	for _, kind := range selected {
		state := states[kind]
		if !safeDisplayPath(state.Path) || (state.AuthPath != "" && !safeDisplayPath(state.AuthPath)) {
			return nil, operationError(CodeConfigInvalid, "Agent target path contains unsafe characters")
		}
		var rendered []Fragment
		var err error
		switch kind {
		case ClaudeCode:
			content, renderErr := renderClaudeFragment(input.config.Claude, input.routerBaseURL, input.key)
			err = renderErr
			rendered = []Fragment{{Agent: kind, Role: "config", Path: state.Path, Format: FormatJSON, Content: string(content)}}
		case OpenCode:
			content, renderErr := renderOpenCodeFragment(input.config.OpenCode, input.apiBaseURL, input.key)
			err = renderErr
			rendered = []Fragment{{Agent: kind, Role: "config", Path: state.Path, Format: FormatJSON, Content: string(content)}}
		case Codex:
			configContent, configErr := renderCodexFragment(input.config.Codex, input.apiBaseURL)
			authContent, authErr := renderCodexAuthFragment(input.key, nil)
			if configErr != nil {
				err = configErr
			} else {
				err = authErr
			}
			rendered = []Fragment{
				{Agent: kind, Role: "config", Path: state.Path, Format: FormatTOML, Content: string(configContent)},
				{Agent: kind, Role: "auth", Path: state.AuthPath, Format: FormatJSON, Content: string(authContent)},
			}
		}
		if err != nil {
			return nil, operationError(CodeModelConfigInvalid, "Agent managed fragment could not be rendered")
		}
		for _, fragment := range rendered {
			total += len(fragment.Path) + len(fragment.Content)
			if total > maxRenderSize {
				return nil, operationError(CodeModelConfigInvalid, "Agent render output exceeds the size limit")
			}
			fragments = append(fragments, fragment)
		}
	}
	return fragments, nil
}

func (i renderInput) configString() string {
	value, _ := modelconfig.Canonical(i.config)
	return string(value)
}

func sameModelAgents(selected []Kind, claims []modelconfig.Agent) bool {
	if len(selected) != len(claims) {
		return false
	}
	for i := range selected {
		if modelAgent(selected[i]) != claims[i] {
			return false
		}
	}
	return true
}

func apiURL(routerBaseURL string) (string, error) {
	parsed, err := url.Parse(routerBaseURL)
	if err != nil || parsed.Scheme != "http" || parsed.Host == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || (parsed.Path != "" && parsed.Path != "/") {
		return "", errors.New("invalid router URL")
	}
	ip := net.ParseIP(parsed.Hostname())
	if ip == nil || !ip.IsLoopback() || parsed.Port() == "" {
		return "", errors.New("router URL is not numeric loopback")
	}
	parsed.Path = "/v1"
	return strings.TrimSuffix(parsed.String(), "/"), nil
}

func safeDisplayPath(path string) bool {
	if path == "" || !filepath.IsAbs(path) {
		return false
	}
	for _, r := range path {
		if unicode.IsControl(r) || r == '\u001b' {
			return false
		}
	}
	return true
}

func marshalIndentedJSON(value any) ([]byte, error) {
	var output bytes.Buffer
	encoder := json.NewEncoder(&output)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(value); err != nil {
		return nil, err
	}
	return output.Bytes(), nil
}
