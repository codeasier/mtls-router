package modelconfig

import (
	"crypto/hmac"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"sort"
	"strings"
)

const (
	TokenVersion          = 1
	CanonicalizationJCS1  = "jcs-rfc8785-v1"
	MaxCatalogTokenSize   = 512 << 10
	MaxRevisionTokenSize  = 128 << 10
	maxTokenPayloadSize   = 384 << 10
	maxTokenStringField   = 4096
	maxCatalogTokenModels = 1000
)

var (
	ErrTokenInvalid = errors.New("invalid authenticated token")
	ErrTokenLimit   = errors.New("authenticated token exceeds limits")
)

// CatalogClaims are the complete consistency boundary for one model catalog.
type CatalogClaims struct {
	Version          int      `json:"version"`
	Models           []string `json:"models"`
	Agents           []Agent  `json:"agents"`
	Owner            string   `json:"owner"`
	RouterBaseURL    string   `json:"router_base_url"`
	DeploymentID     string   `json:"deployment_id"`
	ProtocolVersion  string   `json:"protocol_version"`
	Simplify         bool     `json:"simplify,omitempty"`
	Canonicalization string   `json:"canonicalization"`
	KeyGeneration    string   `json:"key_generation"`
}

// RevisionClaims bind an approved canonical configuration and complete file
// plan to all state that must remain unchanged before a write.
type RevisionClaims struct {
	Version                   int             `json:"version"`
	AgentPlans                []RevisionAgent `json:"agent_plans"`
	CanonicalConfig           json.RawMessage `json:"canonical_config"`
	CatalogIdentity           string          `json:"catalog_identity"`
	SidecarRevision           RevisionState   `json:"sidecar_revision"`
	RouterBaseURL             string          `json:"router_base_url"`
	DeploymentID              string          `json:"deployment_id"`
	ProtocolVersion           string          `json:"protocol_version"`
	Canonicalization          string          `json:"canonicalization"`
	KeyGeneration             string          `json:"key_generation"`
	ManagedDrift              bool            `json:"managed_drift"`
	RequiresCodexAuthApproval bool            `json:"requires_codex_auth_approval"`
	DriftedAgents             []Agent         `json:"drifted_agents"`
	Files                     []RevisionFile  `json:"files"`
}

type RevisionAgent struct {
	Agent Agent  `json:"agent"`
	Mode  string `json:"mode"`
}

type RevisionState struct {
	Exists bool   `json:"exists"`
	Size   int64  `json:"size,omitempty"`
	Mode   uint32 `json:"mode,omitempty"`
	Digest string `json:"digest,omitempty"`
}

type RevisionFile struct {
	Agent           Agent         `json:"agent"`
	Role            string        `json:"role"`
	Format          string        `json:"format"`
	SourcePath      string        `json:"source_path"`
	TargetPath      string        `json:"target_path"`
	Operation       string        `json:"operation"`
	BackupRequired  bool          `json:"backup_required"`
	BackupSource    string        `json:"backup_source,omitempty"`
	SourceRevision  RevisionState `json:"source_revision"`
	TargetRevision  RevisionState `json:"target_revision"`
	CompanionExists bool          `json:"companion_exists,omitempty"`
}

// TokenSigner authenticates opaque cross-process tokens with a private
// 256-bit key. Generation is included in and checked on every token.
type TokenSigner struct {
	key        [32]byte
	generation string
}

func NewTokenSigner(key []byte, generation string) (*TokenSigner, error) {
	if len(key) != 32 || !validTokenField(generation) {
		return nil, ErrTokenInvalid
	}
	s := &TokenSigner{generation: generation}
	copy(s.key[:], key)
	return s, nil
}

func (s *TokenSigner) SignCatalog(claims CatalogClaims) (string, error) {
	if !validAgents(claims.Agents) {
		return "", ErrTokenInvalid
	}
	claims = normalizeCatalogClaims(claims)
	if claims.Version == 0 {
		claims.Version = TokenVersion
	}
	if claims.Canonicalization == "" {
		claims.Canonicalization = CanonicalizationJCS1
	}
	claims.KeyGeneration = s.generation
	if err := validateCatalogClaims(claims); err != nil {
		return "", err
	}
	return s.sign("catalog-token-v1", claims, MaxCatalogTokenSize)
}

