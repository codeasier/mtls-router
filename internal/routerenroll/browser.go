package routerenroll

import (
	"context"
	"crypto/subtle"
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

var enrollmentPage = template.Must(template.New("enroll").Parse(`<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>CodeasierRouter device approval</title></head><body><main><h1>CodeasierRouter device approval</h1><p>Device: {{.Device}}</p><p>Confirmation code: <strong>{{.Short}}</strong></p>{{if .Account}}<p>GitCode account: {{.Account.Username}} ({{.Account.ID}})</p><p>Organization: {{.Account.Organization.Path}} ({{.Account.Organization.ID}})</p>{{end}}<p>{{.Message}}</p>{{if .Action}}<form method="post" action="{{.Action}}"><input type="hidden" name="transaction" value="{{.Transaction}}"><input type="hidden" name="csrf" value="{{.CSRF}}">{{if .Account}}<button name="decision" value="approve">Approve this device</button><button name="decision" value="deny">Deny</button>{{else}}<button type="submit">Continue with GitCode</button>{{end}}</form>{{end}}</main></body></html>`))

type browserPage struct {
	Device, Short, Message, Action, Transaction, CSRF string
	Account                                           *Account
}

func renderEnrollment(w http.ResponseWriter, value browserPage) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Security-Policy", "default-src 'none'; form-action 'self' https://gitcode.com; frame-ancestors 'none'; base-uri 'none'")
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
		} else if v.Status == "verified" {
			var account Account
			if json.Unmarshal([]byte(v.AccountJSON), &account) != nil {
				return ErrStorage
			}
			page.Account = &account
			page.Action = "/enroll/native/approve"
			page.Message = "Check the account, device and confirmation code before approving."
		} else {
			page.Message = "Authorization is in progress. Return to the native client."
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
	renderEnrollment(w, browserPage{Device: claimed.Label, Short: claimed.ShortCode, Message: "Check the account, device and confirmation code before approving.", Action: "/enroll/native/approve", Transaction: claimed.ID, CSRF: csrf, Account: &account})
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
	renderEnrollment(w, browserPage{Device: claimed.Label, Short: claimed.ShortCode, Account: &account, Message: "Device decision saved. Return to the native client."})
}
