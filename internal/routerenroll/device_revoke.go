package routerenroll

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"net/http"
	"strconv"
	"strings"
)

const DeviceRevokePath = "/enroll/device/revoke"

type DeviceRevokeRequest struct {
	Version   string `json:"version"`
	IssuerID  string `json:"issuer_id"`
	Serial    string `json:"serial"`
	Timestamp int64  `json:"timestamp"`
	Signature string `json:"signature"`
}
type DeviceRevokeResult struct {
	Status string `json:"status"`
}

func DeviceRevokeSigningBytes(v DeviceRevokeRequest) ([]byte, string) {
	if v.Version != ProtocolVersion || !issuerIDPattern.MatchString(v.IssuerID) || !serialPattern.MatchString(v.Serial) || v.Timestamp <= 0 {
		return nil, InvalidRequest
	}
	return []byte(strings.Join([]string{"codeasier-router-revoke-v1", EnrollOrigin, DeviceRevokePath, v.Version, v.IssuerID, v.Serial, strconv.FormatInt(v.Timestamp, 10), ""}, "\n")), ""
}

func (s *Service) revokeDevice(ctx context.Context, v DeviceRevokeRequest) (DeviceRevokeResult, string) {
	data, code := DeviceRevokeSigningBytes(v)
	if code != "" {
		return DeviceRevokeResult{}, code
	}
	now := s.now()
	if v.Timestamp < now.Add(-ClockSkew).Unix() || v.Timestamp > now.Add(ClockSkew).Unix() || len(v.Signature) > 128 {
		return DeviceRevokeResult{}, InvalidProof
	}
	if !s.store.Ready(ctx) {
		return DeviceRevokeResult{}, ServiceUnavailable
	}
	cert, err := readCertificate(ctx, s.store.db, v.IssuerID, v.Serial)
	if err != nil {
		return DeviceRevokeResult{}, ClientCertRejected
	}
	parsed, err := parseSingleCertificate([]byte(cert.CertificatePEM))
	if err != nil {
		return DeviceRevokeResult{}, ServiceUnavailable
	}
	key, ok := parsed.PublicKey.(*ecdsa.PublicKey)
	signature, err := base64.RawURLEncoding.Strict().DecodeString(v.Signature)
	digest := sha256.Sum256(data)
	if err != nil || !ok || key.Curve != elliptic.P256() || !ecdsa.VerifyASN1(key, digest[:], signature) {
		return DeviceRevokeResult{}, InvalidProof
	}
	err = s.store.transaction(ctx, func(tx *writeTx) error {
		var disabled int
		var epoch int64
		var user, reason string
		if tx.QueryRowContext(ctx, `SELECT d.disabled,d.epoch,d.user_id,COALESCE(r.reason,'') FROM devices d LEFT JOIN revocations r ON r.issuer_id=? AND r.serial=? WHERE d.id=?`, v.IssuerID, v.Serial, cert.Identity.DeviceID).Scan(&disabled, &epoch, &user, &reason) != nil {
			return ErrStorage
		}
		if epoch != cert.Identity.PolicyEpoch || user != cert.Identity.UserID {
			return errors.New(ClientCertRejected)
		}
		if disabled != 0 {
			return nil
		}
		if reason != "" {
			return errors.New(ClientCertRejected)
		}
		var retired int
		if tx.QueryRowContext(ctx, `SELECT count(*) FROM renewal_operations WHERE issuer_id=? AND predecessor_serial=? AND handover_until<=?`, v.IssuerID, v.Serial, s.now().Unix()).Scan(&retired) != nil {
			return ErrStorage
		}
		if retired != 0 {
			return errors.New(ReauthRequired)
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,serial,?,'device_logout' FROM certificates WHERE device_id=? ON CONFLICT(issuer_id,serial) DO UPDATE SET reason='device_logout',version=revocations.version+1`, s.now().Unix(), cert.Identity.DeviceID); err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, "UPDATE devices SET disabled=1,version=version+1 WHERE id=?", cert.Identity.DeviceID); err != nil {
			return err
		}
		_, err := tx.ExecContext(ctx, `INSERT INTO audit_events(created_at,action,epoch,user_id,device_id) VALUES(?,'device_logout',?,?,?)`, s.now().Unix(), epoch, user, cert.Identity.DeviceID)
		return err
	})
	if err != nil {
		return DeviceRevokeResult{}, publicCode(err)
	}
	result := DeviceRevokeResult{Status: "revocation_pending"}
	if s.publisher != nil {
		if status, err := s.publisher.Status(ctx, s.now()); err == nil && status.Publication == "loaded" {
			result.Status = "revoked"
		}
	}
	return result, ""
}

func (s *Service) revokeDeviceHTTP(w http.ResponseWriter, r *http.Request) {
	var request DeviceRevokeRequest
	if !decodeJSON(w, r, &request) {
		return
	}
	result, code := s.revokeDevice(r.Context(), request)
	if code != "" {
		WriteError(w, code)
		return
	}
	writeJSON(w, http.StatusOK, result)
}
