package routerenroll

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
)

type NativePollResult struct {
	NativeStatus
	Certificate *IssuedCertificate `json:"certificate,omitempty"`
}

func nativeOperation(id string) string {
	value, err := base64.RawURLEncoding.Strict().DecodeString(id)
	if err != nil || len(value) != 32 {
		return ""
	}
	value[6] = (value[6] & 15) | 64
	value[8] = (value[8] & 63) | 128
	return fmt.Sprintf("%x-%x-%x-%x-%x", value[:4], value[4:6], value[6:8], value[8:10], value[10:16])
}

func (s *Service) pollResult(ctx context.Context, proof NativeProof) (NativePollResult, string) {
	state, code := s.poll(ctx, proof)
	if code != "" {
		return NativePollResult{}, code
	}
	if state.Status != "approved" && state.Status != "issued" {
		return NativePollResult{NativeStatus: state}, ""
	}
	v, err := readProvenNative(ctx, s.store.db, proof)
	if err != nil {
		return NativePollResult{}, publicCode(err)
	}
	var account Account
	var encoded string
	if s.store.db.QueryRowContext(ctx, "SELECT account_json FROM enrollment_transactions WHERE id=?", v.ID).Scan(&encoded) != nil || json.Unmarshal([]byte(encoded), &account) != nil || account.ID != v.UserID {
		return NativePollResult{}, ServiceUnavailable
	}
	request := authorizedIssue{OperationID: nativeOperation(v.ID), TransactionID: v.ID, Epoch: v.Epoch, Account: account, DeviceID: v.DeviceID, Kind: v.Kind, Label: v.Label, CSRPEM: v.CSRPEM, TransactionVersion: v.Version}
	if v.Status != "issued" {
		if v.Status != "approved" {
			return NativePollResult{}, OperationConflict
		}
		member, code := s.verifyMember(ctx, v.Epoch, v.UserID)
		if code != "" {
			return NativePollResult{}, code
		}
		if member.Account.Organization != account.Organization {
			return NativePollResult{}, OperationConflict
		}
		request.UserVersion, request.TokenVersion = member.UserVersion, member.TokenVersion
	}
	certificate, code := s.store.issueAuthorized(ctx, s.issuer, request, s.now())
	if code != "" {
		return NativePollResult{}, code
	}
	return NativePollResult{NativeStatus: NativeStatus{Status: "issued", Interval: int64(PollInterval.Seconds())}, Certificate: &certificate}, ""
}
