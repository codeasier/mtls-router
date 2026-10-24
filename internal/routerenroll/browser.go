package routerenroll

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"errors"
	"html/template"
	"net/http"
	"net/url"
)

const browserCookie = "__Host-codeasier-enroll"

type browserTransaction struct {
	nativeTransaction
	BrowserHash, CSRFHash, StateHash string
	StateUsed                        bool
	AccountJSON                      string
}

func readBrowser(ctx context.Context, reader policyReader, id string) (browserTransaction, error) {
	v, err := readNative(ctx, reader, id)
	if err != nil {
		return browserTransaction{}, err
	}
	var b browserTransaction
	b.nativeTransaction = v
	err = reader.QueryRowContext(ctx, "SELECT browser_hash,csrf_hash,COALESCE(state_hash,''),state_used,account_json FROM enrollment_transactions WHERE id=?", id).Scan(&b.BrowserHash, &b.CSRFHash, &b.StateHash, &b.StateUsed, &b.AccountJSON)
	return b, err
}
func sameDigest(a, b string) bool {
	return len(a) == 64 && len(b) == 64 && subtle.ConstantTimeCompare([]byte(a), []byte(b)) == 1
}
func browserBinding(r *http.Request) string {
	value := ""
	count := 0
	for _, cookie := range r.Cookies() {
		if cookie.Name == browserCookie {
			value = cookie.Value
			count++
		}
	}
	if count != 1 || !validProof(value) {
		return ""
	}
	return value
}
func strictValues(values url.Values, allowed ...string) bool {
	set := map[string]bool{}
	for _, key := range allowed {
		set[key] = true
	}
	for key, value := range values {
		if !set[key] || len(value) != 1 {
			return false
		}
	}
	return true
}
func browserForm(w http.ResponseWriter, r *http.Request, allowed ...string) bool {
	if r.Header.Get("Origin") != EnrollOrigin || r.Header.Get("Content-Type") != "application/x-www-form-urlencoded" || r.URL.RawQuery != "" || r.ParseForm() != nil || !strictValues(r.PostForm, allowed...) {
		WriteError(w, InvalidRequest)
		return false
	}
	return true
}

