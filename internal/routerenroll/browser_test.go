//go:build unix

package routerenroll

import (
	"bytes"
	"context"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

var csrfInput = regexp.MustCompile(`name="csrf" value="([A-Za-z0-9_-]+)"`)

type browserFlow struct {
	s              *Service
	handler        http.Handler
	proof          NativeProof
	started        NativeStarted
	cookie         *http.Cookie
	csrf, state    string
	exchanges      atomic.Int32
	member         atomic.Bool
	duringExchange func()
	logs           bytes.Buffer
}

func newBrowserFlow(t *testing.T) *browserFlow {
	t.Helper()
	s, request, verifier := nativeFixture(t)
	request.DeviceLabel = `device <script>alert("x")</script>`
	flow := &browserFlow{s: s}
	flow.member.Store(true)
	s.provider = &GitCode{clientID: "fixture-app", secret: "oauth-private-secret-canary", now: s.now, client: &http.Client{Timeout: time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}}
	s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
		body := ""
		switch r.URL.Scheme + "://" + r.URL.Host + r.URL.Path {
		case tokenURL:
			flow.exchanges.Add(1)
			if flow.duringExchange != nil {
				flow.duringExchange()
			}
			body = `{"access_token":"oauth-access-canary","refresh_token":"oauth-refresh-canary","expires_in":300,"scope":"read_user read_groups"}`
		case userURL:
			body = `{"id":"new-member-id","login":"member <script>bad</script>"}`
		case membershipURL + "msinsight-org":
			active := "false"
			if flow.member.Load() {
				active = "true"
			}
			body = `{"active":` + active + `,"user":{"id":"new-member-id"},"organization":{"id":"1783195","login":"msinsight-org"}}`
		default:
			t.Errorf("unexpected provider target: %s", r.URL.Path)
		}
		return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(body)), Request: r}, nil
	})
	started, code := s.start(context.Background(), request)
	if code != "" {
		t.Fatal(code)
	}
	flow.started = started
	flow.proof = NativeProof{started.Transaction, verifier}
	server, err := NewHTTPServer(DefaultListen, s.Handler(), slog.New(slog.NewJSONHandler(&flow.logs, nil)))
	if err != nil {
		t.Fatal(err)
	}
	flow.s.SetLogger(slog.New(slog.NewJSONHandler(&flow.logs, nil)))
	flow.handler = server.Handler
	w := flow.get("/enroll/native/verify?transaction="+started.Transaction, nil)
	if w.Code != 200 {
		t.Fatalf("verify failed %d", w.Code)
	}
	cookies := w.Result().Cookies()
	if len(cookies) != 1 {
		t.Fatal("binding cookie missing")
	}
	flow.cookie = cookies[0]
	if !flow.cookie.Secure || !flow.cookie.HttpOnly || flow.cookie.Path != "/" || flow.cookie.Domain != "" || flow.cookie.SameSite != http.SameSiteLaxMode {
		t.Fatal("unsafe OAuth binding cookie")
	}
	flow.csrf = extractCSRF(t, w.Body.String())
	if strings.Contains(w.Body.String(), "<script>") {
		t.Fatal("device label XSS")
	}
	if got := w.Header().Get("Content-Security-Policy"); got != enrollmentCSP || !strings.Contains(got, "style-src 'sha256-"+styleDigest(enrollmentCSS)+"'") {
		t.Fatalf("unexpected CSP %q", got)
	}
	return flow
}
func extractCSRF(t *testing.T, body string) string {
	t.Helper()
	matches := csrfInput.FindStringSubmatch(body)
	if len(matches) != 2 {
		t.Fatal("CSRF field missing")
	}
	return matches[1]
}
func (f *browserFlow) get(path string, cookie *http.Cookie) *httptest.ResponseRecorder {
	w := httptest.NewRecorder()
	r := httptest.NewRequest("GET", EnrollOrigin+path, nil)
	if cookie != nil {
		r.AddCookie(cookie)
	}
	f.handler.ServeHTTP(w, r)
	return w
}
func (f *browserFlow) post(path string, values url.Values, cookie *http.Cookie, origin string) *httptest.ResponseRecorder {
	w := httptest.NewRecorder()
	r := httptest.NewRequest("POST", EnrollOrigin+path, strings.NewReader(values.Encode()))
	r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	r.Header.Set("Origin", origin)
	if cookie != nil {
		r.AddCookie(cookie)
	}
	f.handler.ServeHTTP(w, r)
	return w
}
func (f *browserFlow) authorize(t *testing.T) {
	t.Helper()
	w := f.post("/enroll/native/authorize", url.Values{"transaction": {f.started.Transaction}, "csrf": {f.csrf}}, f.cookie, EnrollOrigin)
	if w.Code != 303 {
		t.Fatalf("authorize failed %d", w.Code)
	}
	location, err := url.Parse(w.Header().Get("Location"))
	if err != nil || location.Scheme+"://"+location.Host+location.Path != authorizeURL || location.Query().Get("redirect_uri") != EnrollOrigin+"/enroll/gitcode/callback" || location.Query().Get("scope") != gitcodeScope {
		t.Fatal("unsafe OAuth redirect")
	}
	f.state = location.Query().Get("state")
}
func (f *browserFlow) callback() *httptest.ResponseRecorder {
	return f.get("/enroll/gitcode/callback?"+url.Values{"state": {f.state}, "code": {"oauth-code-canary"}, "client_id": {"fixture-app"}, "scopes": {"all_user,read_groups"}, "response_type": {"code"}}.Encode(), f.cookie)
}

