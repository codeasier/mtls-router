package occupant

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
)

func TestServiceInspectionTokenAndTermination(t *testing.T) {
	now := time.Date(2026, 7, 18, 1, 2, 3, 0, time.UTC)
	identity := testIdentity()
	status := process.StatusGenuine
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil },
		CurrentUser: func() (string, error) { return identity.UserID, nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return status, nil },
		Signal: func(got process.Identity, signal os.Signal) error {
			if signal != os.Kill || got != identity.Process {
				t.Fatalf("signal = %v identity = %+v", signal, got)
			}
			signals++
			status = process.StatusAbsent
			return nil
		},
		Dial:   func(context.Context, string, string) (net.Conn, error) { return nil, errors.New("refused") },
		Random: bytes.NewReader(bytes.Repeat([]byte{7}, 64)), Now: func() time.Time { return now },
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(inspection.ConfirmationToken) != 43 || !inspection.ExpiresAt.Equal(now.Add(30*time.Second)) || inspection.ProcessName != "listener" || inspection.VerificationMode != VerificationModeVerifiedIdentity {
		t.Fatalf("inspection = %+v", inspection)
	}
	result, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken)
	if err != nil || result.Termination != "process_terminated" || result.PortState != "released" || signals != 1 {
		t.Fatalf("result=%+v err=%v signals=%d", result, err, signals)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) || signals != 1 {
		t.Fatalf("replay err=%v signals=%d", err, signals)
	}
}

func TestServiceInspectRecoveryActions(t *testing.T) {
	identity := testIdentity()
	verified := verifiedTarget(identity)
	pidOnly := Target{Mode: VerificationModeWindowsPIDOnly, PID: identity.Process.PID, ListenAddr: identity.ListenAddr}
	serviceTarget := verified
	serviceTarget.Supervisor = &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: []string{"RouterSvc"}}
	pidOnlyServiceTarget := pidOnly
	pidOnlyServiceTarget.Supervisor = &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: []string{"RouterSvc"}}
	pidOnlyServiceTarget.BlockReason = RecoveryReasonServiceManaged
	sessionZeroTarget := pidOnly
	sessionZeroTarget.BlockReason = RecoveryReasonIdentityUnavailable
	permissionTarget := pidOnly
	permissionTarget.BlockReason = RecoveryReasonInsufficientPrivilege
	differentUserTarget := verified
	differentUserTarget.BlockReason = RecoveryReasonDifferentUser
	protectedTarget := verified
	protectedTarget.BlockReason = RecoveryReasonProtectedProcess
	scmQueryUnavailableTarget := pidOnly
	scmQueryUnavailableTarget.BlockReason = RecoveryReasonIdentityUnavailable
	serviceFor := func(target Target) *Service {
		return New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
			Discover: func(context.Context) discovery.Result {
				return discovery.Result{Classification: discovery.UnknownOccupant}
			},
			Inspect:         func(context.Context, string) (Target, error) { return target, nil },
			SupportsPIDOnly: func() bool { return true },
			CurrentUser:     func() (string, error) { return identity.UserID, nil },
			Random:          bytes.NewReader(make([]byte, 32)),
		})
	}
	tests := []struct {
		name         string
		target       Target
		wantAction   RecoveryAction
		wantReason   RecoveryReason
		wantToken    bool
		wantMetadata bool
	}{
		{name: "verified current user", target: verified, wantAction: RecoveryActionForceTerminate, wantToken: true, wantMetadata: true},
		{name: "service", target: serviceTarget, wantAction: RecoveryActionManualStopRequired, wantReason: RecoveryReasonServiceManaged, wantMetadata: true},
		{name: "PID-only SCM service", target: pidOnlyServiceTarget, wantAction: RecoveryActionManualStopRequired, wantReason: RecoveryReasonServiceManaged},
		{name: "unnamed Session 0 target", target: sessionZeroTarget, wantAction: RecoveryActionUnavailable, wantReason: RecoveryReasonIdentityUnavailable},
		{name: "permission denied", target: permissionTarget, wantAction: RecoveryActionManualStopRequired, wantReason: RecoveryReasonInsufficientPrivilege},
		{name: "different user", target: differentUserTarget, wantAction: RecoveryActionManualStopRequired, wantReason: RecoveryReasonDifferentUser},
		{name: "protected", target: protectedTarget, wantAction: RecoveryActionUnavailable, wantReason: RecoveryReasonProtectedProcess, wantMetadata: true},
		{name: "SCM query unavailable", target: scmQueryUnavailableTarget, wantAction: RecoveryActionUnavailable, wantReason: RecoveryReasonIdentityUnavailable},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			inspection, err := serviceFor(test.target).Inspect(context.Background())
			if err != nil || inspection.Recovery.Action != test.wantAction || inspection.Recovery.Reason != test.wantReason {
				t.Fatalf("inspection = %+v, err = %v", inspection, err)
			}
			if (inspection.ConfirmationToken != "") != test.wantToken || (inspection.ExpiresAt != nil) != test.wantToken {
				t.Fatalf("token invariant failed: %+v", inspection)
			}
			hasMetadata := inspection.ProcessName != "" || inspection.Executable != ""
			if hasMetadata != test.wantMetadata || (hasMetadata && (inspection.ProcessName != "listener" || inspection.Executable != identity.Process.Executable)) {
				t.Fatalf("metadata invariant failed: %+v", inspection)
			}
		})
	}
}

