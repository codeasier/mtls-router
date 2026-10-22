package routerenroll

import (
	"context"
	"database/sql"
	"errors"
	"time"
)

type authorizedIssue struct {
	OperationID                                   string
	TransactionID                                 string
	Epoch                                         int64
	Account                                       Account
	DeviceID, Kind, Label, CSRPEM                 string
	UserVersion, TokenVersion, TransactionVersion int64
}

// issueAuthorized is private to the control plane. Its caller must provide a
// freshly provider-verified account, never browser/native account claims. The
// durable active policy, account/device binding and operation are rechecked
// under the write transaction before signing or returning a committed result.
func (s *Store) issueAuthorized(ctx context.Context, issuer *Issuer, request authorizedIssue, now time.Time) (IssuedCertificate, string) {
	binding, code := ParseCSR(request.CSRPEM)
	if code != "" || !devicePattern.MatchString(request.OperationID) || !devicePattern.MatchString(request.DeviceID) || (request.Kind != "desktop" && request.Kind != "linux_cli") {
		return IssuedCertificate{}, InvalidRequest
	}
	var result IssuedCertificate
	failure := ServiceUnavailable
	err := s.transaction(ctx, func(tx *writeTx) error {
		if err := issuerAllowed(ctx, tx, issuer.cert); err != nil {
			return err
		}
		policy, err := readPolicy(ctx, tx)
		if err != nil {
			return ErrStorage
		}
		if policy.Epoch != request.Epoch || policy.BoundID == "" || request.Account.Organization.ID != policy.BoundID || request.Account.Organization.Path != policy.Path {
			failure = OperationConflict
			return ErrStorage
		}
		// The token/account must have been saved by the trusted OAuth flow.
		var userVersion int64
		var userStatus string
		if tx.QueryRowContext(ctx, "SELECT version,verification_status FROM users WHERE epoch=? AND user_id=?", request.Epoch, request.Account.ID).Scan(&userVersion, &userStatus) != nil {
			return ErrStorage
		}
		if userStatus == "revoked" || userStatus == "nonmember" {
			failure = ClientCertRejected
			return ErrStorage
		}
		if request.UserVersion > 0 && userVersion != request.UserVersion {
			failure = OperationConflict
			return ErrStorage
		}
		if request.TokenVersion > 0 {
			var version int64
			if tx.QueryRowContext(ctx, "SELECT version FROM gitcode_tokens WHERE epoch=? AND user_id=?", request.Epoch, request.Account.ID).Scan(&version) != nil || version != request.TokenVersion {
				failure = OperationConflict
				return ErrStorage
			}
		}
		var userID string
		var epoch int64
		var disabled int
		err = tx.QueryRowContext(ctx, "SELECT epoch,user_id,disabled FROM devices WHERE id=?", request.DeviceID).Scan(&epoch, &userID, &disabled)
		if err == nil && (epoch != request.Epoch || userID != request.Account.ID || disabled != 0) {
			failure = ClientCertRejected
			return ErrStorage
		}
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return ErrStorage
		}
		if errors.Is(err, sql.ErrNoRows) {
			if _, err := tx.ExecContext(ctx, "INSERT INTO devices(id,epoch,user_id,kind,label) VALUES(?,?,?,?,?)", request.DeviceID, request.Epoch, request.Account.ID, request.Kind, request.Label); err != nil {
				return ErrStorage
			}
		}
		var existingDigest, existingSPKI, existingDevice string
		var recoverUntil int64
		err = tx.QueryRowContext(ctx, "SELECT csr_digest,spki_digest,device_id,recovery_until FROM issuance_requests WHERE id=?", request.OperationID).Scan(&existingDigest, &existingSPKI, &existingDevice, &recoverUntil)
		if err == nil {
			if existingDigest != binding.CSRDigest || existingSPKI != binding.SPKIDigest || existingDevice != request.DeviceID {
				failure = OperationConflict
				return ErrStorage
			}
			if now.Unix() >= recoverUntil {
				failure = ReauthRequired
				return ErrStorage
			}
			var revoked int
			result.Identity.Version = ProtocolVersion
			err = tx.QueryRowContext(ctx, `SELECT c.issuer_id,c.serial,c.certificate_pem,c.chain_pem,c.not_before,c.not_after,c.spki_digest,c.epoch,c.organization_id,c.user_id,c.device_id,EXISTS(SELECT 1 FROM revocations r WHERE r.issuer_id=c.issuer_id AND r.serial=c.serial) FROM certificates c WHERE c.request_id=?`, request.OperationID).Scan(&result.IssuerID, &result.Serial, &result.CertificatePEM, &result.ChainPEM, &result.NotBefore, &result.NotAfter, &result.SPKIDigest, &result.Identity.PolicyEpoch, &result.Identity.OrganizationID, &result.Identity.UserID, &result.Identity.DeviceID, &revoked)
			if err != nil {
				return ErrStorage
			}
			if revoked != 0 {
				failure = ClientCertRejected
				return ErrStorage
			}
			if certificateIssuerAllowed(ctx, tx, result) != nil {
				failure = ClientCertRejected
				return ErrStorage
			}
			return nil
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return ErrStorage
		}
		if request.TransactionID != "" {
			var status, device, user, digest string
			var epoch, expires, version int64
			err := tx.QueryRowContext(ctx, "SELECT status,epoch,device_id,user_id,csr_digest,expires_at,version FROM enrollment_transactions WHERE id=?", request.TransactionID).Scan(&status, &epoch, &device, &user, &digest, &expires, &version)
			if err != nil || status != "approved" || epoch != request.Epoch || device != request.DeviceID || user != request.Account.ID || digest != binding.CSRDigest || now.Unix() >= expires || (request.TransactionVersion > 0 && request.TransactionVersion != version) {
				failure = OperationConflict
				return ErrStorage
			}
		}
		identity := Identity{ProtocolVersion, policy.BoundID, request.Epoch, request.Account.ID, request.DeviceID}
		result, err = issuer.Issue(request.CSRPEM, identity, now)
		if err != nil {
			return ErrStorage
		}
		var transaction any
		if request.TransactionID != "" {
			transaction = request.TransactionID
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO issuance_requests(id,transaction_id,device_id,csr_digest,spki_digest,committed_at,recovery_until) VALUES(?,?,?,?,?,?,?)`, request.OperationID, transaction, request.DeviceID, binding.CSRDigest, binding.SPKIDigest, now.Unix(), now.Add(RecoveryWindow).Unix()); err != nil {
			return ErrStorage
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO certificates(issuer_id,serial,request_id,device_id,epoch,organization_id,user_id,csr_digest,spki_digest,certificate_pem,chain_pem,not_before,not_after) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)`, result.IssuerID, result.Serial, request.OperationID, request.DeviceID, request.Epoch, policy.BoundID, request.Account.ID, binding.CSRDigest, binding.SPKIDigest, result.CertificatePEM, result.ChainPEM, result.NotBefore, result.NotAfter); err != nil {
			return ErrStorage
		}
		if request.TransactionID != "" {
			if _, err := tx.ExecContext(ctx, "UPDATE enrollment_transactions SET status='issued',version=version+1 WHERE id=?", request.TransactionID); err != nil {
				return ErrStorage
			}
		}
		_, err = tx.ExecContext(ctx, "INSERT INTO audit_events(created_at,action,epoch,user_id,device_id,serial) VALUES(?,'issue',?,?,?,?)", now.Unix(), request.Epoch, request.Account.ID, request.DeviceID, result.Serial)
		return err
	})
	if err != nil {
		return IssuedCertificate{}, failure
	}
	return result, ""
}

