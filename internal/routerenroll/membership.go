package routerenroll

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"time"
)

const MembershipInterval = 300 * time.Second

type memberSnapshot struct {
	Policy                    Policy
	Account                   Account
	UserVersion, TokenVersion int64
	Token                     GitCodeToken
	Status                    string
}
type memberCheck struct {
	done  chan struct{}
	value memberSnapshot
	code  string
}

func (s *Service) memberVersion(ctx context.Context, epoch int64, user string) (int64, error) {
	var version int64
	err := s.store.db.QueryRowContext(ctx, "SELECT version FROM users WHERE epoch=? AND user_id=?", epoch, user).Scan(&version)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, nil
	}
	return version, err
}

// Browser checks also revoke confirmed former members, but never apply a
// network response over a newer token/account decision or organization epoch.
func (s *Service) recordMembershipLoss(ctx context.Context, epoch int64, user string, checkedVersion int64) error {
	if checkedVersion == 0 {
		return nil
	}
	return s.store.transaction(ctx, func(tx *writeTx) error {
		policy, err := readPolicy(ctx, tx)
		if err != nil {
			return err
		}
		if policy.Epoch != epoch {
			return nil
		}
		var version int64
		if tx.QueryRowContext(ctx, "SELECT version FROM users WHERE epoch=? AND user_id=?", epoch, user).Scan(&version) != nil {
			return ErrStorage
		}
		if version != checkedVersion {
			return nil
		}
		if _, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,serial,?,'membership_lost' FROM certificates WHERE epoch=? AND user_id=?`, s.now().Unix(), epoch, user); err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx, "UPDATE devices SET disabled=1,version=version+1 WHERE epoch=? AND user_id=?", epoch, user); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, "UPDATE users SET verification_status='nonmember',version=version+1,verification_attempted_at=? WHERE epoch=? AND user_id=?", s.now().Unix(), epoch, user)
		return err
	})
}

func (s *Service) readMember(ctx context.Context, epoch int64, user string) (memberSnapshot, string) {
	var v memberSnapshot
	var ciphertext []byte
	var err error
	v.Policy, err = readPolicy(ctx, s.store.db)
	if err != nil {
		return v, ServiceUnavailable
	}
	if v.Policy.Epoch != epoch {
		return v, OperationConflict
	}
	v.Account.ID = user
	v.Account.Organization = Organization{v.Policy.BoundID, v.Policy.Path}
	err = s.store.db.QueryRowContext(ctx, `SELECT u.username,u.version,u.verification_status,t.version,t.ciphertext FROM users u JOIN gitcode_tokens t ON t.epoch=u.epoch AND t.user_id=u.user_id WHERE u.epoch=? AND u.user_id=?`, epoch, user).Scan(&v.Account.Username, &v.UserVersion, &v.Status, &v.TokenVersion, &ciphertext)
	if err != nil || s.store.decrypt(ciphertext, tokenAAD(epoch, user, v.TokenVersion), &v.Token) != nil {
		return v, ServiceUnavailable
	}
	if v.Status == "revoked" || v.Status == "nonmember" {
		return v, ClientCertRejected
	}
	return v, ""
}

// verifyMember coalesces only overlapping requests, never caches a previous
// successful decision as a replacement for a new signing-time verification.
func (s *Service) verifyMember(ctx context.Context, epoch int64, user string) (memberSnapshot, string) {
	key := fmt.Sprintf("%d:%s", epoch, user)
	s.checksMu.Lock()
	if pending := s.checks[key]; pending != nil {
		s.checksMu.Unlock()
		select {
		case <-ctx.Done():
			return memberSnapshot{}, ServiceUnavailable
		case <-pending.done:
			return pending.value, pending.code
		}
	}
	if s.checks == nil {
		s.checks = make(map[string]*memberCheck)
	}
	pending := &memberCheck{done: make(chan struct{})}
	s.checks[key] = pending
	s.checksMu.Unlock()
	pending.value, pending.code = s.checkMember(ctx, epoch, user)
	s.checksMu.Lock()
	delete(s.checks, key)
	close(pending.done)
	s.checksMu.Unlock()
	return pending.value, pending.code
}

func (s *Service) checkMember(ctx context.Context, epoch int64, user string) (memberSnapshot, string) {
	v, code := s.readMember(ctx, epoch, user)
	if code != "" {
		return v, code
	}
	now := s.now()
	refreshed := false
	refresh := func() error {
		token, err := s.provider.Refresh(ctx, v.Token)
		if err != nil {
			return err
		}
		v.Token, refreshed = token, true
		account, err := s.provider.User(ctx, token.AccessToken)
		if err != nil {
			return err
		}
		if account.ID != user {
			return providerError("refresh_user_mismatch")
		}
		return nil
	}
	var failure error
	if v.Token.ExpiresAt <= now.Add(time.Minute).Unix() {
		failure = refresh()
	}
	var decision membershipDecision
	if failure == nil {
		decision, failure = s.provider.Membership(ctx, v.Token.AccessToken, v.Account, v.Policy)
		if needsReauthorization(failure) && !refreshed {
			failure = refresh()
			if failure == nil {
				decision, failure = s.provider.Membership(ctx, v.Token.AccessToken, v.Account, v.Policy)
			}
		}
	}
	status := "active"
	code = decision.Code
	if failure != nil || code == ServiceUnavailable {
		status, code = "unavailable", ServiceUnavailable
	}
	if needsReauthorization(failure) {
		status, code = "reauth", ReauthRequired
	}
	if failure == nil && decision.Code == MembershipRequired {
		status, code = "nonmember", MembershipRequired
	}
	if failure == nil && code == "" && decision.Organization != v.Account.Organization {
		status, code = "unavailable", ServiceUnavailable
	}
	err := s.store.transaction(ctx, func(tx *writeTx) error {
		p, err := readPolicy(ctx, tx)
		if err != nil {
			return ErrStorage
		}
		var uv, tv int64
		if p.Epoch != epoch || p.BoundID != v.Policy.BoundID || tx.QueryRowContext(ctx, `SELECT u.version,t.version FROM users u JOIN gitcode_tokens t ON t.epoch=u.epoch AND t.user_id=u.user_id WHERE u.epoch=? AND u.user_id=?`, epoch, user).Scan(&uv, &tv) != nil || uv != v.UserVersion || tv != v.TokenVersion {
			return errors.New(OperationConflict)
		}
		if refreshed {
			encrypted, err := s.store.encrypt(v.Token, tokenAAD(epoch, user, tv+1))
			if err != nil {
				return err
			}
			if _, err = tx.ExecContext(ctx, "UPDATE gitcode_tokens SET version=version+1,ciphertext=? WHERE epoch=? AND user_id=?", encrypted, epoch, user); err != nil {
				return err
			}
			v.TokenVersion++
		}
		if _, err := tx.ExecContext(ctx, `UPDATE users SET version=version+1,verification_status=?,verified_at=CASE WHEN ?='active' THEN ? ELSE verified_at END,verification_attempted_at=? WHERE epoch=? AND user_id=?`, status, status, now.Unix(), now.Unix(), epoch, user); err != nil {
			return err
		}
		if status == "nonmember" {
			if _, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,serial,?,'membership_lost' FROM certificates WHERE epoch=? AND user_id=?`, now.Unix(), epoch, user); err != nil {
				return err
			}
			if _, err := tx.ExecContext(ctx, "UPDATE devices SET disabled=1,version=version+1 WHERE epoch=? AND user_id=?", epoch, user); err != nil {
				return err
			}
		}
		v.UserVersion++
		v.Status = status
		return nil
	})
	if err != nil {
		return memberSnapshot{}, publicCode(err)
	}
	return v, code
}