const enrollmentCSS = `:root{color-scheme:light dark;--ink:#302a25;--muted:#74695f;--paper:#f7f2ea;--paper-deep:#eee6dc;--surface:#fffaf4;--surface-subtle:#fbf7f1;--line:#dfd3c5;--line-strong:#cdbba9;--signal:#b94722;--signal-hover:#963719;--signal-soft:#f8e3d8;--on-signal:#fffaf4;--good:#486331;--good-soft:#e7eddf;--danger:#963c2f;--danger-hover:#70291f;--danger-soft:#f6e1dc;--shadow:0 20px 50px rgba(64,45,30,.10),0 4px 14px rgba(64,45,30,.05)}@media(prefers-color-scheme:dark){:root{--ink:#f3ebe2;--muted:#b6aa9e;--paper:#161311;--paper-deep:#1f1b17;--surface:#24201c;--surface-subtle:#1c1815;--line:#3d352d;--line-strong:#564c42;--signal:#e07a4c;--signal-hover:#f08a5c;--signal-soft:#3a241c;--on-signal:#1c1916;--good:#9bb87a;--good-soft:#24301c;--danger:#e08a7a;--danger-hover:#f0a090;--danger-soft:#3a221e;--shadow:0 24px 60px rgba(0,0,0,.45),0 4px 16px rgba(0,0,0,.28)}}*{box-sizing:border-box}html,body{margin:0;min-height:100%;font-family:"Avenir Next","PingFang SC","Hiragino Sans GB","Microsoft YaHei","Segoe UI",system-ui,sans-serif;color:var(--ink);background:radial-gradient(circle at 50% 0%,var(--signal-soft) 0%,transparent 58%),var(--paper);-webkit-font-smoothing:antialiased}body{display:flex;align-items:center;justify-content:center;padding:28px 16px}main{width:100%;max-width:480px;background:var(--surface);border:1px solid var(--line);border-radius:20px;box-shadow:var(--shadow);padding:28px;position:relative;overflow:hidden}.brand{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:20px}.brand-id{display:flex;align-items:center;gap:10px}.brand-mark{width:36px;height:36px;border-radius:10px;background:var(--signal-soft);color:var(--signal);display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--line-strong);flex-shrink:0}.brand-mark svg{width:20px;height:20px}.brand-text{display:flex;flex-direction:column;gap:2px}.overline{font-size:11px;font-weight:700;letter-spacing:.08em;text-transform:uppercase;color:var(--signal)}.subline{font-size:12px;color:var(--muted)}.badge{font-size:12px;font-weight:600;padding:4px 10px;border-radius:999px;background:var(--paper-deep);color:var(--muted);border:1px solid var(--line);white-space:nowrap}.badge-good{background:var(--good-soft);color:var(--good);border-color:transparent}.badge-danger{background:var(--danger-soft);color:var(--danger);border-color:transparent}h1{margin:0 0 6px;font-size:22px;line-height:1.3;letter-spacing:-.01em}.subtitle{margin:0 0 20px;font-size:14px;line-height:1.55;color:var(--muted)}.code-box{background:var(--surface-subtle);border:1.5px dashed var(--line-strong);border-radius:14px;padding:16px 18px;margin-bottom:18px;display:flex;align-items:center;justify-content:space-between;gap:12px}.code-label{display:flex;flex-direction:column;gap:3px;font-size:12px;color:var(--muted)}.code-label span{font-weight:600;color:var(--ink);font-size:13px}.code-value{font-family:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;font-size:24px;font-weight:800;letter-spacing:.12em;color:var(--signal);background:var(--signal-soft);padding:6px 12px;border-radius:10px}.meta{background:var(--surface-subtle);border:1px solid var(--line);border-radius:14px;padding:6px 16px;margin-bottom:18px}.row{display:flex;align-items:baseline;justify-content:space-between;gap:12px;padding:11px 0;border-bottom:1px solid var(--line);font-size:13.5px}.row:last-child{border-bottom:none}.row-key{color:var(--muted);flex-shrink:0}.row-val{font-weight:600;color:var(--ink);text-align:right;word-break:break-word}.mono-id{font-family:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;font-size:11.5px;font-weight:500;color:var(--muted);margin-left:4px}.notice{margin:0 0 20px;padding:12px 14px;border-radius:12px;background:var(--paper-deep);font-size:13px;line-height:1.5;color:var(--ink)}.notice-good{background:var(--good-soft);color:var(--good)}.notice-danger{background:var(--danger-soft);color:var(--danger)}.notice-sub{display:block;margin-top:3px;font-size:12px;color:var(--muted)}form{margin:0}.actions{display:flex;gap:10px}.btn{appearance:none;border-radius:11px;font-family:inherit;font-size:14px;font-weight:600;padding:12px 18px;cursor:pointer;transition:background-color .15s ease,border-color .15s ease,transform .08s ease;display:inline-flex;align-items:center;justify-content:center;gap:6px;line-height:1.2}.btn:active{transform:translateY(1px)}.btn-primary{flex:1;background:var(--signal);color:var(--on-signal);border:1px solid transparent}.btn-primary:hover{background:var(--signal-hover)}.btn-deny{background:transparent;color:var(--danger);border:1px solid var(--line-strong);min-width:108px}.btn-deny:hover{background:var(--danger-soft);border-color:var(--danger)}.foot{margin-top:18px;padding-top:14px;border-top:1px solid var(--line);display:flex;align-items:center;justify-content:space-between;gap:8px;font-size:11.5px;color:var(--muted)}`