func TestServiceInspectClassifiesSystemdSupervisorAfterOwnership(t *testing.T) {
	identity := testIdentity()
	identity.UserID = "0"
	target := verifiedTarget(identity)
	target.Supervisor = &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"router.service"}}

	for _, test := range []struct {
		name           string
		currentUser    string
		wantReason     RecoveryReason
		wantSupervisor bool
		wantMetadata   bool
	}{
		{name: "same user remains service managed", currentUser: identity.UserID, wantReason: RecoveryReasonServiceManaged, wantSupervisor: true, wantMetadata: true},
		{name: "root service under different user redacts metadata", currentUser: "1001", wantReason: RecoveryReasonDifferentUser},
	} {
		t.Run(test.name, func(t *testing.T) {
			service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
				Discover: func(context.Context) discovery.Result {
					return discovery.Result{Classification: discovery.UnknownOccupant}
				},
				Inspect:     func(context.Context, string) (Target, error) { return target, nil },
				CurrentUser: func() (string, error) { return test.currentUser, nil },
				Random:      bytes.NewReader(make([]byte, 32)),
			})

			inspection, err := service.Inspect(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if inspection.Recovery.Action != RecoveryActionManualStopRequired || inspection.Recovery.Reason != test.wantReason {
				t.Fatalf("recovery = %+v", inspection.Recovery)
			}
			if (inspection.Supervisor != nil) != test.wantSupervisor {
				t.Fatalf("supervisor = %+v", inspection.Supervisor)
			}
			hasMetadata := inspection.ProcessName != "" || inspection.Executable != ""
			if hasMetadata != test.wantMetadata {
				t.Fatalf("metadata invariant failed: %+v", inspection)
			}
		})
	}
}

func TestServiceInspectRejectsMalformedSupervisorMetadata(t *testing.T) {
	identity := testIdentity()
	seventeen := make([]string, 17)
	for index := range seventeen {
		seventeen[index] = fmt.Sprintf("Service%02d", index)
	}
	largeStructure := make([]string, 16)
	for index := range largeStructure {
		largeStructure[index] = fmt.Sprintf("%02d%s", index, strings.Repeat("x", 254))
	}
	tests := []struct {
		name       string
		supervisor *Supervisor
	}{
		{name: "unknown kind", supervisor: &Supervisor{Kind: "other", Scope: SupervisorScopeSystem, Identifiers: []string{"Alpha"}}},
		{name: "wrong scope", supervisor: &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeUser, Identifiers: []string{"Alpha"}}},
		{name: "unsorted", supervisor: &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: []string{"Beta", "Alpha"}}},
		{name: "duplicate", supervisor: &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: []string{"Alpha", "Alpha"}}},
		{name: "too many", supervisor: &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: seventeen}},
		{name: "identifier too large", supervisor: &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: []string{strings.Repeat("x", 257)}}},
		{name: "structure too large", supervisor: &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: largeStructure}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			target := verifiedTarget(identity)
			target.Supervisor = test.supervisor
			service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
				Discover: func(context.Context) discovery.Result {
					return discovery.Result{Classification: discovery.UnknownOccupant}
				},
				Inspect:     func(context.Context, string) (Target, error) { return target, nil },
				CurrentUser: func() (string, error) { return identity.UserID, nil },
				Random:      bytes.NewReader(make([]byte, 32)),
			})

			inspection, err := service.Inspect(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := json.Marshal(inspection)
			if err != nil {
				t.Fatal(err)
			}
			if inspection.Supervisor != nil || inspection.Recovery.Reason != RecoveryReasonIdentityUnavailable || inspection.ConfirmationToken != "" || bytes.Contains(encoded, []byte(`"supervisor"`)) {
				t.Fatalf("malformed supervisor serialized or became forceable: %s", encoded)
			}
		})
	}
}

