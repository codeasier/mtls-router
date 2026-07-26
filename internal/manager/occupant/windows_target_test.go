package occupant

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

func TestInspectWindowsTargetCompleteFirst(t *testing.T) {
	target, err := inspectWindowsTarget(context.Background(), "127.0.0.1:19099", windowsTargetDependencies{
		inspectPIDOwner:  func(context.Context, string) (int, error) { return 42, nil },
		servicesForPID:   func(int) ([]string, error) { return nil, nil },
		processSessionID: func(int) (uint32, error) { return 1, nil },
		processSID:       func(int) (string, error) { return "S-1-5-21-1", nil },
		currentSID:       func() (string, error) { return "S-1-5-21-1", nil },
		inspectProcess: func(int) (process.Identity, error) {
			return process.Identity{PID: 42, StartedAt: "start", Executable: `C:\router.exe`}, nil
		},
		preflightTerminate: func(int) error { return nil },
	})
	if err != nil {
		t.Fatal(err)
	}
	if target.Mode != VerificationModeVerifiedIdentity || target.PID != 42 || target.ListenAddr != "127.0.0.1:19099" {
		t.Fatalf("target = %+v", target)
	}
	if target.Identity.Process.PID != 42 || target.Identity.UserID != "S-1-5-21-1" || target.Identity.SocketID == "" {
		t.Fatalf("identity = %+v", target.Identity)
	}
}