func TestBrowserStateCookieCSRFAndFixedRedirect(t *testing.T) {
	f := newBrowserFlow(t)
	values := url.Values{"transaction": {f.started.Transaction}, "csrf": {f.csrf}}
	if w := f.post("/enroll/native/authorize", values, f.cookie, "https://attacker.invalid"); w.Code != 400 {
		t.Fatal("cross-origin write accepted")
	}
	values.Set("redirect_uri", "https://attacker.invalid/callback")
	if w := f.post("/enroll/native/authorize", values, f.cookie, EnrollOrigin); w.Code != 400 {
		t.Fatal("callback override accepted")
	}
	values.Del("redirect_uri")
	values.Set("csrf", "wrong")
	if w := f.post("/enroll/native/authorize", values, f.cookie, EnrollOrigin); w.Code != 400 {
		t.Fatal("invalid csrf accepted")
	}
	f.authorize(t)
	wrongValue, _ := randomProof()
	wrongCookie := *f.cookie
	wrongCookie.Value = wrongValue
	path := "/enroll/gitcode/callback?" + url.Values{"state": {f.state}, "code": {"oauth-code-canary"}}.Encode()
	if w := f.get(path, &wrongCookie); w.Code != 400 || f.exchanges.Load() != 0 {
		t.Fatal("wrong cookie consumed state")
	}
	w := f.callback()
	if w.Code != 200 || f.exchanges.Load() != 1 {
		t.Fatalf("callback failed %d", w.Code)
	}
	if strings.Contains(w.Body.String(), "<script>") || !strings.Contains(w.Body.String(), f.started.ShortCode) || !strings.Contains(w.Body.String(), "new-member-id") || !strings.Contains(w.Body.String(), "1783195") {
		t.Fatal("unsafe/incomplete approval summary")
	}
	if w := f.callback(); w.Code != 400 || f.exchanges.Load() != 1 {
		t.Fatal("OAuth state replay exchanged code")
	}
	f.csrf = extractCSRF(t, w.Body.String())
	_, _, journalDir := databaseFiles(t, f.s.store)
	journal, err := os.ReadFile(filepath.Join(journalDir, journalName))
	if err != nil {
		t.Fatal(err)
	}
	for _, secret := range []string{"oauth-code-canary", "oauth-access-canary", "oauth-refresh-canary", "oauth-private-secret-canary", f.proof.Verifier, f.cookie.Value, f.state, f.csrf} {
		if strings.Contains(f.logs.String()+string(journal)+w.Body.String(), secret) && secret != f.csrf {
			t.Fatal("browser/control-plane secret leaked")
		}
	}
}

func TestTrustedMemberApprovalAndDenial(t *testing.T) {
	for _, decision := range []string{"approve", "deny"} {
		t.Run(decision, func(t *testing.T) {
			f := newBrowserFlow(t)
			f.authorize(t)
			w := f.callback()
			if w.Code != 200 {
				t.Fatal(w.Code)
			}
			csrf := extractCSRF(t, w.Body.String())
			values := url.Values{"transaction": {f.started.Transaction}, "csrf": {csrf}, "decision": {decision}}
			if bad := f.post("/enroll/native/approve", url.Values{"transaction": {f.started.Transaction}, "decision": {decision}}, f.cookie, EnrollOrigin); bad.Code != 400 {
				t.Fatal("approval without csrf")
			}
			if w := f.post("/enroll/native/approve", values, f.cookie, EnrollOrigin); w.Code != 200 {
				t.Fatalf("decision failed %d", w.Code)
			}
			v, err := readNative(context.Background(), f.s.store.db, f.started.Transaction)
			if err != nil {
				t.Fatal(err)
			}
			want := "approved"
			if decision == "deny" {
				want = "denied"
			}
			if v.Status != want || v.UserID != "new-member-id" {
				t.Fatal("untrusted approval", v.Status)
			}
			if w := f.post("/enroll/native/approve", values, f.cookie, EnrollOrigin); w.Code == 200 {
				t.Fatal("decision replay accepted")
			}
		})
	}
}

func TestNonmemberAndLateOAuthCannotApprove(t *testing.T) {
	f := newBrowserFlow(t)
	f.member.Store(false)
	f.authorize(t)
	if w := f.callback(); w.Code != 403 {
		t.Fatal("nonmember entered device approval", w.Code)
	}
	var accounts int
	if f.s.store.db.QueryRow("SELECT count(*) FROM users WHERE user_id='new-member-id'").Scan(&accounts) != nil || accounts != 0 {
		t.Fatal("nonmember token/account persisted")
	}
	g := newBrowserFlow(t)
	g.authorize(t)
	g.duringExchange = func() {
		if code := g.s.cancel(context.Background(), g.proof); code != "" {
			t.Error(code)
		}
	}
	if w := g.callback(); w.Code == 200 {
		t.Fatal("late callback restored cancelled transaction")
	}
	v, err := readNative(context.Background(), g.s.store.db, g.started.Transaction)
	if err != nil || v.Status != "cancelled" {
		t.Fatal("cancel lost", err)
	}
}

