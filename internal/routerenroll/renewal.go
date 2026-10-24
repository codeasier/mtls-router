package routerenroll

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/sha256"
	"crypto/x509"
	"database/sql"
	"encoding/base64"
	"errors"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"time"
)

const (
	RenewPath                = "/enroll/renew"
	RenewRecoverPath         = "/enroll/renew/recover"
	GatewayVerifyHeader      = "X-Codeasier-Client-Verify"
	GatewayCertificateHeader = "X-Codeasier-Client-Cert"
)

var issuerIDPattern = regexp.MustCompile(`^[0-9a-f]{64}$`)

// RenewalRequest contains a public CSR and a short-lived possession signature,
// never a device private key. Signatures are not persisted or logged.
type RenewalRequest struct {
	Version     string `json:"version"`
	IssuerID    string `json:"issuer_id"`
	Serial      string `json:"serial"`
	OperationID string `json:"operation_id"`
	CSRPEM      string `json:"csr_pem"`
	Timestamp   int64  `json:"timestamp"`
	Signature   string `json:"signature"`
}

// RenewalSigningBytes is the exact language-neutral, domain-separated protocol.
// A recovery proof cannot authorize a new issuance, even on a different origin.
func RenewalSigningBytes(v RenewalRequest, recoverOnly bool) ([]byte, string) {
	binding, code := ParseCSR(v.CSRPEM)
	if code != "" || v.Version != ProtocolVersion || !issuerIDPattern.MatchString(v.IssuerID) || !serialPattern.MatchString(v.Serial) || !devicePattern.MatchString(v.OperationID) || v.Timestamp <= 0 {
		return nil, InvalidRequest
	}
	purpose, origin, path := "renew", RelayOrigin, RenewPath
	if recoverOnly {
		purpose, origin, path = "recover", EnrollOrigin, RenewRecoverPath
	}
	return []byte(strings.Join([]string{"codeasier-router-renew-v1", purpose, origin, path, v.Version, v.IssuerID, v.Serial, v.OperationID, binding.CSRDigest, strconv.FormatInt(v.Timestamp, 10), ""}, "\n")), ""
}

type renewalPredecessor struct {
	Certificate            IssuedCertificate
	Parsed                 *x509.Certificate
	Version, DeviceVersion int64
	RevocationReason       string
}

func readCertificate(ctx context.Context, reader policyReader, issuer, serial string) (IssuedCertificate, error) {
	var v IssuedCertificate
	v.Identity.Version = ProtocolVersion
	err := reader.QueryRowContext(ctx, `SELECT issuer_id,serial,certificate_pem,chain_pem,not_before,not_after,spki_digest,epoch,organization_id,user_id,device_id FROM certificates WHERE issuer_id=? AND serial=?`, issuer, serial).Scan(&v.IssuerID, &v.Serial, &v.CertificatePEM, &v.ChainPEM, &v.NotBefore, &v.NotAfter, &v.SPKIDigest, &v.Identity.PolicyEpoch, &v.Identity.OrganizationID, &v.Identity.UserID, &v.Identity.DeviceID)
	return v, err
}

func readPredecessor(ctx context.Context, reader policyReader, request RenewalRequest, recoverOnly bool, now time.Time) (renewalPredecessor, string) {
	var v renewalPredecessor
	cert, err := readCertificate(ctx, reader, request.IssuerID, request.Serial)
	if errors.Is(err, sql.ErrNoRows) {
		return v, ClientCertRejected
	}
	if err != nil {
		return v, ServiceUnavailable
	}
	v.Certificate = cert
	if certificateIssuerAllowed(ctx, reader, cert) != nil {
		return v, ClientCertRejected
	}
	policy, err := readPolicy(ctx, reader)
	if err != nil {
		return v, ServiceUnavailable
	}
	var disabled int
	var status string
	if reader.QueryRowContext(ctx, `SELECT c.version,d.version,d.disabled,u.verification_status,COALESCE(r.reason,'') FROM certificates c JOIN devices d ON d.id=c.device_id JOIN users u ON u.epoch=c.epoch AND u.user_id=c.user_id LEFT JOIN revocations r ON r.issuer_id=c.issuer_id AND r.serial=c.serial WHERE c.issuer_id=? AND c.serial=?`, request.IssuerID, request.Serial).Scan(&v.Version, &v.DeviceVersion, &disabled, &status, &v.RevocationReason) != nil {
		return v, ServiceUnavailable
	}
	if disabled != 0 || status == "revoked" || status == "nonmember" || policy.Epoch != cert.Identity.PolicyEpoch || policy.BoundID != cert.Identity.OrganizationID || (v.RevocationReason != "" && (!recoverOnly || v.RevocationReason != "handover")) {
		return v, ClientCertRejected
	}
	v.Parsed, err = parseSingleCertificate([]byte(cert.CertificatePEM))
	if err != nil || v.Parsed.SerialNumber.Text(16) != request.Serial || Digest(v.Parsed.RawSubjectPublicKeyInfo) != cert.SPKIDigest {
		return v, ServiceUnavailable
	}
	if now.Unix() < cert.NotBefore {
		return v, ClientCertRejected
	}
	if !recoverOnly && now.Unix() >= cert.NotAfter {
		return v, ClientCertExpired
	}
	return v, ""
}

