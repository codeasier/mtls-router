package occupant

import (
	"context"
	"errors"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

func TestInspectWindowsTargetCompleteFirst(t *testing.T) {
	target, err := inspectWindowsTarget(context.Background(), "127.0.0.1:19099", windowsTargetDependencies{
		inspectPIDOwner: func(context.Context, string) (int, error) { return 42, nil },
		processSID:      func(int) (string, error) { return "S-1-5-21-1", nil },
		currentSID:      func() (string, error) { return "S-1-5-21-1", nil },
		inspectProcess: func(int) (process.Identity, error) {
			return process.Identity{PID: 42, StartedAt: "start", Executable: `C:\router.exe`}, nil
		},
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
				inspectPIDOwner: func(context.Context, string) (int, error) { return 42, nil },
				processSID:      test.processSID,
				currentSID:      test.currentSID,
				inspectProcess:  test.inspectProcess,
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
				inspectPIDOwner: func(context.Context, string) (int, error) { return 0, ownerErr },
				processSID: func(int) (string, error) {
					metadataRead = true
					return "", nil
				},
				currentSID:     func() (string, error) { return "", nil },
				inspectProcess: completeWindowsProcess,
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
		{name: "after process SID", cancelStep: "process SID"},
		{name: "after current SID", cancelStep: "current SID"},
		{name: "after process identity", cancelStep: "process identity"},
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
