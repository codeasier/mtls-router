package routerenroll

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io"
	"mime/multipart"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const (
	authorizeURL      = "https://gitcode.com/oauth/authorize"
	tokenURL          = "https://gitcode.com/oauth/token"
	userURL           = "https://api.gitcode.com/api/v5/user"
	membershipURL     = "https://api.gitcode.com/api/v5/user/memberships/orgs/"
	gitcodeScope      = "all_user read_groups"
	providerBodyLimit = 64 * 1024
)

type GitCodeToken struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresAt    int64  `json:"expires_at"`
	Scope        string `json:"scope"`
}
type Organization struct {
	ID   string `json:"id"`
	Path string `json:"path"`
}
type Account struct {
	ID           string       `json:"id"`
	Username     string       `json:"username"`
	Organization Organization `json:"organization"`
}
type providerFailure struct {
	reason string
	reauth bool
}

func (e *providerFailure) Error() string { return e.reason }
func providerError(reason string) error  { return &providerFailure{reason: reason} }

type GitCode struct {
	client           *http.Client
	clientID, secret string
	now              func() time.Time
}

func NewGitCode(cfg Config) (*GitCode, error) {
	if cfg.Validate() != nil {
		return nil, ErrConfig
	}
	secret, err := readPrivateFile(cfg.ClientSecretFile, 16*1024)
	if err != nil {
		return nil, ErrConfig
	}
	defer clear(secret)
	value := strings.TrimSpace(string(secret))
	if !boundedProviderToken(value) {
		return nil, ErrConfig
	}
	return &GitCode{client: &http.Client{Timeout: 5 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, clientID: cfg.ClientID, secret: value, now: time.Now}, nil
}
func (g *GitCode) AuthorizationURL(state string) string {
	values := url.Values{"client_id": {g.clientID}, "redirect_uri": {EnrollOrigin + "/enroll/gitcode/callback"}, "response_type": {"code"}, "scope": {gitcodeScope}, "state": {state}}
	return authorizeURL + "?" + values.Encode()
}
func (g *GitCode) Exchange(ctx context.Context, code string) (GitCodeToken, error) {
	if !boundedProviderToken(code) || len(code) > 4096 {
		return GitCodeToken{}, providerError("oauth_code_invalid")
	}
	return g.token(ctx, url.Values{"grant_type": {"authorization_code"}, "code": {code}, "client_id": {g.clientID}}, "")
}
func (g *GitCode) Refresh(ctx context.Context, previous GitCodeToken) (GitCodeToken, error) {
	if !boundedProviderToken(previous.RefreshToken) {
		return GitCodeToken{}, &providerFailure{reason: "gitcode_reauthorization_required", reauth: true}
	}
	return g.token(ctx, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {previous.RefreshToken}, "client_id": {g.clientID}}, previous.RefreshToken)
}
func (g *GitCode) token(ctx context.Context, query url.Values, previousRefresh string) (GitCodeToken, error) {
	var body bytes.Buffer
	writer := multipart.NewWriter(&body)
	if writer.WriteField("client_secret", g.secret) != nil || writer.Close() != nil {
		return GitCodeToken{}, providerError("oauth_request_failed")
	}
	request, err := http.NewRequestWithContext(ctx, "POST", tokenURL+"?"+query.Encode(), &body)
	if err != nil {
		return GitCodeToken{}, providerError("oauth_request_failed")
	}
	request.Header.Set("Content-Type", writer.FormDataContentType())
	status, data, err := g.request(request)
	if err != nil {
		return GitCodeToken{}, err
	}
	if status != 200 {
		return GitCodeToken{}, classifyProviderFailure(status, data)
	}
	var payload struct {
		Access  string `json:"access_token"`
		Refresh string `json:"refresh_token"`
		Expires int64  `json:"expires_in"`
		Scope   string `json:"scope"`
	}
	if json.Unmarshal(data, &payload) != nil || !boundedProviderToken(payload.Access) || payload.Expires <= 0 || payload.Expires > 366*24*60*60 || (payload.Refresh != "" && !boundedProviderToken(payload.Refresh)) {
		return GitCodeToken{}, providerError("oauth_response_incomplete")
	}
	scopes := map[string]bool{}
	for _, scope := range strings.Fields(payload.Scope) {
		scopes[scope] = true
	}
	if (!scopes["all_user"] && !scopes["read_user"]) || !scopes["read_groups"] {
		return GitCodeToken{}, providerError("insufficient_scope")
	}
	for scope := range scopes {
		if scope != "all_user" && scope != "read_user" && scope != "read_groups" {
			return GitCodeToken{}, providerError("scope_escalation")
		}
	}
	if payload.Refresh == "" {
		payload.Refresh = previousRefresh
	}
	return GitCodeToken{payload.Access, payload.Refresh, g.now().Add(time.Duration(payload.Expires) * time.Second).Unix(), payload.Scope}, nil
}
func (g *GitCode) User(ctx context.Context, access string) (Account, error) {
	request, _ := http.NewRequestWithContext(ctx, "GET", userURL, nil)
	request.Header.Set("Authorization", "Bearer "+access)
	status, data, err := g.request(request)
	if err != nil {
		return Account{}, err
	}
	if status != 200 {
		return Account{}, classifyProviderFailure(status, data)
	}
	var payload struct {
		ID    json.RawMessage `json:"id"`
		Login string          `json:"login"`
	}
	if json.Unmarshal(data, &payload) != nil || providerID(payload.ID) == "" || len(payload.Login) == 0 || len(payload.Login) > 256 || strings.ContainsAny(payload.Login, "\r\n\x00") {
		return Account{}, providerError("user_response_incomplete")
	}
	return Account{ID: providerID(payload.ID), Username: payload.Login}, nil
}