func TestInspectWindowsTargetClassifiesServiceOwnerAndPrivilege(t *testing.T) {
	readErr := errors.New("unreadable")
	oversizedServices := make([]string, 17)
	for index := range oversizedServices {
		oversizedServices[index] = fmt.Sprintf("Service%02d", index)
	}
	tests := []struct {
		name             string
		services         []string
		servicesErr      error
		sessionID        uint32
		sessionErr       error
		processSID       string
		processSIDErr    error
		currentSID       string
		preflight        error
		wantReason       RecoveryReason
		wantMode         VerificationMode
		wantSupervisor   []string
		wantSessionCalls int
		wantSIDCalls     int
		wantProcessCalls int
		wantPreflights   int
	}{
		{name: "shared service", services: []string{"Beta", "Alpha", "Beta"}, processSID: "S-1-5-18", currentSID: "S-1-5-21-1", wantReason: RecoveryReasonServiceManaged, wantMode: VerificationModeWindowsPIDOnly, wantSupervisor: []string{"Alpha", "Beta"}},
		{name: "oversized service metadata", services: oversizedServices, wantReason: RecoveryReasonServiceManaged, wantMode: VerificationModeWindowsPIDOnly},
		{name: "SCM unavailable continues in user session", servicesErr: readErr, sessionID: 2, processSID: "S-1-5-21-1", currentSID: "S-1-5-21-1", wantMode: VerificationModeVerifiedIdentity, wantSessionCalls: 1, wantSIDCalls: 2, wantProcessCalls: 1, wantPreflights: 1},
		{name: "Session 0 without service names", sessionID: 0, wantReason: RecoveryReasonIdentityUnavailable, wantMode: VerificationModeWindowsPIDOnly, wantSessionCalls: 1},
		{name: "session unavailable", sessionErr: readErr, wantReason: RecoveryReasonIdentityUnavailable, wantMode: VerificationModeWindowsPIDOnly, wantSessionCalls: 1},
		{name: "other SID", sessionID: 1, processSID: "S-1-5-21-2", currentSID: "S-1-5-21-1", wantReason: RecoveryReasonDifferentUser, wantMode: VerificationModeWindowsPIDOnly, wantSessionCalls: 1, wantSIDCalls: 2},
		{name: "same SID elevated", sessionID: 1, processSID: "S-1-5-21-1", currentSID: "S-1-5-21-1", preflight: ErrPermissionDenied, wantReason: RecoveryReasonInsufficientPrivilege, wantMode: VerificationModeVerifiedIdentity, wantSessionCalls: 1, wantSIDCalls: 2, wantProcessCalls: 1, wantPreflights: 1},
		{name: "unreadable process SID but terminable", sessionID: 1, processSIDErr: readErr, preflight: nil, wantMode: VerificationModeWindowsPIDOnly, wantSessionCalls: 1, wantSIDCalls: 1, wantPreflights: 1},
		{name: "unreadable current SID but denied", sessionID: 1, processSID: "S-1-5-21-1", currentSID: "", preflight: ErrPermissionDenied, wantReason: RecoveryReasonInsufficientPrivilege, wantMode: VerificationModeWindowsPIDOnly, wantSessionCalls: 1, wantSIDCalls: 2, wantPreflights: 1},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			sidCalls := 0
			sessionCalls := 0
			processCalls := 0
			preflights := 0
			target, err := inspectWindowsTarget(context.Background(), "127.0.0.1:19099", windowsTargetDependencies{
				inspectPIDOwner: func(context.Context, string) (int, error) { return 42, nil },
				servicesForPID:  func(int) ([]string, error) { return test.services, test.servicesErr },
				processSessionID: func(int) (uint32, error) {
					sessionCalls++
					return test.sessionID, test.sessionErr
				},
				processSID: func(int) (string, error) {
					sidCalls++
					return test.processSID, test.processSIDErr
				},
				currentSID: func() (string, error) {
					sidCalls++
					return test.currentSID, nil
				},
				inspectProcess: func(pid int) (process.Identity, error) {
					processCalls++
					return completeWindowsProcess(pid)
				},
				preflightTerminate: func(int) error {
					preflights++
					return test.preflight
				},
			})
			if err != nil {
				t.Fatal(err)
			}
			if target.Mode != test.wantMode || target.BlockReason != test.wantReason {
				t.Fatalf("target = %+v, want mode %q reason %q", target, test.wantMode, test.wantReason)
			}
			if target.Supervisor != nil {
				if target.Supervisor.Kind != SupervisorWindowsService || target.Supervisor.Scope != SupervisorScopeSystem || !equalStrings(target.Supervisor.Identifiers, test.wantSupervisor) {
					t.Fatalf("supervisor = %+v, want identifiers %v", target.Supervisor, test.wantSupervisor)
				}
			} else if test.wantSupervisor != nil {
				t.Fatalf("supervisor = nil, want identifiers %v", test.wantSupervisor)
			}
			if sessionCalls != test.wantSessionCalls || sidCalls != test.wantSIDCalls || processCalls != test.wantProcessCalls || preflights != test.wantPreflights {
				t.Fatalf("calls: session=%d SID=%d process=%d preflight=%d, want %d/%d/%d/%d", sessionCalls, sidCalls, processCalls, preflights, test.wantSessionCalls, test.wantSIDCalls, test.wantProcessCalls, test.wantPreflights)
			}
		})
	}
}

func TestNormalizeWindowsServices(t *testing.T) {
	seventeen := make([]string, 17)
	for index := range seventeen {
		seventeen[index] = fmt.Sprintf("Service%02d", index)
	}
	largeStructure := make([]string, 16)
	for index := range largeStructure {
		largeStructure[index] = fmt.Sprintf("%02d%s", index, strings.Repeat("x", 254))
	}
	tests := []struct {
		name   string
		values []string
		want   []string
		ok     bool
	}{
		{name: "sorted and deduplicated", values: []string{"Beta", "Alpha", "Beta"}, want: []string{"Alpha", "Beta"}, ok: true},
		{name: "empty set", values: nil},
		{name: "empty identifier", values: []string{""}},
		{name: "too many", values: seventeen},
		{name: "identifier over UTF-8 byte limit", values: []string{strings.Repeat("\xc3\xa9", 129)}},
		{name: "invalid UTF-8", values: []string{string([]byte{0xff})}},
		{name: "structure over limit", values: largeStructure},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, ok := normalizeSupervisorIdentifiers(test.values)
			if ok != test.ok || !equalStrings(got, test.want) {
				t.Fatalf("normalizeSupervisorIdentifiers() = (%v, %t), want (%v, %t)", got, ok, test.want, test.ok)
			}
		})
	}
}