func verifyRenewalProof(v RenewalRequest, cert *x509.Certificate, recoverOnly bool, now time.Time) string {
	message, code := RenewalSigningBytes(v, recoverOnly)
	if code != "" {
		return code
	}
	// Compare bounds rather than subtracting untrusted int64 values.
	if v.Timestamp < now.Add(-ClockSkew).Unix() || v.Timestamp > now.Add(ClockSkew).Unix() {
		return InvalidProof
	}
	if len(v.Signature) > 128 {
		return InvalidProof
	}
	signature, err := base64.RawURLEncoding.Strict().DecodeString(v.Signature)
	key, ok := cert.PublicKey.(*ecdsa.PublicKey)
	digest := sha256.Sum256(message)
	if err != nil || !ok || key.Curve != elliptic.P256() || !ecdsa.VerifyASN1(key, digest[:], signature) {
		return InvalidProof
	}
	return ""
}

func recoverRenewal(ctx context.Context, reader policyReader, request RenewalRequest, now time.Time) (IssuedCertificate, string) {
	old, code := readPredecessor(ctx, reader, request, true, now)
	if code != "" {
		return IssuedCertificate{}, code
	}
	binding, code := ParseCSR(request.CSRPEM)
	if code != "" {
		return IssuedCertificate{}, code
	}
	var issuer, serial, digest, spki, requestID string
	var until int64
	err := reader.QueryRowContext(ctx, `SELECT issuer_id,predecessor_serial,csr_digest,spki_digest,request_id,handover_until FROM renewal_operations WHERE id=?`, request.OperationID).Scan(&issuer, &serial, &digest, &spki, &requestID, &until)
	if errors.Is(err, sql.ErrNoRows) {
		return IssuedCertificate{}, OperationConflict
	}
	if err != nil {
		return IssuedCertificate{}, ServiceUnavailable
	}
	if issuer != request.IssuerID || serial != request.Serial || digest != binding.CSRDigest || spki != binding.SPKIDigest {
		return IssuedCertificate{}, OperationConflict
	}
	if now.Unix() >= until {
		return IssuedCertificate{}, ReauthRequired
	}
	var result IssuedCertificate
	result.Identity.Version = ProtocolVersion
	var revoked int
	err = reader.QueryRowContext(ctx, `SELECT c.issuer_id,c.serial,c.certificate_pem,c.chain_pem,c.not_before,c.not_after,c.spki_digest,c.epoch,c.organization_id,c.user_id,c.device_id,EXISTS(SELECT 1 FROM revocations r WHERE r.issuer_id=c.issuer_id AND r.serial=c.serial) FROM certificates c WHERE c.request_id=?`, requestID).Scan(&result.IssuerID, &result.Serial, &result.CertificatePEM, &result.ChainPEM, &result.NotBefore, &result.NotAfter, &result.SPKIDigest, &result.Identity.PolicyEpoch, &result.Identity.OrganizationID, &result.Identity.UserID, &result.Identity.DeviceID, &revoked)
	if err != nil {
		return IssuedCertificate{}, ServiceUnavailable
	}
	if revoked != 0 || certificateIssuerAllowed(ctx, reader, result) != nil || result.Identity != old.Certificate.Identity || result.SPKIDigest != binding.SPKIDigest || now.Unix() >= result.NotAfter {
		return IssuedCertificate{}, ClientCertRejected
	}
	return result, ""
}

