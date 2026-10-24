//go:build unix

package routerenroll

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type providerTransport func(*http.Request) (*http.Response, error)

func (f providerTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func testConfig(t *testing.T) Config {
	t.Helper()
	dir, key := testStoreFiles(t)
	secret := filepath.Join(filepath.Dir(key), "oauth-secret")
	if err := os.WriteFile(secret, []byte("provider-secret-canary"), 0600); err != nil {
		t.Fatal(err)
	}
	return Config{Listen: DefaultListen, StateDir: dir, TokenKeyFile: key, JournalDir: testJournalDirectory(dir), ClientID: "fixture-app", ClientSecretFile: secret, OrgPath: "msinsight-org"}
}

func TestOrganizationConfigurationAndPolicyBinding(t *testing.T) {
	emptyEnv := func(string) (string, bool) { return "", false }
	if LoadConfig(emptyEnv).OrgPath != "msinsight-org" {
		t.Fatal("missing default")
	}
	cfg := testConfig(t)
	for _, path := range []string{"", "../other", "https://gitcode.com/msinsight-org", "org/other", "org?x=1"} {
		bad := cfg
		bad.OrgPath = path
		if bad.Validate() == nil {
			t.Fatal("unsafe org", path)
		}
	}
	if LoadConfig(func(name string) (string, bool) { return "", name == "GITCODE_ORG_PATH" }).OrgPath != "" {
		t.Fatal("explicit empty silently defaulted")
	}
	s, err := OpenStore(cfg.StateDir, cfg.TokenKeyFile, cfg.JournalDir)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	ctx := context.Background()
	p, err := s.ActivatePolicy(ctx, cfg)
	if err != nil || p.Epoch != 1 {
		t.Fatal(err)
	}
	if err := s.transaction(ctx, func(tx *writeTx) error {
		return bindOrganization(ctx, tx, p.Epoch, Organization{"1783195", cfg.OrgPath})
	}); err != nil {
		t.Fatal(err)
	}
	if err := s.transaction(ctx, func(tx *writeTx) error {
		return bindOrganization(ctx, tx, p.Epoch, Organization{"different-id", cfg.OrgPath})
	}); err == nil {
		t.Fatal("organization rebound")
	}
	restored, err := s.ActivatePolicy(ctx, cfg)
	if err != nil || restored.BoundID != "1783195" || restored.Epoch != p.Epoch {
		t.Fatal("idempotent policy lost binding")
	}
	cfg.OrgPath = "replacement-org"
	next, err := s.ActivatePolicy(ctx, cfg)
	if err != nil || next.Epoch != 2 || next.BoundID != "" {
		t.Fatal("policy epoch not advanced")
	}
	if err := s.transaction(ctx, func(tx *writeTx) error { return bindOrganization(ctx, tx, p.Epoch, Organization{"1783195", p.Path}) }); err == nil {
		t.Fatal("stale network response rebound old policy")
	}
}

func TestMembershipDecisionsAreExplicitAndUserBound(t *testing.T) {
	const user = "650d67fbae6d795139b49b41"
	base := `{"active":true,"user":{"id":"` + user + `"},"organization":{"id":1783195,"login":"msinsight-org"}}`
	p := Policy{Path: "msinsight-org", BoundID: "1783195"}
	for _, test := range []struct {
		name, body   string
		status       int
		code, reason string
	}{
		{"member", base, 200, "", "active"},
		{"integer-active", strings.Replace(base, `true`, `1`, 1), 200, "", "active"},
		{"nonmember", strings.Replace(base, `true`, `false`, 1), 200, MembershipRequired, "inactive"},
		{"inactive-zero", strings.Replace(base, `true`, `0`, 1), 200, MembershipRequired, "inactive"},
		{"invitation", strings.Replace(base, `true`, `"pending"`, 1), 200, ServiceUnavailable, "incomplete_membership"},
		{"wrong-user", strings.Replace(base, user, "other-user", 1), 200, ServiceUnavailable, "membership_user_mismatch"},
		{"wrong-org", strings.Replace(base, "1783195", "1783196", 1), 200, ServiceUnavailable, "organization_id_mismatch"},
		{"renamed", strings.Replace(base, "msinsight-org", "renamed-org", 1), 200, ServiceUnavailable, "organization_path_drift"},
		{"unknown-404", `{}`, 404, ServiceUnavailable, "membership_lookup_unavailable"},
		{"scope", `{"error":"insufficient_scope"}`, 403, ServiceUnavailable, "insufficient_scope"},
		{"outage", `upstream-diagnostic-canary`, 503, ServiceUnavailable, "membership_lookup_unavailable"},
	} {
		t.Run(test.name, func(t *testing.T) {
			got := evaluateMembership(test.status, []byte(test.body), user, p)
			if got.Code != test.code || got.Reason != test.reason {
				t.Fatalf("%+v", got)
			}
		})
	}
	for _, raw := range []string{`"650d67fbae6d795139b49b41"`, `1783195`, `"opaque-user-id"`} {
		if providerID([]byte(raw)) == "" {
			t.Fatal("valid opaque ID rejected", raw)
		}
	}
	for _, raw := range []string{`null`, `-1`, `1.5`, `{}`, `""`, `"id\ncanary"`} {
		if providerID([]byte(raw)) != "" {
			t.Fatal("invalid ID accepted", raw)
		}
	}
}

func TestFixedGitCodeExchangeAndLeastPrivilege(t *testing.T) {
	g, err := NewGitCode(testConfig(t))
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := url.Parse(g.AuthorizationURL("state-canary"))
	if err != nil || parsed.Scheme+"://"+parsed.Host+parsed.Path != authorizeURL || parsed.Query().Get("scope") != gitcodeScope || parsed.Query().Get("redirect_uri") != EnrollOrigin+"/enroll/gitcode/callback" {
		t.Fatal("mutable OAuth trust boundary")
	}
	g.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		if r.Method != "POST" || r.URL.Scheme+"://"+r.URL.Host+r.URL.Path != tokenURL || r.URL.Query().Get("code") != "code-canary" || r.URL.Query().Has("client_secret") {
			t.Fatal("incorrect fixed token request")
		}
		if r.ParseMultipartForm(32*1024) != nil || r.FormValue("client_secret") != "provider-secret-canary" {
			t.Fatal("missing private form secret")
		}
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{"access_token":"access-canary","refresh_token":"refresh-canary","expires_in":300,"scope":"read_user read_groups"}`)), Request: r}, nil
	})
	token, err := g.Exchange(context.Background(), "code-canary")
	if err != nil || token.AccessToken != "access-canary" {
		t.Fatal(err)
	}
	g.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{"access_token":"access-canary","expires_in":300,"scope":"read_user"}`)), Request: r}, nil
	})
	if _, err := g.Exchange(context.Background(), "code-canary"); err == nil || err.Error() != "insufficient_scope" {
		t.Fatal("scope not enforced", err)
	}
	g.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{"access_token":"access-canary","expires_in":300,"scope":"all_user read_groups"}`)), Request: r}, nil
	})
	if _, err := g.Exchange(context.Background(), "code-canary"); err != nil {
		t.Fatal("GitCode all_user scope rejected", err)
	}
	g.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{"access_token":"access-canary","expires_in":300,"scope":"all_user read_groups read_key"}`)), Request: r}, nil
	})
	if _, err := g.Exchange(context.Background(), "code-canary"); err == nil || err.Error() != "scope_escalation" {
		t.Fatal("extra scope not enforced", err)
	}
	for _, payload := range []struct {
		status int
		body   string
		reauth bool
	}{{401, `{"error":"invalid_grant","secret":"canary"}`, true}, {403, `{"error":"insufficient_scope"}`, false}, {503, `private-provider-error-canary`, false}} {
		err := classifyProviderFailure(payload.status, []byte(payload.body))
		if needsReauthorization(err) != payload.reauth || strings.Contains(err.Error(), "canary") {
			t.Fatal("unsafe provider failure", err)
		}
	}
}