func (s *TokenSigner) VerifyCatalog(token string) (CatalogClaims, error) {
	var claims CatalogClaims
	if err := s.verify("catalog-token-v1", token, MaxCatalogTokenSize, &claims); err != nil {
		return CatalogClaims{}, err
	}
	if err := validateCatalogClaims(claims); err != nil || claims.KeyGeneration != s.generation {
		return CatalogClaims{}, ErrTokenInvalid
	}
	normalized := normalizeCatalogClaims(claims)
	if !equalCatalogClaims(claims, normalized) {
		return CatalogClaims{}, ErrTokenInvalid
	}
	return claims, nil
}

func (s *TokenSigner) SignRevision(claims RevisionClaims) (string, error) {
	if !validRevisionAgents(claims.AgentPlans) || !validAgents(claims.DriftedAgents) {
		return "", ErrTokenInvalid
	}
	claims = normalizeRevisionClaims(claims)
	if claims.Version == 0 {
		claims.Version = TokenVersion
	}
	if claims.Canonicalization == "" {
		claims.Canonicalization = CanonicalizationJCS1
	}
	claims.KeyGeneration = s.generation
	if err := validateRevisionClaims(claims); err != nil {
		return "", err
	}
	return s.sign("revision-token-v1", claims, MaxRevisionTokenSize)
}

func (s *TokenSigner) VerifyRevision(token string) (RevisionClaims, error) {
	var claims RevisionClaims
	if err := s.verify("revision-token-v1", token, MaxRevisionTokenSize, &claims); err != nil {
		return RevisionClaims{}, err
	}
	if err := validateRevisionClaims(claims); err != nil || claims.KeyGeneration != s.generation {
		return RevisionClaims{}, ErrTokenInvalid
	}
	normalized := normalizeRevisionClaims(claims)
	want, _ := marshalJCS(claims)
	got, _ := marshalJCS(normalized)
	if subtle.ConstantTimeCompare(want, got) != 1 {
		return RevisionClaims{}, ErrTokenInvalid
	}
	return claims, nil
}

// RevisionMAC returns a context-separated keyed whole-file revision. Context
// and identity must be stable, non-secret identifiers such as "agent-file"
// and an absolute path.
func (s *TokenSigner) RevisionMAC(context, identity string, content []byte) (string, error) {
	if !validTokenField(context) || !validTokenField(identity) {
		return "", ErrTokenInvalid
	}
	key := s.derivedKey("file-revision-v1\x00" + context)
	mac := hmac.New(sha256.New, key[:])
	mac.Write([]byte(identity))
	mac.Write([]byte{0})
	mac.Write(content)
	return base64.RawURLEncoding.EncodeToString(mac.Sum(nil)), nil
}

func (s *TokenSigner) sign(context string, claims any, limit int) (string, error) {
	payload, err := marshalJCS(claims)
	if err != nil {
		return "", ErrTokenInvalid
	}
	if len(payload) > maxTokenPayloadSize {
		return "", ErrTokenLimit
	}
	key := s.derivedKey(context)
	mac := hmac.New(sha256.New, key[:])
	mac.Write(payload)
	token := base64.RawURLEncoding.EncodeToString(payload) + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if len(token) > limit {
		return "", ErrTokenLimit
	}
	return token, nil
}

