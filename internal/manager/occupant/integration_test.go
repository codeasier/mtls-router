//go:build darwin || linux || windows

package occupant

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
)

func TestNativeInspectOwnLoopbackListener(t *testing.T) {
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	service := New(Config{ListenAddr: listener.Addr().String()}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
	})
	inspection, err := service.Inspect(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if inspection.VerificationMode != VerificationModeVerifiedIdentity {
		t.Fatalf("mode = %q", inspection.VerificationMode)
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	executable, err = process.NormalizeExecutable(executable)
	if err != nil {
		t.Fatal(err)
	}
	if inspection.PID != os.Getpid() || inspection.ListenAddr != listener.Addr().String() || inspection.ProcessName != filepath.Base(executable) || inspection.Executable != executable {
		t.Fatalf("inspection = %+v", inspection)
	}
	if inspection.Supervisor != nil {
		if runtime.GOOS != "linux" || !validSupervisor(inspection.Supervisor) {
			t.Fatalf("unexpected native supervisor = %+v", inspection.Supervisor)
		}
		switch inspection.Supervisor.Kind {
		case SupervisorSystemdUser:
			if inspection.Supervisor.Scope != SupervisorScopeUser {
				t.Fatalf("supervisor = %+v", inspection.Supervisor)
			}
		case SupervisorSystemdSystem:
			if inspection.Supervisor.Scope != SupervisorScopeSystem {
				t.Fatalf("supervisor = %+v", inspection.Supervisor)
			}
		default:
			t.Fatalf("unexpected Linux supervisor = %+v", inspection.Supervisor)
		}
		if inspection.Recovery != (Recovery{Action: RecoveryActionManualStopRequired, Reason: RecoveryReasonServiceManaged}) || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
			t.Fatalf("supervised inspection shape = %+v", inspection)
		}
		return
	}
	if inspection.Recovery != (Recovery{Action: RecoveryActionForceTerminate}) || inspection.ConfirmationToken == "" || inspection.ExpiresAt == nil || !inspection.ExpiresAt.After(time.Now()) {
		t.Fatalf("forceable inspection token invariant failed: %+v", inspection)
	}
}