var (
	enrollmentCSP  = "default-src 'none'; style-src 'sha256-" + styleDigest(enrollmentCSS) + "'; form-action 'self' https://gitcode.com; frame-ancestors 'none'; base-uri 'none'"
	enrollmentPage = template.Must(template.New("enroll").Parse(`<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>CodeasierRouter device approval</title><style>` + enrollmentCSS + `</style></head><body><main><header class="brand"><div class="brand-id"><span class="brand-mark" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 4v5c0 4.5-3 8.2-7 9-4-.8-7-4.5-7-9V7l7-4z"/><path d="M9 12l2 2 4-4"/></svg></span><div class="brand-text"><span class="overline">CodeasierRouter</span><span class="subline">mTLS Device Identity · 设备证书控制面</span></div></div>{{if eq .Tone "approved"}}<span class="badge badge-good">已批准 · Approved</span>{{else if eq .Tone "denied"}}<span class="badge badge-danger">已拒绝 · Denied</span>{{else if .Account}}<span class="badge badge-good">GitCode 已核验 · Step 2/2</span>{{else if .Action}}<span class="badge">待核验 · Step 1/2</span>{{else}}<span class="badge">处理中 · In Progress</span>{{end}}</header><h1>CodeasierRouter device approval</h1><p class="subtitle">{{if eq .Tone "approved"}}设备证书授权已确认，请返回客户端完成自动连接。{{else if eq .Tone "denied"}}已拒绝本次设备证书请求，未签发任何证书。{{else if .Account}}请核对下方确认码、设备与 GitCode 组织身份，确认无误后批准此设备。{{else if .Action}}请核对客户端显示的确认码，并前往 GitCode 验证组织成员身份。{{else}}当前设备授权正在处理中，请返回客户端查看状态。{{end}}</p><section class="code-box" aria-label="Confirmation code"><div class="code-label"><span>确认码 · Confirmation code</span>核对与客户端显示是否一致</div><strong class="code-value">{{.Short}}</strong></section><section class="meta" aria-label="Request details"><div class="row"><span class="row-key">请求设备 · Device</span><span class="row-val">{{.Device}}</span></div>{{if .Account}}<div class="row"><span class="row-key">GitCode 账号 · Account</span><span class="row-val">{{.Account.Username}}<span class="mono-id">({{.Account.ID}})</span></span></div><div class="row"><span class="row-key">所属组织 · Organization</span><span class="row-val">{{.Account.Organization.Path}}<span class="mono-id">({{.Account.Organization.ID}})</span></span></div>{{end}}</section><p class="notice{{if eq .Tone "approved"}} notice-good{{else if eq .Tone "denied"}} notice-danger{{end}}">{{.Message}}{{if .Hint}}<span class="notice-sub">{{.Hint}}</span>{{end}}</p>{{if .Action}}<form method="post" action="{{.Action}}"><input type="hidden" name="transaction" value="{{.Transaction}}"><input type="hidden" name="csrf" value="{{.CSRF}}"><div class="actions">{{if .Account}}<button class="btn btn-primary" name="decision" value="approve">Approve this device · 批准此设备</button><button class="btn btn-deny" name="decision" value="deny">Deny · 拒绝</button>{{else}}<button class="btn btn-primary" type="submit">Continue with GitCode · 前往 GitCode 授权</button>{{end}}</div></form>{{end}}<footer class="foot"><span>enroll.codeasier.top</span><span>个人 mTLS 证书仅绑定当前设备</span></footer></main></body></html>`))
)

func styleDigest(css string) string {
	sum := sha256.Sum256([]byte(css))
	return base64.StdEncoding.EncodeToString(sum[:])
}

type browserPage struct {
	Device, Short, Message, Hint, Tone, Action, Transaction, CSRF string
	Account                                                       *Account
}

func renderEnrollment(w http.ResponseWriter, value browserPage) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Security-Policy", enrollmentCSP)
	_ = enrollmentPage.Execute(w, value)
}
func (s *Service) validBrowser(ctx context.Context, tx *writeTx, id, cookie, csrf string) (browserTransaction, error) {
	if !validProof(id) || !validProof(cookie) {
		return browserTransaction{}, errors.New(InvalidRequest)
	}
	v, err := readBrowser(ctx, tx, id)
	if err != nil {
		return v, errors.New(InvalidRequest)
	}
	p, err := readPolicy(ctx, tx)
	if err != nil {
		return v, ErrStorage
	}
	if p.Epoch != v.Epoch || s.now().Unix() >= v.ExpiresAt || v.Status == "cancelled" || v.Status == "expired" || v.Status == "denied" || v.Status == "issued" {
		return v, errors.New(EnrollmentExpired)
	}
	if !sameDigest(v.BrowserHash, Digest([]byte(cookie))) || (csrf != "" && (!validProof(csrf) || !sameDigest(v.CSRFHash, Digest([]byte(csrf))))) {
		return v, errors.New(InvalidRequest)
	}
	return v, nil
}

