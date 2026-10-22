package routerenroll

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/base32"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"sync"
	"time"
)

const MaxPendingEnrollments = 1024

type Service struct {
	store     *Store
	provider  *GitCode
	issuer    *Issuer
	now       func() time.Time
	slots     chan struct{}
	starts    startBudget
	checksMu  sync.Mutex
	checks    map[string]*memberCheck
	publisher *Publisher
}

func (s *Service) SetPublisher(p *Publisher) { s.publisher = p }

func NewService(store *Store, provider *GitCode, issuer *Issuer) *Service {
	return &Service{store: store, provider: provider, issuer: issuer, now: time.Now, slots: make(chan struct{}, MaxConcurrentControlRequests)}
}
func (s *Service) Handler() http.Handler {
	return s.boundedControl(newHandler(func(ctx context.Context) bool { return s.store.Ready(ctx) && s.issuer.Ready(s.now()) }, map[string]http.HandlerFunc{
		"/enroll/native/start": s.startHTTP, "/enroll/native/poll": s.pollHTTP, "/enroll/native/cancel": s.cancelHTTP,
		"/enroll/native/verify": s.verifyHTTP, "/enroll/native/authorize": s.authorizeHTTP, "/enroll/gitcode/callback": s.callbackHTTP, "/enroll/native/approve": s.approveHTTP,
		RenewRecoverPath: s.recoverRenewalHTTP,
		DeviceRevokePath: s.revokeDeviceHTTP,
	}))
}

type NativeStarted struct {
	Version         string `json:"version"`
	Transaction     string `json:"transaction"`
	VerificationURI string `json:"verification_uri"`
	ShortCode       string `json:"short_code"`
	ExpiresIn       int64  `json:"expires_in"`
	Interval        int64  `json:"interval"`
	SPKIDigest      string `json:"spki_digest"`
}
type NativeProof struct {
	Transaction string `json:"transaction"`
	Verifier    string `json:"verifier"`
}
type NativeStatus struct {
	Status   string `json:"status"`
	Interval int64  `json:"interval"`
}
type nativeTransaction struct {
	ID                                                                         string
	Epoch                                                                      int64
	DeviceID, Kind, Label, CSRPEM, CSRDigest, SPKIDigest, Challenge, ShortCode string
	ExpiresAt, NextPoll                                                        int64
	Status, UserID                                                             string
	Version                                                                    int64
	FailureCode                                                                string
}

func randomProof() (string, error) {
	var value [32]byte
	if _, err := rand.Read(value[:]); err != nil {
		return "", ErrStorage
	}
	return base64.RawURLEncoding.EncodeToString(value[:]), nil
}
func randomShortCode() (string, error) {
	var value [5]byte
	if _, err := rand.Read(value[:]); err != nil {
		return "", ErrStorage
	}
	code := base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(value[:])
	return code[:4] + "-" + code[4:], nil
}
func publicCode(err error) string {
	if err == nil {
		return ""
	}
	if _, ok := publicErrors[err.Error()]; ok {
		return err.Error()
	}
	return ServiceUnavailable
}
func readNative(ctx context.Context, reader policyReader, id string) (nativeTransaction, error) {
	var v nativeTransaction
	err := reader.QueryRowContext(ctx, `SELECT id,epoch,device_id,kind,label,csr,csr_digest,spki_digest,challenge,short_code,expires_at,next_poll,status,user_id,version,failure_code FROM enrollment_transactions WHERE id=?`, id).Scan(&v.ID, &v.Epoch, &v.DeviceID, &v.Kind, &v.Label, &v.CSRPEM, &v.CSRDigest, &v.SPKIDigest, &v.Challenge, &v.ShortCode, &v.ExpiresAt, &v.NextPoll, &v.Status, &v.UserID, &v.Version, &v.FailureCode)
	return v, err
}
func readProvenNative(ctx context.Context, reader policyReader, proof NativeProof) (nativeTransaction, error) {
	if !validProof(proof.Transaction) || !validProof(proof.Verifier) {
		return nativeTransaction{}, errors.New(InvalidProof)
	}
	v, err := readNative(ctx, reader, proof.Transaction)
	if errors.Is(err, sql.ErrNoRows) || err == nil && !VerifyProof(proof.Verifier, v.Challenge) {
		return nativeTransaction{}, errors.New(InvalidProof)
	}
	if err != nil {
		return nativeTransaction{}, ErrStorage
	}
	return v, nil
}