func TestServiceInspectProtectedTargetOmitsSupervisor(t *testing.T) {
	identity := testIdentity()
	target := verifiedTarget(identity)
	target.Supervisor = &Supervisor{
		Kind:        SupervisorWindowsService,
		Scope:       SupervisorScopeSystem,
		Identifiers: []string{"RouterSvc"},
	}
	service := New(Config{ListenAddr: identity.ListenAddr, DesktopPID: identity.Process.PID}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return target, nil },
		CurrentUser: func() (string, error) { return identity.UserID, nil },
		Random:      bytes.NewReader(make([]byte, 32)),
	})

	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(inspection)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"pid":42,"verification_mode":"verified_identity","process_name":"listener","executable":"/tmp/listener","listen_addr":"127.0.0.1:19099","recovery":{"action":"unavailable","reason":"protected_process"}}`
	if inspection.Supervisor != nil || string(encoded) != want {
		t.Fatalf("protected inspection shape = %s, want %s", encoded, want)
	}
}

func TestServiceInspectDifferentUserOmitsProcessMetadata(t *testing.T) {
	identity := testIdentity()
	identity.UserID = "other-user"
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil },
		CurrentUser: func() (string, error) { return "current-user", nil },
		Random:      bytes.NewReader(make([]byte, 32)),
	})

	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if inspection.Recovery.Action != RecoveryActionManualStopRequired || inspection.Recovery.Reason != RecoveryReasonDifferentUser {
		t.Fatalf("recovery = %+v", inspection.Recovery)
	}
	if inspection.ProcessName != "" || inspection.Executable != "" {
		t.Fatalf("different-user inspection exposed process metadata: %+v", inspection)
	}
	encoded, err := json.Marshal(inspection)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"pid":42,"verification_mode":"verified_identity","listen_addr":"127.0.0.1:19099","recovery":{"action":"manual_stop_required","reason":"different_user"}}`
	if string(encoded) != want {
		t.Fatalf("different-user inspection shape = %s, want %s", encoded, want)
	}
}

func TestServiceForceTerminateMapsPermissionDenied(t *testing.T) {
	identity := testIdentity()
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil },
		CurrentUser: func() (string, error) { return identity.UserID, nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
		Signal:      func(process.Identity, os.Signal) error { return os.ErrPermission },
		Random:      bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrPermissionDenied) {
		t.Fatalf("error = %v, want %v", err, ErrPermissionDenied)
	}
}

func TestServiceExpirySupersessionAndRestart(t *testing.T) {
	now := time.Now()
	newService := func() *Service {
		return New(Config{ListenAddr: testIdentity().ListenAddr}, Dependencies{
			Discover: func(context.Context) discovery.Result {
				return discovery.Result{Classification: discovery.UnknownOccupant}
			},
			Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(testIdentity()), nil },
			CurrentUser: func() (string, error) { return "user", nil },
			SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
			Validate:    func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
			Random:      bytes.NewReader(append(bytes.Repeat([]byte{9}, 32), bytes.Repeat([]byte{10}, 96)...)),
			Now:         func() time.Time { return now },
		})
	}
	service := newService()
	first, _ := service.Inspect(context.Background())
	second, _ := service.Inspect(context.Background())
	if _, err := service.ForceTerminate(context.Background(), first.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) {
		t.Fatalf("superseded token error = %v", err)
	}
	if _, err := newService().ForceTerminate(context.Background(), second.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) {
		t.Fatalf("restart token error = %v", err)
	}
	third, _ := service.Inspect(context.Background())
	now = now.Add(30 * time.Second)
	if _, err := service.ForceTerminate(context.Background(), third.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) {
		t.Fatalf("expired token error = %v", err)
	}
}