// RecheckMembers scans only users with live certificates. Network operations
// hold no database transaction; each result is committed with user/token CAS.
func (s *Service) RecheckMembers(ctx context.Context) error {
	var after string
	for {
		rows, err := s.store.db.QueryContext(ctx, `SELECT u.epoch,u.user_id FROM users u JOIN organization_policy p ON p.epoch=u.epoch WHERE u.user_id>? AND u.verification_status NOT IN ('revoked','nonmember') AND u.verification_attempted_at<=? AND EXISTS(SELECT 1 FROM certificates c WHERE c.epoch=u.epoch AND c.user_id=u.user_id AND c.not_after>? AND NOT EXISTS(SELECT 1 FROM revocations r WHERE r.issuer_id=c.issuer_id AND r.serial=c.serial)) ORDER BY u.user_id LIMIT 100`, after, s.now().Add(-MembershipInterval).Unix(), s.now().Unix())
		if err != nil {
			return ErrStorage
		}
		type target struct {
			epoch int64
			user  string
		}
		var targets []target
		for rows.Next() {
			var v target
			if rows.Scan(&v.epoch, &v.user) != nil {
				_ = rows.Close()
				return ErrStorage
			}
			targets = append(targets, v)
		}
		err = rows.Err()
		_ = rows.Close()
		if err != nil {
			return ErrStorage
		}
		for _, v := range targets {
			if ctx.Err() != nil {
				return ctx.Err()
			}
			_, _ = s.verifyMember(ctx, v.epoch, v.user)
			after = v.user
		}
		if len(targets) < 100 {
			return nil
		}
	}
}

// RunMaintenance stops with the service; it is not on the model data path.
func (s *Service) RunMaintenance(ctx context.Context, logger *slog.Logger) {
	tick := time.NewTicker(30 * time.Second)
	defer tick.Stop()
	for {
		if err := s.retirePredecessors(ctx); err != nil && ctx.Err() == nil {
			logger.Error("enroll_handover_unavailable")
		}
		if err := s.RecheckMembers(ctx); err != nil && ctx.Err() == nil {
			logger.Warn("enroll_membership_scan_unavailable")
		}
		var overdue int
		if s.store.db.QueryRowContext(ctx, `SELECT count(*) FROM users u WHERE verification_status NOT IN ('revoked','nonmember') AND verified_at<? AND EXISTS(SELECT 1 FROM certificates c WHERE c.epoch=u.epoch AND c.user_id=u.user_id AND c.not_after>? AND NOT EXISTS(SELECT 1 FROM revocations r WHERE r.issuer_id=c.issuer_id AND r.serial=c.serial))`, s.now().Add(-MembershipInterval).Unix(), s.now().Unix()).Scan(&overdue) == nil && overdue > 0 {
			logger.Warn("enroll_membership_overdue", "users", overdue)
		}
		if s.publisher != nil {
			status, err := s.publisher.Publish(ctx, s.now())
			if err != nil && ctx.Err() == nil {
				logger.Error("enroll_publication_unavailable")
			} else if status.Publication != "loaded" && ctx.Err() == nil {
				logger.Warn("enroll_publication_pending", "target_exceeded", status.TargetExceeded)
			}
		}
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
		}
	}
}