// CertificateSummary is intentionally public metadata only, never PEM, tokens,
// paths, private labels or a claim that revocation has reached all gateways.
type CertificateSummary struct {
	IssuerID          string `json:"issuer_id"`
	Serial            string `json:"serial"`
	UserID            string `json:"user_id"`
	DeviceID          string `json:"device_id"`
	ExpiresAt         int64  `json:"expires_at"`
	RevocationDecided bool   `json:"revocation_decided"`
}
type RevocationTarget struct{ Kind, ID string }
type RevocationResult struct {
	Matched     int64  `json:"matched"`
	Publication string `json:"publication"`
}

func listCertificates(ctx context.Context, db *sql.DB, limit int) ([]CertificateSummary, error) {
	if limit < 1 || limit > 100 {
		return nil, ErrConfig
	}
	rows, err := db.QueryContext(ctx, `SELECT c.issuer_id,c.serial,c.user_id,c.device_id,c.not_after,EXISTS(SELECT 1 FROM revocations r WHERE r.issuer_id=c.issuer_id AND r.serial=c.serial) FROM certificates c ORDER BY c.issuer_id,c.serial LIMIT ?`, limit)
	if err != nil {
		return nil, ErrStorage
	}
	defer rows.Close()
	result := make([]CertificateSummary, 0)
	for rows.Next() {
		var value CertificateSummary
		if rows.Scan(&value.IssuerID, &value.Serial, &value.UserID, &value.DeviceID, &value.ExpiresAt, &value.RevocationDecided) != nil {
			return nil, ErrStorage
		}
		result = append(result, value)
	}
	if rows.Err() != nil {
		return nil, ErrStorage
	}
	return result, nil
}

