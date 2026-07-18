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
	ReleaseTimeout  time.Duration
	PollInterval    time.Duration
}

type Dependencies struct {
	Discover    func(context.Context) discovery.Result
	Inspect     func(context.Context, string) (Identity, error)
	CurrentUser func() (string, error)
	SameProcess func(process.Identity, process.Identity) (bool, error)
	Validate    func(process.Identity, string) (process.Status, error)
	Signal      func(process.Identity, os.Signal) error
	Dial        func(context.Context, string, string) (net.Conn, error)
	Random      io.Reader
	Now         func() time.Time
	Sleep       func(context.Context, time.Duration) error
}

type tokenRecord struct {
	value     string
	expiresAt time.Time
	identity  Identity
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
	identity, err := s.inspectEligible(ctx)
	if err != nil {
		return Inspection{}, err
	}
	random := make([]byte, 32)
	if _, err := io.ReadFull(s.deps.Random, random); err != nil {
		return Inspection{}, ErrIdentityUnavailable
	}
	now := s.deps.Now().UTC()
	record := &tokenRecord{
		value: base64.RawURLEncoding.EncodeToString(random), expiresAt: now.Add(tokenLifetime), identity: identity,
	}
	s.token = record
	return Inspection{
		PID: identity.Process.PID, ProcessName: filepath.Base(identity.Process.Executable),
		Executable: identity.Process.Executable, ListenAddr: identity.ListenAddr,
		ConfirmationToken: record.value, ExpiresAt: record.expiresAt,
	}, nil
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
		return Result{}, err
	}
	live, err := s.inspectEligible(preSignalCtx)
	if err != nil {
		if errors.Is(err, ErrNotFound) {
			return Result{}, ErrChanged
		}
		return Result{}, err
	}
	same, err := sameIdentity(record.identity, live, s.deps.SameProcess)
	if err != nil || !same {
		return Result{}, ErrChanged
	}
	status, err := s.deps.Validate(live.Process, live.Process.Executable)
	if err != nil || status != process.StatusGenuine {
		return Result{}, ErrChanged
	}
	if err := s.deps.Signal(live.Process, os.Kill); err != nil {
		if errors.Is(err, process.ErrIdentityMismatch) || errors.Is(err, process.ErrNotFound) {
			return Result{}, ErrChanged
		}
		return Result{}, ErrTerminationFailed
	}
	return s.waitReleased(ctx, live)
}

func (s *Service) inspectEligible(ctx context.Context) (Identity, error) {
	identity, err := s.deps.Inspect(ctx, s.config.ListenAddr)
	if err != nil {
		return Identity{}, err
	}
	if identity.ListenAddr != s.config.ListenAddr || identity.Network != "tcp4" || identity.SocketID == "" || identity.Process.PID <= 0 || identity.Process.StartedAt == "" || identity.Process.Executable == "" || identity.UserID == "" {
		return Identity{}, ErrIdentityUnavailable
	}
	userID, err := s.deps.CurrentUser()
	if err != nil || userID == "" {
		return Identity{}, ErrIdentityUnavailable
	}
	if identity.UserID != userID {
		return Identity{}, ErrNotOwned
	}
	if identity.Process.PID == s.config.DesktopPID || identity.Process.PID == s.config.ManagerIdentity.PID || (s.config.IsProtected != nil && s.config.IsProtected(identity)) {
		return Identity{}, ErrProtected
	}
	return identity, nil
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
				return Result{State: string(discovery.Absent)}, nil
			}
			_ = conn.Close()
			if replacement, inspectErr := s.deps.Inspect(ctx, identity.ListenAddr); inspectErr == nil {
				same, _ := sameIdentity(identity, replacement, s.deps.SameProcess)
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