func TestInspectWindowsTargetDegradesOnlyAfterExactOwner(t *testing.T) {
	readErr := errors.New("unreadable")
	tests := []struct {
		name           string
		processSID     func(int) (string, error)
		currentSID     func() (string, error)
		inspectProcess func(int) (process.Identity, error)
	}{
		{
			name:           "process SID unavailable",
			processSID:     func(int) (string, error) { return "", readErr },
			currentSID:     func() (string, error) { return "S-1-5-21-1", nil },
			inspectProcess: completeWindowsProcess,
		},
		{
			name:           "current SID unavailable",
			processSID:     func(int) (string, error) { return "S-1-5-21-1", nil },
			currentSID:     func() (string, error) { return "", readErr },
			inspectProcess: completeWindowsProcess,
		},
		{
			name:           "process identity unavailable",
			processSID:     func(int) (string, error) { return "S-1-5-21-1", nil },
			currentSID:     func() (string, error) { return "S-1-5-21-1", nil },
			inspectProcess: func(int) (process.Identity, error) { return process.Identity{}, readErr },
		},
		{
			name:           "process identity incomplete",
			processSID:     func(int) (string, error) { return "S-1-5-21-1", nil },
			currentSID:     func() (string, error) { return "S-1-5-21-1", nil },
			inspectProcess: func(pid int) (process.Identity, error) { return process.Identity{PID: pid}, nil },
		},
		{
			name:           "other user SID",
			processSID:     func(int) (string, error) { return "S-1-5-21-2", nil },
			currentSID:     func() (string, error) { return "S-1-5-21-1", nil },
			inspectProcess: completeWindowsProcess,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			target, err := inspectWindowsTarget(context.Background(), "127.0.0.1:19099", windowsTargetDependencies{
				inspectPIDOwner:    func(context.Context, string) (int, error) { return 42, nil },
				servicesForPID:     func(int) ([]string, error) { return nil, nil },
				processSessionID:   func(int) (uint32, error) { return 1, nil },
				processSID:         test.processSID,
				currentSID:         test.currentSID,
				inspectProcess:     test.inspectProcess,
				preflightTerminate: func(int) error { return nil },
			})
			if err != nil {
				t.Fatal(err)
			}
			if target.Mode != VerificationModeWindowsPIDOnly || target.PID != 42 || target.ListenAddr != "127.0.0.1:19099" || target.Identity != (Identity{}) {
				t.Fatalf("target = %+v", target)
			}
		})
	}
}

func TestInspectWindowsTargetDoesNotDegradeOwnerErrors(t *testing.T) {
	for _, ownerErr := range []error{ErrNotFound, ErrIdentityUnavailable} {
		t.Run(ownerErr.Error(), func(t *testing.T) {
			metadataRead := false
			_, err := inspectWindowsTarget(context.Background(), "127.0.0.1:19099", windowsTargetDependencies{
				inspectPIDOwner:  func(context.Context, string) (int, error) { return 0, ownerErr },
				servicesForPID:   func(int) ([]string, error) { return nil, nil },
				processSessionID: func(int) (uint32, error) { return 1, nil },
				processSID: func(int) (string, error) {
					metadataRead = true
					return "", nil
				},
				currentSID:         func() (string, error) { return "", nil },
				inspectProcess:     completeWindowsProcess,
				preflightTerminate: func(int) error { return nil },
			})
			if !errors.Is(err, ownerErr) {
				t.Fatalf("error = %v, want %v", err, ownerErr)
			}
			if metadataRead {
				t.Fatal("read process metadata without an exact owner")
			}
		})
	}
}

