package modelconfig

import (
	"encoding/json"
	"math"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Extension keys fail closed on normalized substrings. The deliberate
// over-match keeps connection and credential-bearing settings manager-owned.
var protectedFragments = []string{"apikey", "credential", "auth", "token", "secret", "password", "bearer", "header", "url", "endpoint", "provider", "connection", "transport", "proxy", "fetch"}

func parseConfig(o object, selected []Agent, catalog []string, requireCatalog bool) (*Config, error) {
	if err := exactKeys(o, "", "version", "claude", "opencode", "codex"); err != nil {
		return nil, err
	}
	v, ok := integer(o["version"])
	if !ok || v != Version {
		return nil, invalid("/version", "version")
	}
	want := map[Agent]bool{}
	for _, a := range selected {
		if a != Claude && a != OpenCode && a != Codex {
			return nil, invalid("", "agent")
		}
		if want[a] {
			return nil, invalid("", "unique_agents")
		}
		want[a] = true
	}
	if len(want) == 0 {
		return nil, invalid("", "agents")
	}
	for _, a := range []Agent{Claude, OpenCode, Codex} {
		_, has := o[string(a)]
		if has != want[a] {
			return nil, invalid("/"+string(a), "section_scope")
		}
	}
	cat := map[string]bool{}
	for _, id := range catalog {
		if validID(id) {
			cat[id] = true
		}
	}
	c := &Config{Version: Version}
	var err error
	if want[Claude] {
		c.Claude, err = parseClaude(asObject(o["claude"]), cat, requireCatalog)
		if err != nil {
			return nil, err
		}
	}
	if want[OpenCode] {
		c.OpenCode, err = parseOpenCode(asObject(o["opencode"]), cat, requireCatalog)
		if err != nil {
			return nil, err
		}
	}
	if want[Codex] {
		c.Codex, err = parseCodex(asObject(o["codex"]), cat, requireCatalog)
		if err != nil {
			return nil, err
		}
	}
	return c, nil
}

func parseClaude(o object, cat map[string]bool, requireCatalog bool) (*ClaudeConfig, error) {
	if o == nil {
		return nil, invalid("/claude", "object")
	}
	if err := exactKeys(o, "/claude", "primary", "haiku", "sonnet", "opus", "extra"); err != nil {
		return nil, err
	}
	p, err := parseModel(asObject(o["primary"]), "/claude/primary", cat, requireCatalog)
	if err != nil {
		return nil, err
	}
	c := &ClaudeConfig{Primary: *p}
	for key, dst := range map[string]*ClaudeRole{"haiku": &c.Haiku, "sonnet": &c.Sonnet, "opus": &c.Opus} {
		*dst, err = parseRole(asObject(o[key]), "/claude/"+key, cat, requireCatalog)
		if err != nil {
			return nil, err
		}
	}
	if raw, ok := o["extra"]; ok {
		ex := asObject(raw)
		if ex == nil {
			return nil, invalid("/claude/extra", "object")
		}
		allowed := map[string]bool{"ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION": true, "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION": true, "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION": true, "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION": true}
		c.Extra = map[string]string{}
		for k, v := range ex {
			s, ok := v.(string)
			if !ok || !allowed[k] || hasControl(k) {
				return nil, invalid(pointer("/claude/extra", k), "allowlist")
			}
			c.Extra[k] = s
		}
	}
	return c, nil
}

func parseModel(o object, path string, cat map[string]bool, requireCatalog bool) (*Model, error) {
	if o == nil {
		return nil, invalid(path, "object")
	}
	if err := exactKeys(o, path, "model", "name", "context"); err != nil {
		return nil, err
	}
	id, ok := o["model"].(string)
	if !ok || !validID(id) || strings.HasSuffix(id, "[1m]") {
		return nil, invalid(path+"/model", "model_id")
	}
	if requireCatalog && !cat[id] {
		return nil, invalid(path+"/model", "catalog")
	}
	m := &Model{Model: id}
	if x, ok := o["name"]; ok {
		s, good := x.(string)
		if !good || !validName(s) {
			return nil, invalid(path+"/name", "name")
		}
		m.Name = &s
	}
	if x, ok := o["context"]; ok {
		s, good := x.(string)
		if !good || s != string(ClaudeContext1M) {
			return nil, invalid(path+"/context", "enum")
		}
		context := ClaudeContext(s)
		m.Context = &context
	}
	return m, nil
}

func parseRole(o object, path string, cat map[string]bool, requireCatalog bool) (ClaudeRole, error) {
	if o == nil {
		return ClaudeRole{}, invalid(path, "object")
	}
	if len(o) == 1 {
		if x, ok := o["inherit_primary"].(bool); ok && x {
			return ClaudeRole{InheritPrimary: true}, nil
		}
	}
	m, err := parseModel(o, path, cat, requireCatalog)
	if err != nil {
		return ClaudeRole{}, err
	}
	return ClaudeRole{Selection: m}, nil
}

func parseOpenCode(o object, cat map[string]bool, requireCatalog bool) (*OpenCodeConfig, error) {
	if o == nil {
		return nil, invalid("/opencode", "object")
	}
	if err := exactKeys(o, "/opencode", "default_model", "models"); err != nil {
		return nil, err
	}
	def, ok := o["default_model"].(string)
	if !ok {
		return nil, invalid("/opencode/default_model", "string")
	}
	models := asObject(o["models"])
	if len(models) == 0 {
		return nil, invalid("/opencode/models", "min_properties")
	}
	if len(models) > MaxReferencedModelsPerAgent {
		return nil, invalid("/opencode/models", "max_properties")
	}
	c := &OpenCodeConfig{DefaultModel: def, Models: map[string]OpenCodeModelConfig{}}
	for id, raw := range models {
		p := pointer("/opencode/models", id)
		if !validID(id) || requireCatalog && !cat[id] {
			return nil, invalid(p, "catalog")
		}
		m, err := parseOpenCodeModel(asObject(raw), p)
		if err != nil {
			return nil, err
		}
		c.Models[id] = *m
	}
	if _, ok := c.Models[def]; !ok {
		return nil, invalid("/opencode/default_model", "selected_model")
	}
	return c, nil
}

func parseOpenCodeModel(o object, path string) (*OpenCodeModelConfig, error) {
	if o == nil {
		return nil, invalid(path, "object")
	}
	if err := exactKeys(o, path, "name", "reasoning", "attachment", "tool_call", "temperature", "limit", "modalities", "interleaved", "options", "extra"); err != nil {
		return nil, err
	}
	m := &OpenCodeModelConfig{}
	if x, ok := o["name"]; ok {
		s, good := x.(string)
		if !good || !validName(s) {
			return nil, invalid(path+"/name", "name")
		}
		m.Name = &s
	}
	for key, dst := range map[string]**bool{"reasoning": &m.Reasoning, "attachment": &m.Attachment, "tool_call": &m.ToolCall, "temperature": &m.Temperature} {
		if x, ok := o[key]; ok {
			b, good := x.(bool)
			if !good {
				return nil, invalid(path+"/"+key, "boolean")
			}
			*dst = &b
		}
	}
	if x, ok := o["limit"]; ok {
		l, err := parseLimit(asObject(x), path+"/limit")
		if err != nil {
			return nil, err
		}
		m.Limit = l
	}
	if x, ok := o["modalities"]; ok {
		v, err := parseModalities(asObject(x), path+"/modalities")
		if err != nil {
			return nil, err
		}
		m.Modalities = v
	}
	if x, ok := o["interleaved"]; ok {
		switch v := x.(type) {
		case bool:
			if !v {
				return nil, invalid(path+"/interleaved", "enum")
			}
			m.Interleaved = true
		case object:
			if err := exactKeys(v, path+"/interleaved", "field"); err != nil {
				return nil, err
			}
			f, good := v["field"].(string)
			if !good || !oneOf(f, "reasoning", "reasoning_content", "reasoning_details") {
				return nil, invalid(path+"/interleaved/field", "enum")
			}
			m.Interleaved = InterleavedField{Field: f}
		default:
			return nil, invalid(path+"/interleaved", "shape")
		}
	}
	if x, ok := o["options"]; ok {
		v := asObject(x)
		if v == nil {
			return nil, invalid(path+"/options", "object")
		}
		if err := validateTree(v, path+"/options", 0, nil); err != nil {
			return nil, err
		}
		m.Options = mapFromObject(v)
	}
	if x, ok := o["extra"]; ok {
		v := asObject(x)
		if v == nil {
			return nil, invalid(path+"/extra", "object")
		}
		allowed := map[string]bool{"family": true, "release_date": true, "cost": true, "status": true, "experimental": true, "variants": true}
		if err := validateTree(v, path+"/extra", 0, allowed); err != nil {
			return nil, err
		}
		m.Extra = mapFromObject(v)
	}
	return m, nil
}

func parseLimit(o object, path string) (*Limit, error) {
	if o == nil {
		return nil, invalid(path, "object")
	}
	if err := exactKeys(o, path, "context", "input", "output"); err != nil {
		return nil, err
	}
	c, ok := positive(o["context"])
	if !ok {
		return nil, invalid(path+"/context", "positive_integer")
	}
	out, ok := positive(o["output"])
	if !ok {
		return nil, invalid(path+"/output", "positive_integer")
	}
	l := &Limit{Context: c, Output: out}
	if x, ok := o["input"]; ok {
		i, good := positive(x)
		if !good {
			return nil, invalid(path+"/input", "positive_integer")
		}
		if i > c {
			return nil, invalid(path+"/input", "maximum_context")
		}
		l.Input = &i
	}
	return l, nil
}

func parseModalities(o object, path string) (*Modalities, error) {
	if o == nil {
		return nil, invalid(path, "object")
	}
	if err := exactKeys(o, path, "input", "output"); err != nil {
		return nil, err
	}
	m := &Modalities{}
	for key, dst := range map[string]*[]string{"input": &m.Input, "output": &m.Output} {
		if x, ok := o[key]; ok {
			a, good := x.([]any)
			if !good {
				return nil, invalid(path+"/"+key, "array")
			}
			seen := map[string]bool{}
			for i, v := range a {
				s, good := v.(string)
				if !good || !oneOf(s, "text", "audio", "image", "video", "pdf") {
					return nil, invalid(path+"/"+key+"/"+strconv.Itoa(i), "enum")
				}
				if seen[s] {
					return nil, invalid(path+"/"+key, "unique")
				}
				seen[s] = true
				*dst = append(*dst, s)
			}
		}
	}
	return m, nil
}

func parseCodex(o object, cat map[string]bool, requireCatalog bool) (*CodexConfig, error) {
	if o == nil {
		return nil, invalid("/codex", "object")
	}
	if err := exactKeys(o, "/codex", "model", "reasoning_effort", "reasoning_summary", "verbosity", "context_window", "auto_compact_token_limit", "extra"); err != nil {
		return nil, err
	}
	id, ok := o["model"].(string)
	if !ok || !validID(id) || requireCatalog && !cat[id] {
		return nil, invalid("/codex/model", "catalog")
	}
	c := &CodexConfig{Model: id}
	if x, ok := o["reasoning_effort"]; ok {
		s, good := x.(string)
		if !good || len(s) > 64 || s == "" || strings.ToLower(s) != s || !isToken(s) {
			return nil, invalid("/codex/reasoning_effort", "lowercase_token")
		}
		c.ReasoningEffort = &s
	}
	if x, ok := o["reasoning_summary"]; ok {
		s, good := x.(string)
		if !good || !oneOf(s, "auto", "concise", "detailed", "none") {
			return nil, invalid("/codex/reasoning_summary", "enum")
		}
		c.ReasoningSummary = &s
	}
	if x, ok := o["verbosity"]; ok {
		s, good := x.(string)
		if !good || !oneOf(s, "low", "medium", "high") {
			return nil, invalid("/codex/verbosity", "enum")
		}
		c.Verbosity = &s
	}
	if x, ok := o["context_window"]; ok {
		n, good := positive(x)
		if !good {
			return nil, invalid("/codex/context_window", "positive_integer")
		}
		c.ContextWindow = &n
	}
	if x, ok := o["auto_compact_token_limit"]; ok {
		n, good := positive(x)
		if !good {
			return nil, invalid("/codex/auto_compact_token_limit", "positive_integer")
		}
		if c.ContextWindow != nil && n > *c.ContextWindow {
			return nil, invalid("/codex/auto_compact_token_limit", "maximum_context")
		}
		c.AutoCompactTokenLimit = &n
	}
	if x, ok := o["extra"]; ok {
		v := asObject(x)
		if v == nil {
			return nil, invalid("/codex/extra", "object")
		}
		if err := exactKeys(v, "/codex/extra", "model_auto_compact_token_limit_scope"); err != nil {
			return nil, err
		}
		if raw, exists := v["model_auto_compact_token_limit_scope"]; exists {
			s, good := raw.(string)
			if !good || !oneOf(s, "total", "body_after_prefix") {
				return nil, invalid("/codex/extra/model_auto_compact_token_limit_scope", "enum")
			}
		}
		c.Extra = mapFromObject(v)
	}
	return c, nil
}

func validateTree(v any, path string, depth int, topAllow map[string]bool) error {
	switch x := v.(type) {
	case nil:
		return invalid(path, "null")
	case string:
		if len(x) > 16<<10 {
			return invalid(path, "string_size")
		}
	case json.Number:
		f, e := x.Float64()
		if e != nil || math.IsInf(f, 0) || math.IsNaN(f) {
			return invalid(path, "number")
		}
	case bool:
	case []any:
		if depth >= 16 {
			return invalid(path, "depth")
		}
		if len(x) > 1024 {
			return invalid(path, "array_size")
		}
		for i, v := range x {
			if err := validateTree(v, path+"/"+strconv.Itoa(i), depth+1, nil); err != nil {
				return err
			}
		}
	case object:
		if depth >= 16 {
			return invalid(path, "depth")
		}
		for k, v := range x {
			kp := pointer(path, k)
			if len(k) > 128 || hasControl(k) {
				return invalid(kp, "key")
			}
			if topAllow != nil && !topAllow[k] {
				return invalid(kp, "allowlist")
			}
			if protectedKey(k) {
				return invalid(kp, "protected_path")
			}
			if err := validateTree(v, kp, depth+1, nil); err != nil {
				return err
			}
		}
	default:
		return invalid(path, "json_value")
	}
	return nil
}

func exactKeys(o object, path string, keys ...string) error {
	allowed := map[string]bool{}
	for _, k := range keys {
		allowed[k] = true
	}
	for k := range o {
		if !allowed[k] {
			return invalid(pointer(path, k), "unknown_field")
		}
	}
	return nil
}
func integer(v any) (int64, bool) {
	n, ok := v.(json.Number)
	if !ok {
		return 0, false
	}
	i, e := strconv.ParseInt(string(n), 10, 64)
	return i, e == nil && i >= 0 && i <= MaxSafeInteger
}
func positive(v any) (int64, bool) { i, ok := integer(v); return i, ok && i > 0 }
func asObject(v any) object        { o, _ := v.(object); return o }
func mapFromObject(o object) map[string]any {
	result := make(map[string]any, len(o))
	for key, value := range o {
		result[key] = plainValue(value)
	}
	return result
}
func plainValue(v any) any {
	switch x := v.(type) {
	case object:
		return mapFromObject(x)
	case []any:
		result := make([]any, len(x))
		for i, value := range x {
			result[i] = plainValue(value)
		}
		return result
	default:
		return x
	}
}
func validID(s string) bool {
	return s != "" && len(s) <= 256 && !hasControl(s) && strings.TrimFunc(s, unicode.IsSpace) == s
}
func validName(s string) bool { return s != "" && !hasControl(s) }
func hasControl(s string) bool {
	for _, r := range s {
		if unicode.IsControl(r) {
			return true
		}
	}
	return false
}
func protectedKey(s string) bool {
	n := strings.NewReplacer("_", "", "-", "", ".", "").Replace(strings.ToLower(s))
	for _, p := range protectedFragments {
		if strings.Contains(n, p) {
			return true
		}
	}
	return false
}
func oneOf(s string, v ...string) bool {
	for _, x := range v {
		if s == x {
			return true
		}
	}
	return false
}
func isToken(s string) bool {
	if !utf8.ValidString(s) {
		return false
	}
	for _, r := range s {
		if !(r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '_' || r == '-') {
			return false
		}
	}
	return true
}
