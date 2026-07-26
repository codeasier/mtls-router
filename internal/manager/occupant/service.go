package occupant

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
)

const tokenLifetime = 30 * time.Second

type Config struct {
	ListenAddr      string
	DesktopPID      int
	ManagerIdentity process.Identity
	IsProtected     func(Identity) bool
	IsProtectedPID  func(int) bool
	ReleaseTimeout  time.Duration
	PollInterval    time.Duration
}

type Dependencies struct {
	Discover        func(context.Context) discovery.Result
	Inspect         func(context.Context, string) (Target, error)
	SupportsPIDOnly func() bool
	InspectPIDOwner func(context.Context, string) (int, error)
	SignalPID       func(int) error
	CurrentUser     func() (string, error)
	SameProcess     func(process.Identity, process.Identity) (bool, error)
	Validate        func(process.Identity, string) (process.Status, error)
	Signal          func(process.Identity, os.Signal) error
	Dial            func(context.Context, string, string) (net.Conn, error)
	Random          io.Reader
	Now             func() time.Time
	Sleep           func(context.Context, time.Duration) error
}

type tokenRecord struct {
	value     string
	expiresAt time.Time
	target    Target
}

type Service struct {
	config Config
	deps   Dependencies
	mu     sync.Mutex
	token  *tokenRecord
}

func New(config Config, deps Dependencies) *Service {
	if config.ReleaseTimeout <= 0 {
		config.ReleaseTimeout = 2 * time.Second
	}
	if config.PollInterval <= 0 {
		config.PollInterval = 50 * time.Millisecond
	}
	if deps.Inspect == nil {
		deps.Inspect = inspectNative
	}
	if deps.SupportsPIDOnly == nil {
		deps.SupportsPIDOnly = supportsPIDOnlyNative
	}
	if deps.InspectPIDOwner == nil {
		deps.InspectPIDOwner = inspectPIDOwnerNative
	}
	if deps.SignalPID == nil {
		deps.SignalPID = signalPIDNative
	}
	if deps.CurrentUser == nil {
		deps.CurrentUser = currentUserNative
	}
	if deps.SameProcess == nil {
		deps.SameProcess = process.SameIdentity
	}
	if deps.Validate == nil {
		deps.Validate = process.Validate
	}
	if deps.Signal == nil {
		deps.Signal = process.SignalIdentity
	}
	if deps.Dial == nil {
		deps.Dial = (&net.Dialer{Timeout: 50 * time.Millisecond}).DialContext
	}
	if deps.Random == nil {
		deps.Random = rand.Reader
	}
	if deps.Now == nil {
		deps.Now = time.Now
	}
	if deps.Sleep == nil {
		deps.Sleep = func(ctx context.Context, delay time.Duration) error {
			timer := time.NewTimer(delay)
			defer timer.Stop()
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-timer.C:
				return nil
			}
		}
	}
	return &Service{config: config, deps: deps}
}

func (s *Service) Inspect(ctx context.Context) (Inspection, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.token = nil
	if err := s.requireUnknown(ctx); err != nil {
		return Inspection{}, err
	}
	target, err := s.deps.Inspect(ctx, s.config.ListenAddr)
	if err != nil {
		return Inspection{}, err
	}
	inspection, forceable, err := s.classifyTarget(target)
	if err != nil {
		return Inspection{}, err
	}
	if !forceable {
		return inspection, nil
	}
	record, err := s.mintToken(target)
	if err != nil {
		return Inspection{}, err
	}
	inspection.Recovery = Recovery{Action: RecoveryActionForceTerminate}
	inspection.ConfirmationToken = record.value
	inspection.ExpiresAt = &record.expiresAt
	return inspection, nil
}

func (s *Service) mintToken(target Target) (*tokenRecord, error) {
	random := make([]byte, 32)
	if _, err := io.ReadFull(s.deps.Random, random); err != nil {
		return nil, ErrIdentityUnavailable
	}
	now := s.deps.Now().UTC()
	record := &tokenRecord{
		value: base64.RawURLEncoding.EncodeToString(random), expiresAt: now.Add(tokenLifetime), target: target,
	}
	s.token = record
	return record, nil
}

