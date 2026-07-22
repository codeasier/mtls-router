package occupant

import (
	"context"
	"fmt"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

func windowsSocketID(listenAddr string, pid int) string {
	return fmt.Sprintf("tcp4:%s:%d", listenAddr, pid)
}

func windowsPID(pid int) (uint32, error) {
	if pid <= 0 || uint64(pid) > uint64(^uint32(0)) {
		return 0, process.ErrNotFound
	}
	return uint32(pid), nil
}

type windowsTargetDependencies struct {
	inspectPIDOwner func(context.Context, string) (int, error)
	processSID      func(int) (string, error)
	currentSID      func() (string, error)
	inspectProcess  func(int) (process.Identity, error)
}

func inspectWindowsTarget(ctx context.Context, listenAddr string, deps windowsTargetDependencies) (Target, error) {
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	pid, err := deps.inspectPIDOwner(ctx, listenAddr)
	if err != nil {
		return Target{}, err
	}
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	degraded := Target{Mode: VerificationModeWindowsPIDOnly, PID: pid, ListenAddr: listenAddr}
	processSID, err := deps.processSID(pid)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if err != nil || processSID == "" {
		return degraded, nil
	}
	currentSID, err := deps.currentSID()
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if err != nil || currentSID == "" {
		return degraded, nil
	}
	identity, err := deps.inspectProcess(pid)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if err != nil || identity.PID != pid || identity.StartedAt == "" || identity.Executable == "" || processSID != currentSID {
		return degraded, nil
	}
	complete := Identity{
		ListenAddr: listenAddr,
		Network:    "tcp4",
		SocketID:   windowsSocketID(listenAddr, pid),
		Process:    identity,
		UserID:     processSID,
	}
	return Target{
		Mode:       VerificationModeVerifiedIdentity,
		Identity:   complete,
		PID:        pid,
		ListenAddr: listenAddr,
	}, nil
}
