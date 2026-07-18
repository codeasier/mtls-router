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
	Canonicalization string   `json:"canonicalization"`
	KeyGeneration    string   `json:"key_generation"`
}

// RevisionClaims bind an approved canonical configuration to all state that
// must remain unchanged before a write. Bindings are caller-defined keyed
// revision MACs, allowing sidecar and file snapshots to share this contract.
type RevisionClaims struct {
	Version                   int               `json:"version"`
	Agents                    []Agent           `json:"agents"`
	CanonicalConfig           json.RawMessage   `json:"canonical_config"`
	CatalogIdentity           string            `json:"catalog_identity"`
	SidecarRevision           string            `json:"sidecar_revision"`
	RouterBaseURL             string            `json:"router_base_url"`
	DeploymentID              string            `json:"deployment_id"`
	ProtocolVersion           string            `json:"protocol_version"`
	Canonicalization          string            `json:"canonicalization"`
	KeyGeneration             string            `json:"key_generation"`
	ManagedDrift              bool              `json:"managed_drift"`
	RequiresCodexAuthApproval bool              `json:"requires_codex_auth_approval"`
	DriftedAgents             []Agent           `json:"drifted_agents"`
	Bindings                  []RevisionBinding `json:"bindings"`
}

type RevisionBinding struct {
	Context  string `json:"context"`
	Identity string `json:"identity"`
	Revision string `json:"revision"`
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
	if !validAgents(claims.Agents) || !validAgents(claims.DriftedAgents) {
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
	c.Agents = normalizeAgents(c.Agents)
	c.DriftedAgents = normalizeAgents(c.DriftedAgents)
	sort.Slice(c.Bindings, func(i, j int) bool {
		if c.Bindings[i].Context != c.Bindings[j].Context {
			return c.Bindings[i].Context < c.Bindings[j].Context
		}
		return c.Bindings[i].Identity < c.Bindings[j].Identity
	})
	return c
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
	if c.Version != TokenVersion || c.Canonicalization != CanonicalizationJCS1 || len(c.Agents) == 0 || len(c.CanonicalConfig) == 0 || len(c.Bindings) > 32 {
		return ErrTokenInvalid
	}
	for _, value := range []string{c.CatalogIdentity, c.SidecarRevision, c.RouterBaseURL, c.DeploymentID, c.ProtocolVersion, c.Canonicalization, c.KeyGeneration} {
		if !validTokenField(value) {
			return ErrTokenInvalid
		}
	}
	for _, binding := range c.Bindings {
		if !validTokenField(binding.Context) || !validTokenField(binding.Identity) || !validTokenField(binding.Revision) {
			return ErrTokenInvalid
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

func validTokenField(value string) bool {
	return value != "" && len(value) <= maxTokenStringField && strings.TrimSpace(value) == value
}

func equalCatalogClaims(a, b CatalogClaims) bool {
	x, _ := marshalJCS(a)
	y, _ := marshalJCS(b)
	return subtle.ConstantTimeCompare(x, y) == 1
}