func TestInspectWindowsTargetCancellationNeverMintsTarget(t *testing.T) {
	tests := []struct {
		name       string
		cancelStep string
	}{
		{name: "before owner lookup", cancelStep: "before"},
		{name: "after owner lookup", cancelStep: "owner"},
		{name: "after service lookup", cancelStep: "services"},
		{name: "after session lookup", cancelStep: "session"},
		{name: "after process SID", cancelStep: "process SID"},
		{name: "after current SID", cancelStep: "current SID"},
		{name: "after process identity", cancelStep: "process identity"},
		{name: "after terminate preflight", cancelStep: "preflight"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(context.Background())
			if test.cancelStep == "before" {
				cancel()
			} else {
				defer cancel()
			}
			ownerCalls := 0
			metadataCalls := 0
			target, err := inspectWindowsTarget(ctx, "127.0.0.1:19099", windowsTargetDependencies{
				inspectPIDOwner: func(context.Context, string) (int, error) {
					ownerCalls++
					if test.cancelStep == "owner" {
						cancel()
					}
					return 42, nil
				},
				servicesForPID: func(int) ([]string, error) {
					metadataCalls++
					if test.cancelStep == "services" {
						cancel()
					}
					return nil, nil
				},
				processSessionID: func(int) (uint32, error) {
					metadataCalls++
					if test.cancelStep == "session" {
						cancel()
					}
					return 1, nil
				},
				processSID: func(int) (string, error) {
					metadataCalls++
					if test.cancelStep == "process SID" {
						cancel()
					}
					return "S-1-5-21-1", nil
				},
				currentSID: func() (string, error) {
					metadataCalls++
					if test.cancelStep == "current SID" {
						cancel()
					}
					return "S-1-5-21-1", nil
				},
				inspectProcess: func(pid int) (process.Identity, error) {
					metadataCalls++
					if test.cancelStep == "process identity" {
						cancel()
					}
					return completeWindowsProcess(pid)
				},
				preflightTerminate: func(int) error {
					metadataCalls++
					if test.cancelStep == "preflight" {
						cancel()
					}
					return nil
				},
			})
			if !errors.Is(err, ErrIdentityUnavailable) {
				t.Fatalf("error = %v, want %v", err, ErrIdentityUnavailable)
			}
			if target != (Target{}) {
				t.Fatalf("target = %+v, want empty", target)
			}
			if test.cancelStep == "before" && ownerCalls != 0 {
				t.Fatalf("owner calls = %d, want 0", ownerCalls)
			}
			if (test.cancelStep == "before" || test.cancelStep == "owner") && metadataCalls != 0 {
				t.Fatalf("metadata calls = %d, want 0", metadataCalls)
			}
		})
	}
}

func TestWindowsPIDRange(t *testing.T) {
	tests := []struct {
		name    string
		pid     int
		want    uint32
		wantErr error
	}{
		{name: "negative", pid: -1, wantErr: process.ErrNotFound},
		{name: "zero", pid: 0, wantErr: process.ErrNotFound},
		{name: "valid", pid: 42, want: 42},
	}
	maxInt := int(^uint(0) >> 1)
	if uint64(maxInt) > uint64(^uint32(0)) {
		tests = append(tests, struct {
			name    string
			pid     int
			want    uint32
			wantErr error
		}{name: "uint32 overflow", pid: maxInt, wantErr: process.ErrNotFound})
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got, err := windowsPID(test.pid)
			if !errors.Is(err, test.wantErr) || got != test.want {
				t.Fatalf("windowsPID(%d) = (%d, %v), want (%d, %v)", test.pid, got, err, test.want, test.wantErr)
			}
		})
	}
}

func completeWindowsProcess(pid int) (process.Identity, error) {
	return process.Identity{PID: pid, StartedAt: "start", Executable: `C:\router.exe`}, nil
}

func equalStrings(left, right []string) bool {
	if len(left) != len(right) {
		return false
	}
	for index := range left {
		if left[index] != right[index] {
			return false
		}
	}
	return true
}