func TestServiceDiagnosesChangedOwnerAndProtectedWithoutSignal(t *testing.T) {
	for _, test := range []struct {
		name       string
		mutate     func(*Identity, *Config)
		wantAction RecoveryAction
		wantReason RecoveryReason
	}{
		{name: "other user", mutate: func(identity *Identity, _ *Config) { identity.UserID = "other" }, wantAction: RecoveryActionManualStopRequired, wantReason: RecoveryReasonDifferentUser},
		{name: "desktop", mutate: func(identity *Identity, config *Config) { config.DesktopPID = identity.Process.PID }, wantAction: RecoveryActionUnavailable, wantReason: RecoveryReasonProtectedProcess},
		{name: "manager", mutate: func(identity *Identity, config *Config) { config.ManagerIdentity = identity.Process }, wantAction: RecoveryActionUnavailable, wantReason: RecoveryReasonProtectedProcess},
		{name: "managed router", mutate: func(_ *Identity, config *Config) { config.IsProtected = func(Identity) bool { return true } }, wantAction: RecoveryActionUnavailable, wantReason: RecoveryReasonProtectedProcess},
	} {
		t.Run(test.name, func(t *testing.T) {
			identity := testIdentity()
			config := Config{ListenAddr: identity.ListenAddr}
			test.mutate(&identity, &config)
			signals := 0
			service := New(config, Dependencies{
				Discover: func(context.Context) discovery.Result {
					return discovery.Result{Classification: discovery.UnknownOccupant}
				},
				Inspect: func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil }, CurrentUser: func() (string, error) { return "user", nil },
				Signal: func(process.Identity, os.Signal) error { signals++; return nil }, Random: bytes.NewReader(make([]byte, 32)),
			})
			inspection, err := service.Inspect(context.Background())
			if err != nil || inspection.Recovery.Action != test.wantAction || inspection.Recovery.Reason != test.wantReason || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil || signals != 0 {
				t.Fatalf("inspection=%+v error=%v signals=%d", inspection, err, signals)
			}
		})
	}
}

func TestServiceConcurrentTokenConsumptionSignalsOnce(t *testing.T) {
	identity := testIdentity()
	status := process.StatusGenuine
	var signalMu sync.Mutex
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect: func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil }, CurrentUser: func() (string, error) { return "user", nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return status, nil },
		Signal: func(process.Identity, os.Signal) error {
			signalMu.Lock()
			signals++
			status = process.StatusAbsent
			signalMu.Unlock()
			return nil
		},
		Dial: func(context.Context, string, string) (net.Conn, error) { return nil, errors.New("refused") }, Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, _ := service.Inspect(context.Background())
	var wait sync.WaitGroup
	for range 2 {
		wait.Add(1)
		go func() {
			defer wait.Done()
			_, _ = service.ForceTerminate(context.Background(), inspection.ConfirmationToken)
		}()
	}
	wait.Wait()
	if signals != 1 {
		t.Fatalf("signals = %d", signals)
	}
}

func TestServiceChangedIdentityConsumesTokenWithoutSignal(t *testing.T) {
	identity := testIdentity()
	live := identity
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(live), nil },
		CurrentUser: func() (string, error) { return "user", nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Signal:      func(process.Identity, os.Signal) error { signals++; return nil }, Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	live.SocketID = "replacement-socket"
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrChanged) || signals != 0 {
		t.Fatalf("error=%v signals=%d", err, signals)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) || signals != 0 {
		t.Fatalf("replay error=%v signals=%d", err, signals)
	}
}

func TestServiceConfirmationPermissionLossConsumesTokenWithoutSignal(t *testing.T) {
	identity := testIdentity()
	permissionDenied := false
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect: func(context.Context, string) (Target, error) {
			target := verifiedTarget(identity)
			if permissionDenied {
				target.BlockReason = RecoveryReasonInsufficientPrivilege
			}
			return target, nil
		},
		CurrentUser: func() (string, error) { return identity.UserID, nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
		Signal:      func(process.Identity, os.Signal) error { signals++; return nil },
		Random:      bytes.NewReader(make([]byte, 32)),
	})

	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	permissionDenied = true
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrPermissionDenied) || signals != 0 {
		t.Fatalf("error=%v signals=%d", err, signals)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) || signals != 0 {
		t.Fatalf("replay error=%v signals=%d", err, signals)
	}
}

func TestServiceNeverSignalsReplacementDuringReleaseWait(t *testing.T) {
	identity := testIdentity()
	live := identity
	status := process.StatusGenuine
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(live), nil },
		CurrentUser: func() (string, error) { return "user", nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return status, nil },
		Signal: func(process.Identity, os.Signal) error {
			signals++
			status = process.StatusAbsent
			live.SocketID = "replacement"
			live.Process.PID++
			return nil
		},
		Dial:   func(context.Context, string, string) (net.Conn, error) { return &stubConn{}, nil },
		Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, _ := service.Inspect(context.Background())
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrChanged) || signals != 1 {
		t.Fatalf("error=%v signals=%d", err, signals)
	}
}