func (s *TokenSigner) verify(context, token string, limit int, dst any) error {
	if token == "" || len(token) > limit || strings.Count(token, ".") != 1 {
		return ErrTokenInvalid
	}
	parts := strings.SplitN(token, ".", 2)
	payload, err := base64.RawURLEncoding.Strict().DecodeString(parts[0])
	if err != nil || len(payload) == 0 || len(payload) > maxTokenPayloadSize {
		return ErrTokenInvalid
	}
	signature, err := base64.RawURLEncoding.Strict().DecodeString(parts[1])
	if err != nil || len(signature) != sha256.Size {
		return ErrTokenInvalid
	}
	key := s.derivedKey(context)
	mac := hmac.New(sha256.New, key[:])
	mac.Write(payload)
	if subtle.ConstantTimeCompare(signature, mac.Sum(nil)) != 1 {
		return ErrTokenInvalid
	}
	decoder := json.NewDecoder(strings.NewReader(string(payload)))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(dst); err != nil {
		return ErrTokenInvalid
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return ErrTokenInvalid
	}
	canonical, err := marshalJCS(dst)
	if err != nil || subtle.ConstantTimeCompare(payload, canonical) != 1 {
		return ErrTokenInvalid
	}
	return nil
}

func (s *TokenSigner) derivedKey(context string) [32]byte {
	mac := hmac.New(sha256.New, s.key[:])
	mac.Write([]byte("mtls-router-agent-modelconfig\x00" + context))
	var result [32]byte
	copy(result[:], mac.Sum(nil))
	return result
}

func normalizeCatalogClaims(c CatalogClaims) CatalogClaims {
	c.Models = normalizeStrings(c.Models)
	c.Agents = normalizeAgents(c.Agents)
	return c
}

func normalizeRevisionClaims(c RevisionClaims) RevisionClaims {
	sort.Slice(c.AgentPlans, func(i, j int) bool { return agentRank(c.AgentPlans[i].Agent) < agentRank(c.AgentPlans[j].Agent) })
	c.DriftedAgents = normalizeAgents(c.DriftedAgents)
	sort.Slice(c.Files, func(i, j int) bool {
		if c.Files[i].Agent != c.Files[j].Agent {
			return agentRank(c.Files[i].Agent) < agentRank(c.Files[j].Agent)
		}
		if c.Files[i].Role != c.Files[j].Role {
			return c.Files[i].Role < c.Files[j].Role
		}
		return c.Files[i].TargetPath < c.Files[j].TargetPath
	})
	return c
}

func agentRank(value Agent) int {
	for i, agent := range []Agent{Claude, OpenCode, Codex} {
		if value == agent {
			return i
		}
	}
	return 3
}

func normalizeStrings(values []string) []string {
	result := append([]string(nil), values...)
	sort.Strings(result)
	if len(result) == 0 {
		return result
	}
	out := result[:1]
	for _, value := range result[1:] {
		if value != out[len(out)-1] {
			out = append(out, value)
		}
	}
	return out
}

func normalizeAgents(values []Agent) []Agent {
	seen := make(map[Agent]bool, len(values))
	for _, value := range values {
		seen[value] = true
	}
	result := make([]Agent, 0, len(seen))
	for _, value := range []Agent{Claude, OpenCode, Codex} {
		if seen[value] {
			result = append(result, value)
		}
	}
	return result
}

func validAgents(values []Agent) bool {
	for _, value := range values {
		if value != Claude && value != OpenCode && value != Codex {
			return false
		}
	}
	return true
}

func validRevisionAgents(values []RevisionAgent) bool {
	for _, value := range values {
		if !validAgents([]Agent{value.Agent}) {
			return false
		}
	}
	return true
}

func validateCatalogClaims(c CatalogClaims) error {
	if c.Version != TokenVersion || c.Canonicalization != CanonicalizationJCS1 || len(c.Models) == 0 || len(c.Models) > maxCatalogTokenModels || len(c.Agents) == 0 || (c.Owner != "cli" && c.Owner != "desktop") {
		return ErrTokenInvalid
	}
	for _, value := range append(append([]string{}, c.Models...), c.Owner, c.RouterBaseURL, c.DeploymentID, c.ProtocolVersion, c.Canonicalization, c.KeyGeneration) {
		if !validTokenField(value) {
			return ErrTokenInvalid
		}
	}
	return nil
}