func TestPollingDoesNotInvalidateOAuthCompletion(t *testing.T) {
	f := newBrowserFlow(t)
	f.authorize(t)
	f.duringExchange = func() {
		if _, code := f.s.poll(context.Background(), f.proof); code != "" {
			t.Error(code)
		}
	}
	if w := f.callback(); w.Code != 200 {
		t.Fatal("normal native poll invalidated browser exchange", w.Code)
	}
}

func TestFinalApprovalRechecksMembership(t *testing.T) {
	f := newBrowserFlow(t)
	f.authorize(t)
	page := f.callback()
	if page.Code != 200 {
		t.Fatal(page.Code)
	}
	f.member.Store(false)
	w := f.post("/enroll/native/approve", url.Values{"transaction": {f.started.Transaction}, "csrf": {extractCSRF(t, page.Body.String())}, "decision": {"approve"}}, f.cookie, EnrollOrigin)
	if w.Code != 403 {
		t.Fatal("member removal between OAuth and final approval accepted", w.Code)
	}
	v, err := readNative(context.Background(), f.s.store.db, f.started.Transaction)
	if err != nil || v.Status == "approved" {
		t.Fatal("inactive account approved")
	}
}

func TestGitCodeCallbackFailuresStayDistinct(t *testing.T) {
	const tokenOK = `{"access_token":"oauth-access-canary","refresh_token":"oauth-refresh-canary","expires_in":300,"scope":"read_user read_groups"}`
	const userOK = `{"id":"new-member-id","login":"member-canary"}`
	cases := []struct {
		name, tokenBody, memberBody, wantCode, wantReason string
		tokenStatus, memberStatus, wantHTTP, wantProvider int
	}{
		{"not-visible", tokenOK, `{"private":"account-canary"}`, MembershipRequired, "membership_not_found", 200, 404, 403, 404},
		{"scope", tokenOK, `{"error":"insufficient_scope","access_token":"scope-canary"}`, ReauthRequired, "insufficient_scope", 200, 403, 401, 403},
		{"outage", tokenOK, `upstream-diagnostic-canary`, ServiceUnavailable, "membership_lookup_unavailable", 200, 503, 503, 503},
		{"token-scope", `{"error":"insufficient_scope","access_token":"leaked-token-canary"}`, `{}`, ReauthRequired, "insufficient_scope", 403, 200, 401, 403},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			f := newBrowserFlow(t)
			f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
				status, body := 500, `{}`
				switch r.URL.Scheme + "://" + r.URL.Host + r.URL.Path {
				case tokenURL:
					status, body = test.tokenStatus, test.tokenBody
				case userURL:
					status, body = 200, userOK
				case membershipURL + "msinsight-org":
					status, body = test.memberStatus, test.memberBody
				default:
					t.Errorf("unexpected provider target")
				}
				return &http.Response{StatusCode: status, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(body)), Request: r}, nil
			})
			f.authorize(t)
			w := f.callback()
			if w.Code != test.wantHTTP || !strings.Contains(w.Body.String(), test.wantCode) {
				t.Fatalf("callback %d %s", w.Code, w.Body.String())
			}
			if _, code := f.s.poll(context.Background(), f.proof); code != test.wantCode {
				t.Fatal("poll collapsed the failure", code)
			}
			v, err := readNative(context.Background(), f.s.store.db, f.started.Transaction)
			if err != nil || v.FailureCode != test.wantCode || v.UserID != "" {
				t.Fatalf("ledger %+v %v", v, err)
			}
			var accounts, revoked int
			if f.s.store.db.QueryRow("SELECT count(*) FROM users WHERE user_id='new-member-id'").Scan(&accounts) != nil || accounts != 0 || f.s.store.db.QueryRow("SELECT count(*) FROM revocations WHERE reason='membership_lost'").Scan(&revoked) != nil || revoked != 0 {
				t.Fatal("ambiguous provider result revoked or stored an account")
			}
			logged := f.logs.String()
			if !strings.Contains(logged, `"msg":"enroll_provider_failure"`) || !strings.Contains(logged, `"reason":"`+test.wantReason+`"`) || !strings.Contains(logged, `"provider_status":`+strconv.Itoa(test.wantProvider)) {
				t.Fatalf("missing sanitized failure log: %s", logged)
			}
			for _, secret := range []string{"oauth-access-canary", "oauth-refresh-canary", "leaked-token-canary", "scope-canary", "account-canary", "upstream-diagnostic-canary", "member-canary", "oauth-code-canary", f.proof.Verifier} {
				if strings.Contains(logged, secret) || strings.Contains(w.Body.String(), secret) {
					t.Fatal("provider diagnostic leaked", secret)
				}
			}
		})
	}
}