func TestServiceReservesReleaseWindowBeforeSignaling(t *testing.T) {
	identity := testIdentity()
	service := New(Config{ListenAddr: identity.ListenAddr, ReleaseTimeout: 2 * time.Second}, Dependencies{
		Discover: func(ctx context.Context) discovery.Result {
			deadline, ok := ctx.Deadline()
			if ok {
				remaining := time.Until(deadline)
				if remaining < 900*time.Millisecond || remaining > time.Second {
					t.Fatalf("pre-signal budget = %s", remaining)
				}
			}
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil },
		CurrentUser: func() (string, error) { return identity.UserID, nil },
		Random:      bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	_, _ = service.ForceTerminate(ctx, inspection.ConfirmationToken)
}

func TestServiceRevalidatesProcessWhenReleaseWaitExpires(t *testing.T) {
	for _, test := range []struct {
		name      string
		final     process.Status
		wantError error
	}{
		{name: "process exited", final: process.StatusAbsent, wantError: ErrPortReleaseTimeout},
		{name: "process survived", final: process.StatusGenuine, wantError: ErrTerminationFailed},
		{name: "identity changed", final: process.StatusStale, wantError: ErrChanged},
	} {
		t.Run(test.name, func(t *testing.T) {
			identity := testIdentity()
			status := process.StatusGenuine
			validations := 0
			service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
				Discover: func(context.Context) discovery.Result {
					return discovery.Result{Classification: discovery.UnknownOccupant}
				},
				Inspect:     func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil },
				CurrentUser: func() (string, error) { return identity.UserID, nil },
				SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
				Validate: func(process.Identity, string) (process.Status, error) {
					validations++
					if validations >= 3 {
						return test.final, nil
					}
					return status, nil
				},
				Signal: func(process.Identity, os.Signal) error { return nil },
				Sleep:  func(context.Context, time.Duration) error { return context.DeadlineExceeded },
				Random: bytes.NewReader(make([]byte, 32)),
			})
			inspection, err := service.Inspect(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, test.wantError) {
				t.Fatalf("error = %v, want %v", err, test.wantError)
			}
		})
	}
}

func TestServiceInspectPIDOnly(t *testing.T) {
	now := time.Date(2026, 7, 22, 1, 2, 3, 0, time.UTC)
	target := testPIDOnlyTarget()
	fixture := &pidOnlyFixture{supports: true, now: now}
	service := newPIDOnlyService(t, Config{ListenAddr: target.ListenAddr}, target, fixture)

	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if inspection.VerificationMode != VerificationModeWindowsPIDOnly || inspection.PID != target.PID || inspection.ListenAddr != target.ListenAddr || inspection.ConfirmationToken == "" || !inspection.ExpiresAt.Equal(now.Add(tokenLifetime)) {
		t.Fatalf("inspection = %+v", inspection)
	}
	if inspection.ProcessName != "" || inspection.Executable != "" {
		t.Fatalf("PID-only inspection exposed identity metadata: %+v", inspection)
	}
}