type membershipDecision struct {
	Organization Organization
	Code, Reason string
}

func (g *GitCode) Membership(ctx context.Context, access string, account Account, policy Policy) (membershipDecision, error) {
	if !orgPattern.MatchString(policy.Path) {
		return membershipDecision{}, providerError("organization_config_invalid")
	}
	request, _ := http.NewRequestWithContext(ctx, "GET", membershipURL+url.PathEscape(policy.Path), nil)
	request.Header.Set("Authorization", "Bearer "+access)
	status, data, err := g.request(request)
	if err != nil {
		return membershipDecision{}, err
	}
	if status == 401 {
		return membershipDecision{}, classifyProviderFailure(status, data)
	}
	return evaluateMembership(status, data, account.ID, policy), nil
}
func evaluateMembership(status int, data []byte, userID string, policy Policy) membershipDecision {
	reject := func(code, reason string) membershipDecision { return membershipDecision{Code: code, Reason: reason} }
	if status != 200 {
		var payload struct {
			Error string `json:"error"`
		}
		_ = json.Unmarshal(data, &payload)
		if payload.Error == "insufficient_scope" {
			return reject(ServiceUnavailable, "insufficient_scope")
		}
		return reject(ServiceUnavailable, "membership_lookup_unavailable")
	}
	var payload struct {
		Active json.RawMessage `json:"active"`
		User   struct {
			ID json.RawMessage `json:"id"`
		} `json:"user"`
		Organization struct {
			ID    json.RawMessage `json:"id"`
			Login string          `json:"login"`
		} `json:"organization"`
	}
	if json.Unmarshal(data, &payload) != nil || userID == "" || providerID(payload.User.ID) != userID {
		return reject(ServiceUnavailable, "membership_user_mismatch")
	}
	orgID := providerID(payload.Organization.ID)
	if orgID == "" || payload.Organization.Login == "" {
		return reject(ServiceUnavailable, "incomplete_membership")
	}
	if policy.organizationID() != "" && policy.organizationID() != orgID {
		return reject(ServiceUnavailable, "organization_id_mismatch")
	}
	if payload.Organization.Login != policy.Path {
		return reject(ServiceUnavailable, "organization_path_drift")
	}
	// Published schema says integer; its successful example says boolean. Only
	// explicit true/1 or false/0 is accepted, never a truthy string or invitation.
	switch string(bytes.TrimSpace(payload.Active)) {
	case "true", "1":
		return membershipDecision{Organization: Organization{orgID, payload.Organization.Login}, Reason: "active"}
	case "false", "0":
		return reject(MembershipRequired, "inactive")
	default:
		return reject(ServiceUnavailable, "incomplete_membership")
	}
}
func providerID(raw json.RawMessage) string {
	var text string
	if json.Unmarshal(raw, &text) == nil {
		if stableIDPattern.MatchString(text) {
			return text
		}
		return ""
	}
	var number json.Number
	if json.Unmarshal(raw, &number) == nil {
		if value, err := number.Int64(); err == nil && value > 0 {
			return number.String()
		}
	}
	return ""
}
func (g *GitCode) request(request *http.Request) (int, []byte, error) {
	request.Header.Set("Accept", "application/json")
	response, err := g.client.Do(request)
	if err != nil {
		return 0, nil, providerError("gitcode_request_failed")
	}
	defer func() { _ = response.Body.Close() }()
	data, err := io.ReadAll(io.LimitReader(response.Body, providerBodyLimit+1))
	if err != nil || len(data) > providerBodyLimit {
		return 0, nil, providerError("gitcode_response_invalid")
	}
	return response.StatusCode, data, nil
}
func classifyProviderFailure(status int, data []byte) error {
	var payload struct {
		Error string `json:"error"`
	}
	_ = json.Unmarshal(data, &payload)
	if payload.Error == "insufficient_scope" {
		return providerError("insufficient_scope")
	}
	if status == 401 || payload.Error == "invalid_grant" {
		return &providerFailure{reason: "gitcode_reauthorization_required", reauth: true}
	}
	return providerError("gitcode_response_unavailable")
}
func boundedProviderToken(value string) bool {
	return len(value) > 0 && len(value) <= 8192 && !strings.ContainsAny(value, "\r\n\x00")
}
func needsReauthorization(err error) bool {
	var failure *providerFailure
	return errors.As(err, &failure) && failure.reauth
}

// saveGitCodeToken runs only after trusted user/membership validation. It does
// no network I/O while holding the transaction, and binds ciphertext to version.
func (s *Store) saveGitCodeToken(ctx context.Context, tx *writeTx, epoch int64, account Account, token GitCodeToken) error {
	var previous int64
	if err := tx.QueryRowContext(ctx, "SELECT version FROM gitcode_tokens WHERE epoch=? AND user_id=?", epoch, account.ID).Scan(&previous); err != nil && !errors.Is(err, sql.ErrNoRows) {
		return ErrStorage
	}
	version := previous + 1
	encrypted, err := s.encrypt(token, tokenAAD(epoch, account.ID, version))
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO users(epoch,user_id,username) VALUES(?,?,?) ON CONFLICT(epoch,user_id) DO UPDATE SET username=excluded.username,version=users.version+1,verification_status='unknown'`, epoch, account.ID, account.Username); err != nil {
		return ErrStorage
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO gitcode_tokens(epoch,user_id,version,ciphertext) VALUES(?,?,?,?) ON CONFLICT(epoch,user_id) DO UPDATE SET version=excluded.version,ciphertext=excluded.ciphertext`, epoch, account.ID, version, encrypted); err != nil {
		return ErrStorage
	}
	return nil
}