func (s *Service) verifyHTTP(w http.ResponseWriter, r *http.Request) {
	query, err := url.ParseQuery(r.URL.RawQuery)
	if err != nil || !strictValues(query, "transaction") || !validProof(query.Get("transaction")) {
		WriteError(w, InvalidRequest)
		return
	}
	id := query.Get("transaction")
	cookie := browserBinding(r)
	if cookie == "" {
		cookie, err = randomProof()
		if err != nil {
			WriteError(w, ServiceUnavailable)
			return
		}
	}
	csrf, err := randomProof()
	if err != nil {
		WriteError(w, ServiceUnavailable)
		return
	}
	var page browserPage
	err = s.store.transaction(r.Context(), func(tx *writeTx) error {
		v, err := readBrowser(r.Context(), tx, id)
		if err != nil {
			return errors.New(InvalidRequest)
		}
		p, err := readPolicy(r.Context(), tx)
		if err != nil {
			return ErrStorage
		}
		if p.Epoch != v.Epoch || s.now().Unix() >= v.ExpiresAt || v.Status == "cancelled" || v.Status == "expired" || v.Status == "denied" || v.Status == "issued" {
			return errors.New(EnrollmentExpired)
		}
		if v.BrowserHash != "" && !sameDigest(v.BrowserHash, Digest([]byte(cookie))) {
			return errors.New(InvalidRequest)
		}
		page = browserPage{Device: v.Label, Short: v.ShortCode, Transaction: v.ID, CSRF: csrf}
		if v.Status == "pending" || v.Status == "failed" {
			page.Action = "/enroll/native/authorize"
			page.Message = "Only continue if you started this device request."
			page.Hint = "仅当您本人在客户端发起该请求时继续；若非本人操作，请直接关闭此页面。"
		} else if v.Status == "verified" {
			var account Account
			if json.Unmarshal([]byte(v.AccountJSON), &account) != nil {
				return ErrStorage
			}
			page.Account = &account
			page.Action = "/enroll/native/approve"
			page.Message = "Check the account, device and confirmation code before approving."
			page.Hint = "批准后将为上述设备签发绑定该组织成员身份的个人 mTLS 证书。"
		} else {
			page.Message = "Authorization is in progress. Return to the native client."
			page.Hint = "授权流程正在进行或已提交，请返回 CodeasierRouter 客户端。"
			return nil
		}
		_, err = tx.ExecContext(r.Context(), "UPDATE enrollment_transactions SET browser_hash=?,csrf_hash=?,version=version+1 WHERE id=?", Digest([]byte(cookie)), Digest([]byte(csrf)), id)
		return err
	})
	if err != nil {
		WriteError(w, publicCode(err))
		return
	}
	http.SetCookie(w, &http.Cookie{Name: browserCookie, Value: cookie, Path: "/", Secure: true, HttpOnly: true, SameSite: http.SameSiteLaxMode, MaxAge: int(EnrollmentTTL.Seconds())})
	renderEnrollment(w, page)
}
func (s *Service) authorizeHTTP(w http.ResponseWriter, r *http.Request) {
	if !browserForm(w, r, "transaction", "csrf") {
		return
	}
	id, csrf := r.PostForm.Get("transaction"), r.PostForm.Get("csrf")
	if csrf == "" {
		WriteError(w, InvalidRequest)
		return
	}
	state, err := randomProof()
	if err != nil {
		WriteError(w, ServiceUnavailable)
		return
	}
	err = s.store.transaction(r.Context(), func(tx *writeTx) error {
		v, err := s.validBrowser(r.Context(), tx, id, browserBinding(r), csrf)
		if err != nil {
			return err
		}
		if v.Status != "pending" && v.Status != "failed" {
			return errors.New(InvalidRequest)
		}
		_, err = tx.ExecContext(r.Context(), "UPDATE enrollment_transactions SET status='authorizing',state_hash=?,state_used=0,csrf_hash='',version=version+1 WHERE id=?", Digest([]byte(state)), id)
		return err
	})
	if err != nil {
		WriteError(w, publicCode(err))
		return
	}
	w.Header().Set("Location", s.provider.AuthorizationURL(state))
	w.WriteHeader(http.StatusSeeOther)
}