// GatewayHandler must ONLY be mounted on the separate protected Unix socket.
// The public Handler has no signing route and never trusts client identity heads.
func (s *Service) GatewayHandler() http.Handler {
	return s.boundedControl(newHandler(nil, map[string]http.HandlerFunc{RenewPath: s.renewHTTP}))
}
func singleHeader(r *http.Request, name string) string {
	values := r.Header.Values(name)
	if len(values) != 1 {
		return ""
	}
	return values[0]
}
func (s *Service) renewHTTP(w http.ResponseWriter, r *http.Request) {
	var request RenewalRequest
	if !decodeJSON(w, r, &request) {
		return
	}
	if _, code := RenewalSigningBytes(request, false); code != "" {
		WriteError(w, code)
		return
	}
	if singleHeader(r, GatewayVerifyHeader) != "SUCCESS" {
		WriteError(w, ClientCertRejected)
		return
	}
	encoded := singleHeader(r, GatewayCertificateHeader)
	if len(encoded) == 0 || len(encoded) > MaxCertificateBytes*3 {
		WriteError(w, ClientCertRejected)
		return
	}
	value, err := url.PathUnescape(encoded)
	cert, parseErr := parseSingleCertificate([]byte(value))
	if err != nil || parseErr != nil {
		WriteError(w, ClientCertRejected)
		return
	}
	result, code := s.renew(r.Context(), request, cert)
	if code != "" {
		WriteError(w, code)
		return
	}
	writeJSON(w, http.StatusOK, result)
}

func (s *Service) recoverRenewalHTTP(w http.ResponseWriter, r *http.Request) {
	var request RenewalRequest
	if !decodeJSON(w, r, &request) {
		return
	}
	result, code := s.recoverRenewal(r.Context(), request)
	if code != "" {
		WriteError(w, code)
		return
	}
	writeJSON(w, http.StatusOK, result)
}

func (s *Service) recoverRenewal(ctx context.Context, request RenewalRequest) (IssuedCertificate, string) {
	if _, code := RenewalSigningBytes(request, true); code != "" {
		return IssuedCertificate{}, code
	}
	var result IssuedCertificate
	if !s.store.Ready(ctx) {
		return result, ServiceUnavailable
	}
	// A read transaction gives one coherent snapshot across policy, revocation
	// and operation checks. It neither appends a journal record nor signs a leaf.
	tx, err := s.store.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return result, ServiceUnavailable
	}
	defer tx.Rollback()
	old, code := readPredecessor(ctx, tx, request, true, s.now())
	if code != "" {
		return result, code
	}
	if code = verifyRenewalProof(request, old.Parsed, true, s.now()); code != "" {
		return result, code
	}
	result, code = recoverRenewal(ctx, tx, request, s.now())
	return result, code
}