func TestServiceForceTerminatePIDOnly(t *testing.T) {
	for _, test := range []struct {
		name           string
		owners         []pidOwnerResult
		protectAfter   bool
		signalErr      error
		sleepErr       error
		wantError      error
		wantReleased   bool
		wantSignals    []int
		wantOwnerCalls int
	}{
		{name: "same PID released", owners: []pidOwnerResult{{pid: 4242}, {err: ErrNotFound}}, wantReleased: true, wantSignals: []int{4242}, wantOwnerCalls: 2},
		{name: "changed PID before signal", owners: []pidOwnerResult{{pid: 4343}}, wantError: ErrChanged, wantOwnerCalls: 1},
		{name: "owner disappeared before signal", owners: []pidOwnerResult{{err: ErrNotFound}}, wantError: ErrChanged, wantOwnerCalls: 1},
		{name: "ambiguous before signal", owners: []pidOwnerResult{{err: ErrIdentityUnavailable}}, wantError: ErrChanged, wantOwnerCalls: 1},
		{name: "protected before signal", owners: []pidOwnerResult{{pid: 4242}}, protectAfter: true, wantError: ErrProtected, wantOwnerCalls: 1},
		{name: "process disappeared while signaling", owners: []pidOwnerResult{{pid: 4242}}, signalErr: process.ErrNotFound, wantError: ErrChanged, wantSignals: []int{4242}, wantOwnerCalls: 1},
		{name: "permission denied while signaling", owners: []pidOwnerResult{{pid: 4242}}, signalErr: os.ErrPermission, wantError: ErrPermissionDenied, wantSignals: []int{4242}, wantOwnerCalls: 1},
		{name: "native permission denied while signaling", owners: []pidOwnerResult{{pid: 4242}}, signalErr: ErrPermissionDenied, wantError: ErrPermissionDenied, wantSignals: []int{4242}, wantOwnerCalls: 1},
		{name: "signal failed", owners: []pidOwnerResult{{pid: 4242}}, signalErr: errors.New("denied"), wantError: ErrTerminationFailed, wantSignals: []int{4242}, wantOwnerCalls: 1},
		{name: "replacement during release", owners: []pidOwnerResult{{pid: 4242}, {pid: 4343}}, wantError: ErrChanged, wantSignals: []int{4242}, wantOwnerCalls: 2},
		{name: "ambiguity during release", owners: []pidOwnerResult{{pid: 4242}, {err: ErrIdentityUnavailable}}, wantError: ErrChanged, wantSignals: []int{4242}, wantOwnerCalls: 2},
		{name: "released at timeout recheck", owners: []pidOwnerResult{{pid: 4242}, {pid: 4242}, {err: ErrNotFound}}, sleepErr: context.DeadlineExceeded, wantReleased: true, wantSignals: []int{4242}, wantOwnerCalls: 3},
		{name: "replacement at timeout recheck", owners: []pidOwnerResult{{pid: 4242}, {pid: 4242}, {pid: 4343}}, sleepErr: context.DeadlineExceeded, wantError: ErrChanged, wantSignals: []int{4242}, wantOwnerCalls: 3},
		{name: "same PID times out", owners: []pidOwnerResult{{pid: 4242}, {pid: 4242}, {pid: 4242}}, sleepErr: context.DeadlineExceeded, wantError: ErrPortReleaseTimeout, wantSignals: []int{4242}, wantOwnerCalls: 3},
	} {
		t.Run(test.name, func(t *testing.T) {
			target := testPIDOnlyTarget()
			fixture := &pidOnlyFixture{supports: true, owners: test.owners, signalErr: test.signalErr, sleepErr: test.sleepErr}
			config := Config{ListenAddr: target.ListenAddr, IsProtectedPID: func(int) bool { return fixture.protected }}
			service := newPIDOnlyService(t, config, target, fixture)
			inspection, err := service.Inspect(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			fixture.protected = test.protectAfter

			result, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken)
			isReleased := result.Termination == "process_terminated" && result.PortState == "released"
			if !errors.Is(err, test.wantError) || isReleased != test.wantReleased {
				t.Fatalf("result=%+v error=%v, want released=%t error=%v", result, err, test.wantReleased, test.wantError)
			}
			if !equalInts(fixture.signals, test.wantSignals) {
				t.Fatalf("signals=%v, want %v", fixture.signals, test.wantSignals)
			}
			if fixture.ownerCalls != test.wantOwnerCalls {
				t.Fatalf("owner calls=%d, want %d", fixture.ownerCalls, test.wantOwnerCalls)
			}
			if _, replayErr := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(replayErr, ErrConfirmationExpired) {
				t.Fatalf("replay error = %v", replayErr)
			}
		})
	}
}

func TestServicePIDOnlyDiscoveryAbsentAfterConfirmationReturnsChanged(t *testing.T) {
	target := testPIDOnlyTarget()
	fixture := &pidOnlyFixture{supports: true, discovery: discovery.UnknownOccupant}
	service := newPIDOnlyService(t, Config{ListenAddr: target.ListenAddr}, target, fixture)
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	fixture.discovery = discovery.Absent

	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrChanged) {
		t.Fatalf("error = %v, want %v", err, ErrChanged)
	}
	if fixture.ownerCalls != 0 || len(fixture.signals) != 0 {
		t.Fatalf("owner calls=%d signals=%v", fixture.ownerCalls, fixture.signals)
	}
}