func (s *Service) callbackHTTP(w http.ResponseWriter, r *http.Request) {
	query, err := url.ParseQuery(r.URL.RawQuery)
	if err != nil || !strictValues(query, "state", "code", "error", "error_description", "client_id", "scopes", "response_type") || !validProof(query.Get("state")) || len(query.Get("code")) > 4096 || len(query.Get("error")) > 128 || len(query.Get("error_description")) > 1024 || (query.Get("client_id") != "" && query.Get("client_id") != s.provider.clientID) || (query.Get("response_type") != "" && query.Get("response_type") != "code") {
		WriteError(w, InvalidRequest)
		return
	}
	cookie := browserBinding(r)
	var claimed browserTransaction
	err = s.store.transaction(r.Context(), func(tx *writeTx) error {
		var id string
		if tx.QueryRowContext(r.Context(), "SELECT id FROM enrollment_transactions WHERE state_hash=?", Digest([]byte(query.Get("state")))).Scan(&id) != nil {
			return errors.New(InvalidRequest)
		}
		v, err := s.validBrowser(r.Context(), tx, id, cookie, "")
		if err != nil {
			return err
		}
		if v.Status != "authorizing" || v.StateUsed {
			return errors.New(InvalidRequest)
		}
		_, err = tx.ExecContext(r.Context(), "UPDATE enrollment_transactions SET state_used=1,status='verifying',version=version+1 WHERE id=?", id)
		claimed = v
		claimed.Version++
		claimed.Status = "verifying"
		return err
	})
	if err != nil {
		WriteError(w, publicCode(err))
		return
	}
	if query.Get("error") != "" || query.Get("code") == "" {
		s.finishBrowserFailure(r.Context(), claimed, EnrollmentDenied)
		WriteError(w, EnrollmentDenied)
		return
	}
	token, err := s.provider.Exchange(r.Context(), query.Get("code"))
	if err != nil {
		s.finishBrowserFailure(r.Context(), claimed, ServiceUnavailable)
		WriteError(w, ServiceUnavailable)
		return
	}
	account, err := s.provider.User(r.Context(), token.AccessToken)
	if err != nil {
		s.finishBrowserFailure(r.Context(), claimed, ServiceUnavailable)
		WriteError(w, ServiceUnavailable)
		return
	}
	policy, err := readPolicy(r.Context(), s.store.db)
	if err != nil {
		WriteError(w, ServiceUnavailable)
		return
	}
	checkedVersion, err := s.memberVersion(r.Context(), claimed.Epoch, account.ID)
	if err != nil {
		WriteError(w, ServiceUnavailable)
		return
	}
	decision, err := s.provider.Membership(r.Context(), token.AccessToken, account, policy)
	if err != nil || decision.Code != "" {
		code := ServiceUnavailable
		if decision.Code != "" {
			code = decision.Code
		}
		if err == nil && code == MembershipRequired {
			if s.recordMembershipLoss(r.Context(), claimed.Epoch, account.ID, checkedVersion) != nil {
				code = ServiceUnavailable
			}
		}
		s.finishBrowserFailure(r.Context(), claimed, code)
		WriteError(w, code)
		return
	}
	account.Organization = decision.Organization
	csrf, err := randomProof()
	if err != nil {
		WriteError(w, ServiceUnavailable)
		return
	}
	err = s.store.transaction(r.Context(), func(tx *writeTx) error {
		v, err := s.validBrowser(r.Context(), tx, claimed.ID, cookie, "")
		if err != nil {
			return err
		}
		if v.Status != "verifying" || v.Version != claimed.Version {
			return errors.New(OperationConflict)
		}
		if err := bindOrganization(r.Context(), tx, v.Epoch, account.Organization); err != nil {
			return err
		}
		if err := s.store.saveGitCodeToken(r.Context(), tx, v.Epoch, account, token); err != nil {
			return err
		}
		encoded, err := json.Marshal(account)
		if err != nil {
			return ErrStorage
		}
		_, err = tx.ExecContext(r.Context(), "UPDATE enrollment_transactions SET status='verified',user_id=?,account_json=?,csrf_hash=?,version=version+1 WHERE id=?", account.ID, string(encoded), Digest([]byte(csrf)), v.ID)
		return err
	})
	if err != nil {
		WriteError(w, publicCode(err))
		return
	}
	renderEnrollment(w, browserPage{Device: claimed.Label, Short: claimed.ShortCode, Message: "Check the account, device and confirmation code before approving.", Hint: "批准后将为上述设备签发绑定该组织成员身份的个人 mTLS 证书。", Action: "/enroll/native/approve", Transaction: claimed.ID, CSRF: csrf, Account: &account})
}
func (s *Service) finishBrowserFailure(ctx context.Context, claimed browserTransaction, code string) {
	_ = s.store.transaction(ctx, func(tx *writeTx) error {
		status := "failed"
		if code == EnrollmentDenied || code == MembershipRequired {
			status = "denied"
		}
		_, err := tx.ExecContext(ctx, "UPDATE enrollment_transactions SET status=?,failure_code=?,version=version+1 WHERE id=? AND version=? AND status='verifying'", status, code, claimed.ID, claimed.Version)
		return err
	})
}

