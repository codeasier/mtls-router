package occupant

import (
	"context"
	"errors"
	"fmt"
	"sort"

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
	inspectPIDOwner    func(context.Context, string) (int, error)
	servicesForPID     func(int) ([]string, error)
	processSessionID   func(int) (uint32, error)
	processSID         func(int) (string, error)
	currentSID         func() (string, error)
	inspectProcess     func(int) (process.Identity, error)
	preflightTerminate func(int) error
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
	// SCM enumeration is advisory because inaccessible services may be omitted;
	// the session check prevents unsafe authorization without inferring ownership.
	services, _ := deps.servicesForPID(pid)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if len(services) > 0 {
		identifiers, ok := normalizeSupervisorIdentifiers(services)
		if !ok {
			degraded.BlockReason = RecoveryReasonServiceManaged
			return degraded, nil
		}
		degraded.Supervisor = &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: identifiers}
		degraded.BlockReason = RecoveryReasonServiceManaged
		return degraded, nil
	}
	sessionID, sessionErr := deps.processSessionID(pid)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if sessionErr != nil {
		degraded.BlockReason = RecoveryReasonIdentityUnavailable
		return degraded, nil
	}
	if sessionID == 0 {
		degraded.BlockReason = RecoveryReasonIdentityUnavailable
		return degraded, nil
	}
	processSID, err := deps.processSID(pid)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if err != nil || processSID == "" {
		return preflightWindowsTarget(ctx, degraded, deps.preflightTerminate)
	}
	currentSID, err := deps.currentSID()
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if err != nil || currentSID == "" {
		return preflightWindowsTarget(ctx, degraded, deps.preflightTerminate)
	}
	if processSID != currentSID {
		degraded.BlockReason = RecoveryReasonDifferentUser
		return degraded, nil
	}
	identity, err := deps.inspectProcess(pid)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if err != nil || identity.PID != pid || identity.StartedAt == "" || identity.Executable == "" {
		return preflightWindowsTarget(ctx, degraded, deps.preflightTerminate)
	}
	complete := Identity{
		ListenAddr: listenAddr,
		Network:    "tcp4",
		SocketID:   windowsSocketID(listenAddr, pid),
		Process:    identity,
		UserID:     processSID,
	}
	target := Target{
		Mode:       VerificationModeVerifiedIdentity,
		Identity:   complete,
		PID:        pid,
		ListenAddr: listenAddr,
	}
	return preflightWindowsTarget(ctx, target, deps.preflightTerminate)
}

func preflightWindowsTarget(ctx context.Context, target Target, preflight func(int) error) (Target, error) {
	err := preflight(target.PID)
	if ctx.Err() != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if errors.Is(err, ErrPermissionDenied) {
		target.BlockReason = RecoveryReasonInsufficientPrivilege
	} else if err != nil {
		target.BlockReason = RecoveryReasonIdentityUnavailable
	}
	return target, nil
}

func normalizeSupervisorIdentifiers(values []string) ([]string, bool) {
	if len(values) == 0 || len(values) > maxSupervisorIdentifiers {
		return nil, false
	}
	seen := make(map[string]struct{}, len(values))
	for _, value := range values {
		seen[value] = struct{}{}
	}
	result := make([]string, 0, len(seen))
	for value := range seen {
		result = append(result, value)
	}
	sort.Strings(result)
	supervisor := &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: result}
	if !validSupervisor(supervisor) {
		return nil, false
	}
	return result, true
}