func (s *Service) classifyTarget(target Target) (Inspection, bool, error) {
	invalidSupervisor := target.Supervisor != nil && !validSupervisor(target.Supervisor)
	if invalidSupervisor {
		target.Supervisor = nil
	}
	inspection := Inspection{
		PID: target.PID, VerificationMode: target.Mode, ListenAddr: target.ListenAddr, Supervisor: target.Supervisor,
	}
	identity := target.Identity
	validVerifiedIdentity := target.Mode == VerificationModeVerifiedIdentity &&
		identity.ListenAddr == s.config.ListenAddr && identity.Network == "tcp4" && identity.SocketID != "" &&
		identity.Process.PID > 0 && identity.Process.StartedAt != "" && identity.Process.Executable != "" && identity.UserID != "" &&
		target.PID == identity.Process.PID && target.ListenAddr == identity.ListenAddr
	if validVerifiedIdentity {
		inspection.ProcessName = filepath.Base(identity.Process.Executable)
		inspection.Executable = identity.Process.Executable
	}
	if s.isProtectedTarget(target) {
		inspection.Supervisor = nil
		inspection.Recovery = Recovery{Action: RecoveryActionUnavailable, Reason: RecoveryReasonProtectedProcess}
		return inspection, false, nil
	}
	if target.Mode == VerificationModeVerifiedIdentity {
		if !validVerifiedIdentity {
			inspection.Supervisor = nil
			inspection.Recovery = Recovery{Action: RecoveryActionUnavailable, Reason: RecoveryReasonIdentityUnavailable}
			return inspection, false, nil
		}
		userID, err := s.deps.CurrentUser()
		if err != nil || userID == "" {
			inspection.Supervisor = nil
			inspection.Recovery = Recovery{Action: RecoveryActionUnavailable, Reason: RecoveryReasonIdentityUnavailable}
			return inspection, false, nil
		}
		if identity.UserID != userID {
			inspection.ProcessName = ""
			inspection.Executable = ""
			inspection.Supervisor = nil
			inspection.Recovery = Recovery{Action: RecoveryActionManualStopRequired, Reason: RecoveryReasonDifferentUser}
			return inspection, false, nil
		}
	}
	if invalidSupervisor {
		inspection.Recovery = Recovery{Action: RecoveryActionUnavailable, Reason: RecoveryReasonIdentityUnavailable}
		return inspection, false, nil
	}
	if target.Supervisor != nil {
		inspection.Recovery = Recovery{Action: RecoveryActionManualStopRequired, Reason: RecoveryReasonServiceManaged}
		return inspection, false, nil
	}
	if target.BlockReason != "" {
		recovery, ok := recoveryForReason(target.BlockReason)
		if !ok {
			return Inspection{}, false, ErrIdentityUnavailable
		}
		inspection.Recovery = recovery
		if recovery.Reason == RecoveryReasonDifferentUser {
			inspection.ProcessName = ""
			inspection.Executable = ""
		}
		return inspection, false, nil
	}
	switch target.Mode {
	case VerificationModeVerifiedIdentity:
		return inspection, true, nil
	case VerificationModeWindowsPIDOnly:
		if s.deps.SupportsPIDOnly == nil || !s.deps.SupportsPIDOnly() || s.deps.InspectPIDOwner == nil || s.deps.SignalPID == nil || target.ListenAddr != s.config.ListenAddr || target.PID <= 0 {
			inspection.Recovery = Recovery{Action: RecoveryActionUnavailable, Reason: RecoveryReasonIdentityUnavailable}
			return inspection, false, nil
		}
		return inspection, true, nil
	default:
		inspection.Recovery = Recovery{Action: RecoveryActionUnavailable, Reason: RecoveryReasonIdentityUnavailable}
		return inspection, false, nil
	}
}

func recoveryForReason(reason RecoveryReason) (Recovery, bool) {
	switch reason {
	case RecoveryReasonServiceManaged, RecoveryReasonInsufficientPrivilege, RecoveryReasonDifferentUser:
		return Recovery{Action: RecoveryActionManualStopRequired, Reason: reason}, true
	case RecoveryReasonProtectedProcess, RecoveryReasonIdentityUnavailable:
		return Recovery{Action: RecoveryActionUnavailable, Reason: reason}, true
	default:
		return Recovery{}, false
	}
}

func (s *Service) ForceTerminate(ctx context.Context, token string) (Result, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	record := s.token
	s.token = nil
	if record == nil || token == "" || token != record.value || !s.deps.Now().Before(record.expiresAt) {
		return Result{}, ErrConfirmationExpired
	}
	preSignalCtx, cancel := reserveReleaseWindow(ctx, s.config.ReleaseTimeout)
	defer cancel()
	if err := s.requireUnknown(preSignalCtx); err != nil {
		if record.target.Mode == VerificationModeWindowsPIDOnly && errors.Is(err, ErrNotFound) {
			return Result{}, ErrChanged
		}
		return Result{}, err
	}
	switch record.target.Mode {
	case VerificationModeVerifiedIdentity:
		return s.forceTerminateVerified(preSignalCtx, ctx, record.target)
	case VerificationModeWindowsPIDOnly:
		return s.forceTerminatePIDOnly(preSignalCtx, ctx, record.target)
	default:
		return Result{}, ErrChanged
	}
}