func (s *Service) renew(ctx context.Context, request RenewalRequest, verified *x509.Certificate) (IssuedCertificate, string) {
	if _, code := RenewalSigningBytes(request, false); code != "" {
		return IssuedCertificate{}, code
	}
	old, code := readPredecessor(ctx, s.store.db, request, false, s.now())
	if code != "" {
		return IssuedCertificate{}, code
	}
	if verified == nil || !bytes.Equal(verified.Raw, old.Parsed.Raw) {
		return IssuedCertificate{}, ClientCertRejected
	}
	if code = verifyRenewalProof(request, old.Parsed, false, s.now()); code != "" {
		return IssuedCertificate{}, code
	}
	// Idempotent mTLS retries are reads, also when GitCode is temporarily down.
	if result, code := recoverRenewal(ctx, s.store.db, request, s.now()); code != OperationConflict {
		return result, code
	}
	if old.Certificate.NotAfter > s.now().Add(RenewBefore).Unix() {
		return IssuedCertificate{}, OperationConflict
	}
	member, code := s.verifyMember(ctx, old.Certificate.Identity.PolicyEpoch, old.Certificate.Identity.UserID)
	if code != "" {
		return IssuedCertificate{}, code
	}
	var result IssuedCertificate
	err := s.store.transaction(ctx, func(tx *writeTx) error {
		current, code := readPredecessor(ctx, tx, request, false, s.now())
		if code != "" {
			return errors.New(code)
		}
		if current.Version != old.Version || current.DeviceVersion != old.DeviceVersion {
			return errors.New(OperationConflict)
		}
		if existing, code := recoverRenewal(ctx, tx, request, s.now()); code != OperationConflict {
			result = existing
			if code != "" {
				return errors.New(code)
			}
			return nil
		}
		var uv, tv int64
		if tx.QueryRowContext(ctx, `SELECT u.version,t.version FROM users u JOIN gitcode_tokens t ON t.epoch=u.epoch AND t.user_id=u.user_id WHERE u.epoch=? AND u.user_id=?`, member.Policy.Epoch, member.Account.ID).Scan(&uv, &tv) != nil {
			return ErrStorage
		}
		if uv != member.UserVersion || tv != member.TokenVersion || member.Status != "active" {
			return errors.New(OperationConflict)
		}
		var count int
		if tx.QueryRowContext(ctx, `SELECT count(*) FROM renewal_operations WHERE (issuer_id=? AND predecessor_serial=?) OR id=?`, request.IssuerID, request.Serial, request.OperationID).Scan(&count) != nil {
			return ErrStorage
		}
		if count != 0 {
			return errors.New(OperationConflict)
		}
		binding, code := ParseCSR(request.CSRPEM)
		if code != "" {
			return errors.New(code)
		}
		if binding.SPKIDigest == old.Certificate.SPKIDigest {
			return errors.New(OperationConflict)
		}
		var exists int
		if tx.QueryRowContext(ctx, "SELECT count(*) FROM issuance_requests WHERE id=?", request.OperationID).Scan(&exists) != nil {
			return ErrStorage
		}
		if exists != 0 {
			return errors.New(OperationConflict)
		}
		now := s.now()
		var err error
		if err := issuerAllowed(ctx, tx, s.issuer.cert); err != nil {
			return err
		}
		result, err = s.issuer.Issue(request.CSRPEM, current.Certificate.Identity, now)
		if err != nil {
			return ErrStorage
		}
		if result.NotAfter <= current.Certificate.NotAfter {
			return errors.New(ReauthRequired)
		}
		if _, err = tx.ExecContext(ctx, `INSERT INTO issuance_requests(id,device_id,csr_digest,spki_digest,committed_at,recovery_until) VALUES(?,?,?,?,?,?)`, request.OperationID, current.Certificate.Identity.DeviceID, binding.CSRDigest, binding.SPKIDigest, now.Unix(), now.Add(RecoveryWindow).Unix()); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `INSERT INTO certificates(issuer_id,serial,request_id,device_id,epoch,organization_id,user_id,csr_digest,spki_digest,certificate_pem,chain_pem,not_before,not_after) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)`, result.IssuerID, result.Serial, request.OperationID, result.Identity.DeviceID, result.Identity.PolicyEpoch, result.Identity.OrganizationID, result.Identity.UserID, binding.CSRDigest, binding.SPKIDigest, result.CertificatePEM, result.ChainPEM, result.NotBefore, result.NotAfter); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `INSERT INTO renewal_operations(id,issuer_id,predecessor_serial,csr_digest,spki_digest,request_id,handover_until) VALUES(?,?,?,?,?,?,?)`, request.OperationID, request.IssuerID, request.Serial, binding.CSRDigest, binding.SPKIDigest, request.OperationID, now.Add(RecoveryWindow).Unix()); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, `INSERT INTO audit_events(created_at,action,epoch,user_id,device_id,serial) VALUES(?,'renew',?,?,?,?)`, now.Unix(), result.Identity.PolicyEpoch, result.Identity.UserID, result.Identity.DeviceID, result.Serial)
		return err
	})
	if err != nil {
		return IssuedCertificate{}, publicCode(err)
	}
	return result, ""
}

func (s *Service) retirePredecessors(ctx context.Context) error {
	return s.store.transaction(ctx, func(tx *writeTx) error {
		var count int
		if tx.QueryRowContext(ctx, `SELECT count(*) FROM renewal_operations o WHERE handover_until<=? AND NOT EXISTS(SELECT 1 FROM revocations r WHERE r.issuer_id=o.issuer_id AND r.serial=o.predecessor_serial)`, s.now().Unix()).Scan(&count) != nil {
			return ErrStorage
		}
		if count == 0 {
			return nil
		}
		_, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,predecessor_serial,?,'handover' FROM renewal_operations WHERE handover_until<=?`, s.now().Unix(), s.now().Unix())
		return err
	})
}