func (s *Service) start(ctx context.Context, request NativeStart) (NativeStarted, string) {
	binding, code := request.Validate()
	if code != "" {
		return NativeStarted{}, code
	}
	id, err := randomProof()
	if err != nil {
		return NativeStarted{}, ServiceUnavailable
	}
	short, err := randomShortCode()
	if err != nil {
		return NativeStarted{}, ServiceUnavailable
	}
	now := s.now()
	err = s.store.transaction(ctx, func(tx *writeTx) error {
		policy, err := readPolicy(ctx, tx)
		if err != nil {
			return ErrStorage
		}
		var active int
		if tx.QueryRowContext(ctx, `SELECT count(*) FROM enrollment_transactions WHERE expires_at>? AND status NOT IN ('expired','cancelled','denied','issued')`, now.Unix()).Scan(&active) != nil {
			return ErrStorage
		}
		if active >= MaxPendingEnrollments {
			return errors.New(CapacityReached)
		}
		_, err = tx.ExecContext(ctx, `INSERT INTO enrollment_transactions(id,epoch,device_id,kind,label,csr,csr_digest,spki_digest,challenge,short_code,expires_at,status) VALUES(?,?,?,?,?,?,?,?,?,?,?,'pending')`, id, policy.Epoch, request.DeviceID, request.DeviceKind, request.DeviceLabel, request.CSRPEM, binding.CSRDigest, binding.SPKIDigest, request.Challenge, short, now.Add(EnrollmentTTL).Unix())
		return err
	})
	if err != nil {
		return NativeStarted{}, publicCode(err)
	}
	link, _ := VerificationURL(id)
	return NativeStarted{ProtocolVersion, id, link, short, int64(EnrollmentTTL.Seconds()), int64(PollInterval.Seconds()), binding.SPKIDigest}, ""
}
func (s *Service) poll(ctx context.Context, proof NativeProof) (NativeStatus, string) {
	var status string
	var failureCode string
	err := s.store.transaction(ctx, func(tx *writeTx) error {
		v, err := readProvenNative(ctx, tx, proof)
		if err != nil {
			return err
		}
		policy, err := readPolicy(ctx, tx)
		if err != nil {
			return ErrStorage
		}
		now := s.now().Unix()
		status = v.Status
		failureCode = v.FailureCode
		if status == "revocation_pending" {
			return nil
		}
		if v.Status != "issued" && (now >= v.ExpiresAt || policy.Epoch != v.Epoch) {
			status = "expired"
			_, err = tx.ExecContext(ctx, "UPDATE enrollment_transactions SET status='expired',version=version+1 WHERE id=?", v.ID)
			return err
		}
		if status == "cancelled" || status == "expired" || status == "denied" {
			return nil
		}
		if now < v.NextPoll {
			return errors.New(SlowDown)
		}
		_, err = tx.ExecContext(ctx, "UPDATE enrollment_transactions SET next_poll=? WHERE id=?", now+int64(PollInterval.Seconds()), v.ID)
		return err
	})
	if err != nil {
		return NativeStatus{}, publicCode(err)
	}
	switch status {
	case "expired":
		return NativeStatus{}, EnrollmentExpired
	case "cancelled":
		return NativeStatus{}, EnrollmentCancelled
	case "denied":
		if failureCode == MembershipRequired {
			return NativeStatus{}, MembershipRequired
		}
		return NativeStatus{}, EnrollmentDenied
	case "failed":
		return NativeStatus{}, ServiceUnavailable
	case "revocation_pending":
		return NativeStatus{}, ClientCertRejected
	}
	return NativeStatus{status, int64(PollInterval.Seconds())}, ""
}
func (s *Service) cancel(ctx context.Context, proof NativeProof) string {
	_, code := s.cancelResult(ctx, proof)
	return code
}
func (s *Service) cancelResult(ctx context.Context, proof NativeProof) (NativeStatus, string) {
	status := "cancelled"
	err := s.store.transaction(ctx, func(tx *writeTx) error {
		v, err := readProvenNative(ctx, tx, proof)
		if err != nil {
			return err
		}
		if v.Status == "revocation_pending" {
			status = "revocation_pending"
			return nil
		}
		if v.Status == "issued" {
			if _, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,serial,?,'native_cancel' FROM certificates WHERE request_id=?`, s.now().Unix(), nativeOperation(v.ID)); err != nil {
				return ErrStorage
			}
			status = "revocation_pending"
			_, err := tx.ExecContext(ctx, "UPDATE enrollment_transactions SET status='revocation_pending',version=version+1 WHERE id=?", v.ID)
			return err
		}
		if v.Status == "cancelled" {
			return nil
		}
		_, err = tx.ExecContext(ctx, "UPDATE enrollment_transactions SET status='cancelled',version=version+1 WHERE id=?", v.ID)
		return err
	})
	if err != nil {
		return NativeStatus{}, publicCode(err)
	}
	if status == "revocation_pending" && s.publisher != nil {
		if publication, err := s.publisher.Status(ctx, s.now()); err == nil && publication.Publication == "loaded" {
			status = "revoked"
		}
	}
	return NativeStatus{Status: status}, ""
}
func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}
func (s *Service) startHTTP(w http.ResponseWriter, r *http.Request) {
	var request NativeStart
	if !decodeJSON(w, r, &request) {
		return
	}
	result, code := s.start(r.Context(), request)
	if code != "" {
		WriteError(w, code)
		return
	}
	writeJSON(w, 201, result)
}
func (s *Service) pollHTTP(w http.ResponseWriter, r *http.Request) {
	var proof NativeProof
	if !decodeJSON(w, r, &proof) {
		return
	}
	defer func() { proof.Verifier = "" }()
	result, code := s.pollResult(r.Context(), proof)
	if code != "" {
		WriteError(w, code)
		return
	}
	writeJSON(w, 200, result)
}
func (s *Service) cancelHTTP(w http.ResponseWriter, r *http.Request) {
	var proof NativeProof
	if !decodeJSON(w, r, &proof) {
		return
	}
	defer func() { proof.Verifier = "" }()
	result, code := s.cancelResult(r.Context(), proof)
	if code != "" {
		WriteError(w, code)
		return
	}
	writeJSON(w, 200, result)
}