func TestGitCodeRedirectAndResponseBudgets(t *testing.T) {
	g, err := NewGitCode(testConfig(t))
	if err != nil {
		t.Fatal(err)
	}
	calls := 0
	g.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		calls++
		return &http.Response{StatusCode: 302, Header: http.Header{"Location": {"https://attacker.invalid/token"}}, Body: io.NopCloser(strings.NewReader("redirect-canary")), Request: r}, nil
	})
	if _, err := g.User(context.Background(), "access-canary"); err == nil || calls != 1 {
		t.Fatal("redirect followed", err, calls)
	}
	g.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(strings.Repeat("x", providerBodyLimit+1))), Request: r}, nil
	})
	if _, err := g.User(context.Background(), "access-canary"); err == nil || err.Error() != "gitcode_response_invalid" {
		t.Fatal("unbounded response", err)
	}
	g.client.Transport = providerTransport(func(*http.Request) (*http.Response, error) { return nil, errors.New("oauth-code-token-secret-canary") })
	if _, err := g.User(context.Background(), "access-canary"); err == nil || strings.Contains(err.Error(), "canary") {
		t.Fatal("transport diagnostic leaked", err)
	}
}

func TestGitCodeTokenVersionsEncrypted(t *testing.T) {
	cfg := testConfig(t)
	s, err := OpenStore(cfg.StateDir, cfg.TokenKeyFile, cfg.JournalDir)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	ctx := context.Background()
	account := Account{ID: "opaque-user-id", Username: "fixture"}
	for i := 1; i <= 2; i++ {
		err := s.transaction(ctx, func(tx *writeTx) error {
			return s.saveGitCodeToken(ctx, tx, 1, account, GitCodeToken{AccessToken: "access-canary", RefreshToken: "refresh-canary"})
		})
		if err != nil {
			t.Fatal(err)
		}
		var ciphertext []byte
		var version int64
		if err := s.db.QueryRow("SELECT version,ciphertext FROM gitcode_tokens").Scan(&version, &ciphertext); err != nil {
			t.Fatal(err)
		}
		var token GitCodeToken
		if version != int64(i) || s.decrypt(ciphertext, tokenAAD(1, account.ID, version), &token) != nil || token.AccessToken != "access-canary" {
			t.Fatal("version/encryption mismatch")
		}
	}
}