func (s *Service) approveHTTP(w http.ResponseWriter, r *http.Request) {
	if !browserForm(w, r, "transaction", "csrf", "decision") {
		return
	}
	id, csrf, decision := r.PostForm.Get("transaction"), r.PostForm.Get("csrf"), r.PostForm.Get("decision")
	if csrf == "" || (decision != "approve" && decision != "deny") {
		WriteError(w, InvalidRequest)
		return
	}
	var claimed browserTransaction
	err := s.store.transaction(r.Context(), func(tx *writeTx) error {
		v, err := s.validBrowser(r.Context(), tx, id, browserBinding(r), csrf)
		if err != nil {
			return err
		}
		if v.Status != "verified" {
			return errors.New(InvalidRequest)
		}
		claimed = v
		return nil
	})
	if err != nil {
		WriteError(w, publicCode(err))
		return
	}
	var account Account
	if json.Unmarshal([]byte(claimed.AccountJSON), &account) != nil {
		WriteError(w, ServiceUnavailable)
		return
	}
	var version int64
	var checkedVersion int64
	if decision == "approve" {
		var encrypted []byte
		if s.store.db.QueryRowContext(r.Context(), "SELECT t.version,t.ciphertext,u.version FROM gitcode_tokens t JOIN users u ON u.epoch=t.epoch AND u.user_id=t.user_id WHERE t.epoch=? AND t.user_id=?", claimed.Epoch, claimed.UserID).Scan(&version, &encrypted, &checkedVersion) != nil {
			WriteError(w, ServiceUnavailable)
			return
		}
		var token GitCodeToken
		if s.store.decrypt(encrypted, tokenAAD(claimed.Epoch, claimed.UserID, version), &token) != nil {
			WriteError(w, ServiceUnavailable)
			return
		}
		policy, err := readPolicy(r.Context(), s.store.db)
		if err != nil {
			WriteError(w, ServiceUnavailable)
			return
		}
		membership, err := s.provider.Membership(r.Context(), token.AccessToken, account, policy)
		if err != nil || membership.Code != "" {
			code := ServiceUnavailable
			if membership.Code != "" {
				code = membership.Code
			}
			if err == nil && code == MembershipRequired {
				if s.recordMembershipLoss(r.Context(), claimed.Epoch, account.ID, checkedVersion) != nil {
					code = ServiceUnavailable
				} else if s.store.transaction(r.Context(), func(tx *writeTx) error {
					_, err := tx.ExecContext(r.Context(), "UPDATE enrollment_transactions SET status='denied',failure_code=?,version=version+1 WHERE id=? AND version=? AND status='verified'", MembershipRequired, claimed.ID, claimed.Version)
					return err
				}) != nil {
					code = ServiceUnavailable
				}
			}
			WriteError(w, code)
			return
		}
	}
	err = s.store.transaction(r.Context(), func(tx *writeTx) error {
		v, err := s.validBrowser(r.Context(), tx, id, browserBinding(r), csrf)
		if err != nil {
			return err
		}
		if v.Status != "verified" || v.Version != claimed.Version {
			return errors.New(OperationConflict)
		}
		if decision == "approve" {
			var current int64
			if tx.QueryRowContext(r.Context(), "SELECT version FROM gitcode_tokens WHERE epoch=? AND user_id=?", v.Epoch, v.UserID).Scan(&current) != nil || current != version {
				return errors.New(OperationConflict)
			}
		}
		status := "denied"
		if decision == "approve" {
			status = "approved"
		}
		_, err = tx.ExecContext(r.Context(), "UPDATE enrollment_transactions SET status=?,csrf_hash='',version=version+1 WHERE id=?", status, id)
		return err
	})
	if err != nil {
		WriteError(w, publicCode(err))
		return
	}
	tone := "denied"
	hint := "已拒绝本次设备授权请求，现在可以安全关闭此浏览器标签页。"
	if decision == "approve" {
		tone = "approved"
		hint = "设备批准已生效，客户端将在数秒内自动完成证书领取与连接，现在可以关闭此页面。"
	}
	renderEnrollment(w, browserPage{Device: claimed.Label, Short: claimed.ShortCode, Account: &account, Tone: tone, Message: "Device decision saved. Return to the native client.", Hint: hint})
}