type journalWriter interface {
	transaction(context.Context, func(*writeTx) error) error
}

func revoke(ctx context.Context, writer journalWriter, target RevocationTarget, now time.Time) (RevocationResult, error) {
	column := ""
	switch target.Kind {
	case "serial":
		column = "serial"
		if !serialPattern.MatchString(target.ID) {
			return RevocationResult{}, ErrConfig
		}
	case "device":
		column = "device_id"
		if !devicePattern.MatchString(target.ID) {
			return RevocationResult{}, ErrConfig
		}
	case "user":
		column = "user_id"
		if !stableIDPattern.MatchString(target.ID) {
			return RevocationResult{}, ErrConfig
		}
	default:
		return RevocationResult{}, ErrConfig
	}
	var matched int64
	err := writer.transaction(ctx, func(tx *writeTx) error {
		if tx.QueryRowContext(ctx, "SELECT count(*) FROM certificates WHERE "+column+"=?", target.ID).Scan(&matched) != nil {
			return ErrStorage
		}
		if matched == 0 {
			return ErrConfig
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,serial,?,'admin' FROM certificates WHERE `+column+"=? ON CONFLICT(issuer_id,serial) DO UPDATE SET reason='admin',version=revocations.version+1", now.Unix(), target.ID); err != nil {
			return ErrStorage
		}
		if target.Kind != "serial" {
			deviceColumn := "id"
			if target.Kind == "user" {
				deviceColumn = "user_id"
			}
			if _, err := tx.ExecContext(ctx, "UPDATE devices SET disabled=1,version=version+1 WHERE "+deviceColumn+"=?", target.ID); err != nil {
				return ErrStorage
			}
		}
		if target.Kind == "user" {
			if _, err := tx.ExecContext(ctx, "UPDATE users SET version=version+1,verification_status='revoked' WHERE user_id=?", target.ID); err != nil {
				return ErrStorage
			}
		}
		// Only closed action names and stable IDs are audited; never the raw argv.
		user, device, serial := "", "", ""
		switch target.Kind {
		case "user":
			user = target.ID
		case "device":
			device = target.ID
		case "serial":
			serial = target.ID
		}
		if _, err := tx.ExecContext(ctx, "INSERT INTO audit_events(created_at,action,epoch,user_id,device_id,serial) VALUES(?,?,0,?,?,?)", now.Unix(), "revoke_"+target.Kind, user, device, serial); err != nil {
			return ErrStorage
		}
		return nil
	})
	if err != nil {
		return RevocationResult{}, err
	}
	return RevocationResult{matched, "pending"}, nil
}