func validateRevisionClaims(c RevisionClaims) error {
	if c.Version != TokenVersion || c.Canonicalization != CanonicalizationJCS1 || len(c.AgentPlans) == 0 || len(c.AgentPlans) > 3 || len(c.CanonicalConfig) == 0 || len(c.Files) > 16 {
		return ErrTokenInvalid
	}
	for _, value := range []string{c.CatalogIdentity, c.RouterBaseURL, c.DeploymentID, c.ProtocolVersion, c.Canonicalization, c.KeyGeneration} {
		if !validTokenField(value) {
			return ErrTokenInvalid
		}
	}
	if !validRevisionState(c.SidecarRevision) {
		return ErrTokenInvalid
	}
	planned := make(map[Agent]bool, len(c.AgentPlans))
	filesByAgent := make(map[Agent]map[string]RevisionFile, len(c.AgentPlans))
	for _, plan := range c.AgentPlans {
		if planned[plan.Agent] || (plan.Mode != "merge" && plan.Mode != "rebuild") {
			return ErrTokenInvalid
		}
		planned[plan.Agent] = true
		filesByAgent[plan.Agent] = map[string]RevisionFile{}
	}
	seenFiles := map[string]bool{}
	for _, file := range c.Files {
		identity := string(file.Agent) + "\x00" + file.Role + "\x00" + file.TargetPath
		if !planned[file.Agent] || seenFiles[identity] || (file.Role != "config" && file.Role != "auth") ||
			(file.Format != "json" && file.Format != "jsonc" && file.Format != "toml") ||
			(file.Operation != "create" && file.Operation != "replace") ||
			!validTokenField(file.SourcePath) || !validTokenField(file.TargetPath) ||
			file.BackupRequired != file.SourceRevision.Exists || file.BackupRequired != (file.BackupSource != "") ||
			(file.BackupSource != "" && !validTokenField(file.BackupSource)) ||
			!validRevisionState(file.SourceRevision) || !validRevisionState(file.TargetRevision) ||
			(file.Operation == "create") != !file.TargetRevision.Exists ||
			(file.Operation == "replace") != file.TargetRevision.Exists ||
			(file.BackupRequired && file.BackupSource != file.SourcePath) ||
			(file.SourcePath == file.TargetPath && file.SourceRevision != file.TargetRevision) ||
			(file.SourcePath != file.TargetPath && file.TargetRevision.Exists) {
			return ErrTokenInvalid
		}
		seenFiles[identity] = true
		if _, exists := filesByAgent[file.Agent][file.Role]; exists {
			return ErrTokenInvalid
		}
		filesByAgent[file.Agent][file.Role] = file
	}
	for agent, files := range filesByAgent {
		config, configOK := files["config"]
		switch agent {
		case Claude:
			if !configOK || len(files) != 1 || config.Format != "json" || config.CompanionExists {
				return ErrTokenInvalid
			}
		case OpenCode:
			if !configOK || len(files) != 1 || (config.Format != "json" && config.Format != "jsonc") || config.CompanionExists {
				return ErrTokenInvalid
			}
		case Codex:
			auth, authOK := files["auth"]
			if !configOK || !authOK || len(files) != 2 || config.Format != "toml" || auth.Format != "json" ||
				config.CompanionExists != auth.SourceRevision.Exists || auth.CompanionExists != config.SourceRevision.Exists {
				return ErrTokenInvalid
			}
		}
	}
	var value any
	if err := json.Unmarshal(c.CanonicalConfig, &value); err != nil {
		return ErrTokenInvalid
	}
	canonical, err := marshalJCS(value)
	if err != nil || subtle.ConstantTimeCompare(c.CanonicalConfig, canonical) != 1 {
		return ErrTokenInvalid
	}
	return nil
}

func validRevisionState(value RevisionState) bool {
	if !value.Exists {
		return value.Size == 0 && value.Mode == 0 && value.Digest == ""
	}
	return value.Size >= 0 && value.Size <= 1<<30 && validTokenField(value.Digest)
}

func validTokenField(value string) bool {
	return value != "" && len(value) <= maxTokenStringField && strings.TrimSpace(value) == value
}

func equalCatalogClaims(a, b CatalogClaims) bool {
	x, _ := marshalJCS(a)
	y, _ := marshalJCS(b)
	return subtle.ConstantTimeCompare(x, y) == 1
}