func TestServicePIDOnlyCancellationBeforeSignalReturnsChangedWithoutSignal(t *testing.T) {
	target := testPIDOnlyTarget()
	ctx, cancel := context.WithCancel(context.Background())
	signals := 0
	service := New(Config{ListenAddr: target.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:         func(context.Context, string) (Target, error) { return target, nil },
		SupportsPIDOnly: func() bool { return true },
		InspectPIDOwner: func(context.Context, string) (int, error) {
			cancel()
			return target.PID, nil
		},
		SignalPID: func(int) error {
			signals++
			return nil
		},
		Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	if _, err := service.ForceTerminate(ctx, inspection.ConfirmationToken); !errors.Is(err, ErrChanged) {
		t.Fatalf("error = %v, want %v", err, ErrChanged)
	}
	if signals != 0 {
		t.Fatalf("signals = %d, want 0", signals)
	}
}

func TestServiceVerifiedDiscoveryAbsentAfterConfirmationRemainsNotFound(t *testing.T) {
	identity := testIdentity()
	classification := discovery.UnknownOccupant
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result { return discovery.Result{Classification: classification} },
		Inspect:  func(context.Context, string) (Target, error) { return verifiedTarget(identity), nil },
		CurrentUser: func() (string, error) {
			return identity.UserID, nil
		},
		Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	classification = discovery.Absent

	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrNotFound) {
		t.Fatalf("error = %v, want %v", err, ErrNotFound)
	}
}

func TestServicePIDOnlyTimeoutFinalLookupUsesLiveBoundedContext(t *testing.T) {
	target := testPIDOnlyTarget()
	ownerCalls := 0
	releaseTimeout := 40 * time.Millisecond
	pollInterval := 20 * time.Millisecond
	var parentDeadline time.Time
	service := New(Config{ListenAddr: target.ListenAddr, ReleaseTimeout: releaseTimeout, PollInterval: pollInterval}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:         func(context.Context, string) (Target, error) { return target, nil },
		SupportsPIDOnly: func() bool { return true },
		InspectPIDOwner: func(ctx context.Context, _ string) (int, error) {
			ownerCalls++
			if ownerCalls == 3 {
				if err := ctx.Err(); err != nil {
					t.Fatalf("final owner lookup used expired context: %v", err)
				}
				if _, ok := ctx.Deadline(); !ok {
					t.Fatal("final owner lookup context is not bounded")
				}
				if remaining := time.Until(mustDeadline(t, ctx)); remaining <= 0 || remaining > pollInterval {
					t.Fatalf("final owner lookup budget = %s, want 0 < budget <= %s", remaining, pollInterval)
				}
			}
			return target.PID, nil
		},
		SignalPID: func(int) error {
			timer := time.NewTimer(time.Until(parentDeadline) - releaseTimeout)
			defer timer.Stop()
			<-timer.C
			return nil
		},
		Sleep: func(ctx context.Context, _ time.Duration) error {
			<-ctx.Done()
			return ctx.Err()
		},
		Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}

	parent, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	parentDeadline = mustDeadline(t, parent)
	if _, err := service.ForceTerminate(parent, inspection.ConfirmationToken); !errors.Is(err, ErrPortReleaseTimeout) {
		t.Fatalf("error = %v, want %v", err, ErrPortReleaseTimeout)
	}
	if ownerCalls != 3 {
		t.Fatalf("owner calls = %d, want 3", ownerCalls)
	}
}

func mustDeadline(t *testing.T, ctx context.Context) time.Time {
	t.Helper()
	deadline, ok := ctx.Deadline()
	if !ok {
		t.Fatal("context has no deadline")
	}
	return deadline
}

func TestServiceRejectsPIDOnlyWithoutExplicitSupport(t *testing.T) {
	target := testPIDOnlyTarget()
	fixture := &pidOnlyFixture{}
	service := newPIDOnlyService(t, Config{ListenAddr: target.ListenAddr}, target, fixture)

	inspection, err := service.Inspect(context.Background())
	if err != nil || inspection.Recovery.Action != RecoveryActionUnavailable || inspection.Recovery.Reason != RecoveryReasonIdentityUnavailable || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
		t.Fatalf("inspection = %+v, error = %v", inspection, err)
	}
	if fixture.ownerCalls != 0 || len(fixture.signals) != 0 {
		t.Fatalf("owner calls=%d signals=%v", fixture.ownerCalls, fixture.signals)
	}
}

func TestServiceRejectsInvalidPIDOnlyTargets(t *testing.T) {
	for _, test := range []struct {
		name   string
		mutate func(*Target)
	}{
		{name: "missing PID", mutate: func(target *Target) { target.PID = 0 }},
		{name: "changed address", mutate: func(target *Target) { target.ListenAddr = "127.0.0.1:19100" }},
	} {
		t.Run(test.name, func(t *testing.T) {
			target := testPIDOnlyTarget()
			listenAddr := target.ListenAddr
			test.mutate(&target)
			fixture := &pidOnlyFixture{supports: true}
			service := newPIDOnlyService(t, Config{ListenAddr: listenAddr}, target, fixture)

			inspection, err := service.Inspect(context.Background())
			if err != nil || inspection.Recovery.Action != RecoveryActionUnavailable || inspection.Recovery.Reason != RecoveryReasonIdentityUnavailable || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
				t.Fatalf("inspection = %+v, error = %v", inspection, err)
			}
			if fixture.ownerCalls != 0 || len(fixture.signals) != 0 {
				t.Fatalf("owner calls=%d signals=%v", fixture.ownerCalls, fixture.signals)
			}
		})
	}
}