func (s *Service) forceTerminateVerified(preSignalCtx, ctx context.Context, target Target) (Result, error) {
	live, err := s.inspectVerifiedEligible(preSignalCtx)
	if err != nil {
		if errors.Is(err, ErrNotFound) {
			return Result{}, ErrChanged
		}
		return Result{}, err
	}
	same, err := sameIdentity(target.Identity, live.Identity, s.deps.SameProcess)
	if err != nil || !same {
		return Result{}, ErrChanged
	}
	if live.BlockReason == RecoveryReasonInsufficientPrivilege {
		return Result{}, ErrPermissionDenied
	}
	status, err := s.deps.Validate(live.Identity.Process, live.Identity.Process.Executable)
	if err != nil || status != process.StatusGenuine {
		return Result{}, ErrChanged
	}
	if err := s.deps.Signal(live.Identity.Process, os.Kill); err != nil {
		if errors.Is(err, process.ErrIdentityMismatch) || errors.Is(err, process.ErrNotFound) {
			return Result{}, ErrChanged
		}
		if errors.Is(err, os.ErrPermission) || errors.Is(err, ErrPermissionDenied) {
			return Result{}, ErrPermissionDenied
		}
		return Result{}, ErrTerminationFailed
	}
	return s.waitReleased(ctx, live.Identity)
}

func (s *Service) forceTerminatePIDOnly(preSignalCtx, ctx context.Context, target Target) (Result, error) {
	livePID, err := s.deps.InspectPIDOwner(preSignalCtx, target.ListenAddr)
	if err != nil || livePID != target.PID {
		return Result{}, ErrChanged
	}
	if s.isProtectedPID(livePID) {
		return Result{}, ErrProtected
	}
	if preSignalCtx.Err() != nil {
		return Result{}, ErrChanged
	}
	if err := s.deps.SignalPID(livePID); err != nil {
		if errors.Is(err, process.ErrNotFound) {
			return Result{}, ErrChanged
		}
		if errors.Is(err, os.ErrPermission) || errors.Is(err, ErrPermissionDenied) {
			return Result{}, ErrPermissionDenied
		}
		return Result{}, ErrTerminationFailed
	}
	return s.waitPIDReleased(ctx, target)
}

func (s *Service) inspectVerifiedEligible(ctx context.Context) (Target, error) {
	target, err := s.deps.Inspect(ctx, s.config.ListenAddr)
	if err != nil {
		return Target{}, err
	}
	if target.Mode != VerificationModeVerifiedIdentity {
		return Target{}, ErrChanged
	}
	return s.validateVerifiedTarget(target)
}

func (s *Service) validateVerifiedTarget(target Target) (Target, error) {
	if target.Supervisor != nil {
		return Target{}, ErrChanged
	}
	switch target.BlockReason {
	case "", RecoveryReasonInsufficientPrivilege:
	case RecoveryReasonProtectedProcess:
		return Target{}, ErrProtected
	default:
		return Target{}, ErrChanged
	}
	identity := target.Identity
	if identity.ListenAddr != s.config.ListenAddr || identity.Network != "tcp4" || identity.SocketID == "" || identity.Process.PID <= 0 || identity.Process.StartedAt == "" || identity.Process.Executable == "" || identity.UserID == "" {
		return Target{}, ErrIdentityUnavailable
	}
	if target.PID != identity.Process.PID || target.ListenAddr != identity.ListenAddr {
		return Target{}, ErrIdentityUnavailable
	}
	userID, err := s.deps.CurrentUser()
	if err != nil || userID == "" {
		return Target{}, ErrIdentityUnavailable
	}
	if identity.UserID != userID {
		return Target{}, ErrNotOwned
	}
	if identity.Process.PID == s.config.DesktopPID || identity.Process.PID == s.config.ManagerIdentity.PID || (s.config.IsProtected != nil && s.config.IsProtected(identity)) {
		return Target{}, ErrProtected
	}
	return target, nil
}

func (s *Service) isProtectedPID(pid int) bool {
	return pid > 0 && (pid == s.config.DesktopPID || pid == s.config.ManagerIdentity.PID || (s.config.IsProtectedPID != nil && s.config.IsProtectedPID(pid)))
}