func TestServiceDiagnosesProtectedPIDOnlyTargets(t *testing.T) {
	for _, test := range []struct {
		name   string
		config func(Target) Config
	}{
		{name: "desktop", config: func(target Target) Config { return Config{ListenAddr: target.ListenAddr, DesktopPID: target.PID} }},
		{name: "manager", config: func(target Target) Config {
			return Config{ListenAddr: target.ListenAddr, ManagerIdentity: process.Identity{PID: target.PID}}
		}},
		{name: "protected PID", config: func(target Target) Config {
			return Config{ListenAddr: target.ListenAddr, IsProtectedPID: func(pid int) bool { return pid == target.PID }}
		}},
	} {
		t.Run(test.name, func(t *testing.T) {
			target := testPIDOnlyTarget()
			fixture := &pidOnlyFixture{supports: true}
			service := newPIDOnlyService(t, test.config(target), target, fixture)
			inspection, err := service.Inspect(context.Background())
			if err != nil || inspection.Recovery.Action != RecoveryActionUnavailable || inspection.Recovery.Reason != RecoveryReasonProtectedProcess || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
				t.Fatalf("inspection = %+v, error = %v", inspection, err)
			}
			if fixture.ownerCalls != 0 || len(fixture.signals) != 0 {
				t.Fatalf("owner calls=%d signals=%v", fixture.ownerCalls, fixture.signals)
			}
		})
	}
}

type pidOwnerResult struct {
	pid int
	err error
}

type pidOnlyFixture struct {
	owners     []pidOwnerResult
	signals    []int
	ownerCalls int
	supports   bool
	protected  bool
	signalErr  error
	sleepErr   error
	now        time.Time
	discovery  discovery.Classification
}

func newPIDOnlyService(t *testing.T, config Config, target Target, fixture *pidOnlyFixture) *Service {
	t.Helper()
	if fixture.now.IsZero() {
		fixture.now = time.Date(2026, 7, 22, 1, 2, 3, 0, time.UTC)
	}
	return New(config, Dependencies{
		Discover: func(context.Context) discovery.Result {
			classification := fixture.discovery
			if classification == "" {
				classification = discovery.UnknownOccupant
			}
			return discovery.Result{Classification: classification}
		},
		Inspect:         func(context.Context, string) (Target, error) { return target, nil },
		SupportsPIDOnly: func() bool { return fixture.supports },
		InspectPIDOwner: func(_ context.Context, listenAddr string) (int, error) {
			if listenAddr != target.ListenAddr {
				t.Fatalf("owner lookup address = %q, want %q", listenAddr, target.ListenAddr)
			}
			fixture.ownerCalls++
			if len(fixture.owners) == 0 {
				return 0, ErrIdentityUnavailable
			}
			result := fixture.owners[0]
			fixture.owners = fixture.owners[1:]
			return result.pid, result.err
		},
		SignalPID: func(pid int) error {
			fixture.signals = append(fixture.signals, pid)
			return fixture.signalErr
		},
		Sleep: func(context.Context, time.Duration) error {
			if fixture.sleepErr != nil {
				return fixture.sleepErr
			}
			return nil
		},
		Random: bytes.NewReader(make([]byte, 32)),
		Now:    func() time.Time { return fixture.now },
	})
}

func equalInts(left, right []int) bool {
	if len(left) != len(right) {
		return false
	}
	for i := range left {
		if left[i] != right[i] {
			return false
		}
	}
	return true
}

type stubConn struct{ net.Conn }

func (stubConn) Close() error { return nil }

func testIdentity() Identity {
	return Identity{ListenAddr: "127.0.0.1:19099", Network: "tcp4", SocketID: "socket", Process: process.Identity{PID: 42, StartedAt: "start", Executable: "/tmp/listener"}, UserID: "user"}
}

func verifiedTarget(identity Identity) Target {
	return Target{Mode: VerificationModeVerifiedIdentity, Identity: identity, PID: identity.Process.PID, ListenAddr: identity.ListenAddr}
}

func testPIDOnlyTarget() Target {
	return Target{Mode: VerificationModeWindowsPIDOnly, PID: 4242, ListenAddr: "127.0.0.1:19099"}
}