func (s *Service) isProtectedTarget(target Target) bool {
	if target.BlockReason == RecoveryReasonProtectedProcess || s.isProtectedPID(target.PID) {
		return true
	}
	if target.Mode != VerificationModeVerifiedIdentity {
		return false
	}
	identity := target.Identity
	return s.isProtectedPID(identity.Process.PID) || (s.config.IsProtected != nil && s.config.IsProtected(identity))
}

func (s *Service) requireUnknown(ctx context.Context) error {
	if s.deps.Discover == nil {
		return ErrIdentityUnavailable
	}
	found := s.deps.Discover(ctx)
	if ctx.Err() != nil {
		return ErrIdentityUnavailable
	}
	if found.Classification != discovery.UnknownOccupant {
		if found.Classification == discovery.Absent {
			return ErrNotFound
		}
		return ErrProtected
	}
	return nil
}

func (s *Service) waitReleased(parent context.Context, identity Identity) (Result, error) {
	ctx, cancel := context.WithTimeout(parent, s.config.ReleaseTimeout)
	defer cancel()
	for {
		status, _ := s.deps.Validate(identity.Process, identity.Process.Executable)
		if status == process.StatusStale {
			return Result{}, ErrChanged
		}
		if status == process.StatusAbsent {
			conn, err := s.deps.Dial(ctx, "tcp4", identity.ListenAddr)
			if err != nil {
				return releasedResult(), nil
			}
			_ = conn.Close()
			if replacement, inspectErr := s.deps.Inspect(ctx, identity.ListenAddr); inspectErr == nil {
				same, _ := sameIdentity(identity, replacement.Identity, s.deps.SameProcess)
				if !same {
					return Result{}, ErrChanged
				}
			}
		}
		if err := s.deps.Sleep(ctx, s.config.PollInterval); err != nil {
			finalStatus, _ := s.deps.Validate(identity.Process, identity.Process.Executable)
			switch finalStatus {
			case process.StatusStale:
				return Result{}, ErrChanged
			case process.StatusGenuine:
				return Result{}, ErrTerminationFailed
			default:
				return Result{}, ErrPortReleaseTimeout
			}
		}
	}
}

func (s *Service) waitPIDReleased(parent context.Context, target Target) (Result, error) {
	ctx, cancel := context.WithTimeout(parent, s.config.ReleaseTimeout)
	defer cancel()
	for {
		pid, err := s.deps.InspectPIDOwner(ctx, target.ListenAddr)
		if errors.Is(err, ErrNotFound) {
			return releasedResult(), nil
		}
		if err != nil || pid != target.PID {
			return Result{}, ErrChanged
		}
		if err := s.deps.Sleep(ctx, s.config.PollInterval); err != nil {
			finalCtx, cancel := context.WithTimeout(context.WithoutCancel(parent), s.config.PollInterval)
			pid, inspectErr := s.deps.InspectPIDOwner(finalCtx, target.ListenAddr)
			cancel()
			switch {
			case errors.Is(inspectErr, ErrNotFound):
				return releasedResult(), nil
			case inspectErr != nil || pid != target.PID:
				return Result{}, ErrChanged
			default:
				return Result{}, ErrPortReleaseTimeout
			}
		}
	}
}

func releasedResult() Result {
	return Result{Termination: "process_terminated", PortState: "released"}
}

func reserveReleaseWindow(parent context.Context, releaseTimeout time.Duration) (context.Context, context.CancelFunc) {
	deadline, ok := parent.Deadline()
	if !ok {
		return context.WithCancel(parent)
	}
	return context.WithDeadline(parent, deadline.Add(-releaseTimeout))
}

func sameIdentity(left, right Identity, sameProcess func(process.Identity, process.Identity) (bool, error)) (bool, error) {
	if left.ListenAddr != right.ListenAddr || left.Network != right.Network || left.SocketID != right.SocketID || left.UserID != right.UserID {
		return false, nil
	}
	return sameProcess(left.Process, right.Process)
}

func validateAddress(value string) (net.IP, int, error) {
	host, portText, err := net.SplitHostPort(value)
	if err != nil {
		return nil, 0, ErrIdentityUnavailable
	}
	ip := net.ParseIP(strings.TrimSpace(host))
	if ip == nil || ip.To4() == nil || !ip.IsLoopback() {
		return nil, 0, ErrIdentityUnavailable
	}
	port, err := net.LookupPort("tcp", portText)
	if err != nil || port <= 0 {
		return nil, 0, ErrIdentityUnavailable
	}
	return ip.To4(), port, nil
}
