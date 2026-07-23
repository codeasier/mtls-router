// Package lifecycle starts and stops routers while preserving complete process
// identity and desktop ownership invariants.
package lifecycle

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/codeasier/mtls-router/internal/background"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
	"github.com/codeasier/mtls-router/internal/version"
)

type Error struct {
	Code         protocol.ErrorCode
	Err          error
	Launched     bool
	RecentOutput string
}

func (e *Error) Error() string { return fmt.Sprintf("%s: %v", e.Code, e.Err) }
func (e *Error) Unwrap() error { return e.Err }

// UnexpectedExit describes a verified desktop child that terminated without
// an explicit stop. RecentOutput is bounded but remains unsanitized until the
// app protocol boundary.
type UnexpectedExit struct {
	Identity     process.Identity
	RecentOutput string
	Err          error
}

type Config struct {
	RouterPath                string
	ListenAddr                string
	DesktopStatePath          string
	CLIStatePath              string
	DesktopLockPath           string
	DesktopLogPath            string
	CLILogPath                string
	SessionID                 string
	ManagerIdentity           process.Identity
	ParentIdentity            process.Identity
	ManagerVersion            string
	DeploymentID              string
	ManagementProtocolVersion string
	StartupTimeout            time.Duration
	StopTimeout               time.Duration
	ForceTimeout              time.Duration
	PollInterval              time.Duration
	RecentOutputBytes         int
}

type Dependencies struct {
	Discover       func(context.Context) discovery.Result
	Inspect        func(int) (process.Identity, error)
	Validate       func(process.Identity, string) (process.Status, error)
	Signal         func(process.Identity, string, os.Signal) error
	LaunchDesktop  func(string, []string, []string, io.Writer) (foregroundProcess, error)
	LaunchDetached func(string, []string, string) (int, error)
	ReadState      func(string) (state.RouterState, error)
	WriteState     func(string, state.RouterState) error
	RemoveState    func(string) error
	AcquireLock    func(string) (io.Closer, error)
	Environ        func() []string
	OpenLog        func(string) (*os.File, error)
	Verify         func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error)
	Sleep          func(context.Context, time.Duration) error
	Now            func() time.Time
}

type Manager struct {
	config Config
	deps   Dependencies

	mu          sync.Mutex
	operationMu sync.Mutex
	lock        io.Closer
	desktopRun  *desktopRun
	cleanupRun  *desktopRun
	recent      *boundedOutput
	exitCh      chan UnexpectedExit
}

type desktopRun struct {
	identity     process.Identity
	done         chan struct{}
	err          error
	recentOutput string
	logBaseline  os.FileInfo
	inherited    *boundedOutput
	intentional  bool
}

func New(config Config, deps Dependencies) *Manager {
	if config.ListenAddr == "" {
		config.ListenAddr = "127.0.0.1:19099"
	}
	if config.ManagerVersion == "" {
		config.ManagerVersion = version.Version
	}
	if config.DeploymentID == "" {
		config.DeploymentID = version.DeploymentID
	}
	if config.ManagementProtocolVersion == "" {
		config.ManagementProtocolVersion = version.ManagementProtocolVersion
	}
	if config.StartupTimeout <= 0 {
		config.StartupTimeout = 10 * time.Second
	}
	if config.StopTimeout <= 0 {
		config.StopTimeout = 5 * time.Second
	}
	if config.ForceTimeout <= 0 {
		config.ForceTimeout = 2 * time.Second
	}
	if config.PollInterval <= 0 {
		config.PollInterval = 100 * time.Millisecond
	}
	if deps.Inspect == nil {
		deps.Inspect = process.Inspect
	}
	if deps.Validate == nil {
		deps.Validate = process.Validate
	}
	if deps.Signal == nil {
		deps.Signal = process.Signal
	}
	if deps.LaunchDesktop == nil {
		deps.LaunchDesktop = launchForeground
	}
	if deps.LaunchDetached == nil {
		deps.LaunchDetached = background.Start
	}
	if deps.ReadState == nil {
		deps.ReadState = state.Read
	}
	if deps.WriteState == nil {
		deps.WriteState = state.Write
	}
	if deps.RemoveState == nil {
		deps.RemoveState = os.Remove
	}
	if deps.AcquireLock == nil {
		deps.AcquireLock = func(path string) (io.Closer, error) { return state.AcquireLock(path) }
	}
	if deps.Environ == nil {
		deps.Environ = os.Environ
	}
	if deps.OpenLog == nil {
		deps.OpenLog = background.OpenLogFile
	}
	if deps.Verify == nil {
		deps.Verify = verifyEndpoints
	}
	if deps.Sleep == nil {
		deps.Sleep = sleepContext
	}
	if deps.Now == nil {
		deps.Now = time.Now
	}
	return &Manager{config: config, deps: deps, recent: newBoundedOutput(config.RecentOutputBytes), exitCh: make(chan UnexpectedExit, 1)}
}

func (m *Manager) Start(ctx context.Context, owner protocol.RouterOwner) (state.RouterState, *Error) {
	m.operationMu.Lock()
	defer m.operationMu.Unlock()
	switch owner {
	case protocol.RouterOwnerDesktop:
		return m.startDesktop(ctx)
	case protocol.RouterOwnerCLI:
		return m.startCLI(ctx)
	default:
		return state.RouterState{}, lifecycleError(protocol.CodeInvalidParams, "owner must be desktop or cli")
	}
}

func (m *Manager) startDesktop(ctx context.Context) (state.RouterState, *Error) {
	if !completeIdentity(m.config.ManagerIdentity) || !completeIdentity(m.config.ParentIdentity) || m.config.SessionID == "" {
		return state.RouterState{}, lifecycleError(protocol.CodeInvalidParams, "complete session, manager, and parent identity are required")
	}
	pending, err := m.acquireDesktopStartLock()
	if err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "desktop ownership is locked")
	}
	if pending {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "previous desktop router cleanup is pending")
	}
	keepLock := false
	defer func() {
		if !keepLock {
			m.releaseLock()
		}
	}()
	if existing, err := m.deps.ReadState(m.config.DesktopStatePath); err == nil && existing.Owner == "desktop" {
		status, _ := m.deps.Validate(routerIdentity(existing), existing.BinaryPath)
		if status == process.StatusGenuine {
			if completeDesktopState(existing) && existing.DesktopSessionID == m.config.SessionID && managerMatches(existing, m.config.ManagerIdentity) && existing.DeploymentID == m.config.DeploymentID && existing.ManagementProtocolVersion == m.config.ManagementProtocolVersion {
				keepLock = true
				return existing, nil
			}
			return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "a desktop router is already owned")
		}
	}
	if m.deps.Discover != nil {
		found := m.deps.Discover(ctx)
		switch found.Classification {
		case discovery.ExternalCompatible:
			m.releaseLock()
			return found.State, nil
		case discovery.Degraded:
			if found.Owner == "cli" {
				m.releaseLock()
				return found.State, &Error{Code: protocol.CodeRouterDegraded, Err: errors.New("external router has unavailable endpoint or upstream")}
			}
			return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "a desktop router is already running but degraded")
		case discovery.UnknownOccupant:
			return state.RouterState{}, lifecycleError(protocol.CodePortOccupied, "router port is occupied")
		case discovery.Stale:
			return state.RouterState{}, lifecycleError(protocol.CodeRouterStateStale, "router state is stale")
		case discovery.DesktopOwned:
			return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "a desktop router is already running")
		}
	}

	if err := os.MkdirAll(filepath.Dir(m.config.DesktopLogPath), 0o700); err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot create desktop log directory")
	}
	if err := os.Chmod(filepath.Dir(m.config.DesktopLogPath), 0o700); err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot restrict desktop log directory")
	}
	logFile, err := m.deps.OpenLog(m.config.DesktopLogPath)
	if err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot open desktop log")
	}
	logInfo, err := logFile.Stat()
	if err != nil {
		_ = logFile.Close()
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot inspect desktop log")
	}
	inherited := newBoundedOutput(m.config.RecentOutputBytes)
	output := io.MultiWriter(logFile, m.recent, inherited)
	args := []string{"-listen", m.config.ListenAddr, "-log", m.config.DesktopLogPath, "-tls-min", "tls1.2", "-timeout", "10s", "-debug=false"}
	child, err := m.deps.LaunchDesktop(m.config.RouterPath, args, background.DesktopChildEnv(m.deps.Environ()), output)
	if err != nil {
		_ = logFile.Close()
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "router launch failed")
	}
	run := &desktopRun{done: make(chan struct{}), logBaseline: logInfo, inherited: inherited}
	go func() {
		run.err = child.Wait()
		_ = logFile.Close()
		run.recentOutput = m.recentOutputForRun(run)
		close(run.done)
	}()

	identity, versionInfo, healthInfo, startErr := m.waitUntilReady(ctx, child.PID(), run.done)
	if startErr != nil {
		startErr, keepLock = m.cleanupFailedDesktopStart(child, run, startErr)
		return state.RouterState{}, startErr
	}
	value := m.routerState("desktop", identity, versionInfo)
	if err := m.deps.WriteState(m.config.DesktopStatePath, value); err != nil {
		startErr, pending := m.cleanupFailedDesktopStart(child, run, lifecycleError(protocol.CodeRouterStartFailed, "cannot persist verified router state"))
		keepLock = pending
		return state.RouterState{}, startErr
	}
	run.identity = identity
	m.mu.Lock()
	select {
	case <-run.done:
		m.mu.Unlock()
		_ = m.deps.RemoveState(m.config.DesktopStatePath)
		startErr, pending := m.cleanupFailedDesktopStart(child, run, lifecycleError(protocol.CodeRouterStartFailed, "router exited before startup completed"))
		keepLock = pending
		return state.RouterState{}, startErr
	default:
	}
	m.desktopRun = run
	m.mu.Unlock()
	go m.reportUnexpectedExit(run)
	keepLock = true
	if healthInfo.Status != "ok" {
		return value, &Error{Code: protocol.CodeRouterDegraded, Err: errors.New("router started with unavailable upstream")}
	}
	return value, nil
}

func (m *Manager) startCLI(ctx context.Context) (state.RouterState, *Error) {
	if m.deps.Discover != nil {
		found := m.deps.Discover(ctx)
		if found.Classification != discovery.Absent {
			return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "router endpoint is already in use")
		}
	}
	args := []string{"-listen", m.config.ListenAddr, "-log", m.config.CLILogPath}
	pid, err := m.deps.LaunchDetached(m.config.RouterPath, args, m.config.CLILogPath)
	if err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "detached router launch failed")
	}
	identity, versionInfo, healthInfo, startErr := m.waitUntilReady(ctx, pid, nil)
	if startErr != nil {
		m.cleanupFailedChild(identity)
		return state.RouterState{}, startErr
	}
	value := m.routerState("cli", identity, versionInfo)
	if err := m.deps.WriteState(m.config.CLIStatePath, value); err != nil {
		m.cleanupFailedChild(identity)
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot persist verified router state")
	}
	if healthInfo.Status != "ok" {
		return value, &Error{Code: protocol.CodeRouterDegraded, Err: errors.New("router started with unavailable upstream")}
	}
	return value, nil
}

func (m *Manager) waitUntilReady(ctx context.Context, pid int, childExit <-chan struct{}) (process.Identity, discovery.Version, discovery.Health, *Error) {
	identity, err := m.deps.Inspect(pid)
	if err != nil {
		return process.Identity{PID: pid}, discovery.Version{}, discovery.Health{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot inspect launched router")
	}
	startupCtx, cancel := context.WithTimeout(ctx, m.config.StartupTimeout)
	defer cancel()
	for {
		if childExit != nil {
			select {
			case <-childExit:
				return identity, discovery.Version{}, discovery.Health{}, lifecycleError(protocol.CodeRouterStartFailed, "router exited during startup")
			default:
			}
		}
		versionInfo, healthInfo, err := m.deps.Verify(startupCtx, "http://"+m.config.ListenAddr, pid, m.config.DeploymentID, m.config.ManagementProtocolVersion)
		if err == nil {
			status, validateErr := m.deps.Validate(identity, identity.Executable)
			if validateErr == nil && status == process.StatusGenuine {
				return identity, versionInfo, healthInfo, nil
			}
			return identity, discovery.Version{}, discovery.Health{}, lifecycleError(protocol.CodeRouterStateStale, "launched router identity changed")
		}
		if err := m.deps.Sleep(startupCtx, m.config.PollInterval); err != nil {
			return identity, discovery.Version{}, discovery.Health{}, lifecycleError(protocol.CodeRouterNotReady, "router did not become ready before timeout")
		}
	}
}

func (m *Manager) Stop(ctx context.Context) *Error {
	m.operationMu.Lock()
	defer m.operationMu.Unlock()
	return m.stop(ctx)
}

func (m *Manager) stop(ctx context.Context) *Error {
	statePath := m.config.DesktopStatePath
	desktop := m.config.SessionID != ""
	if !desktop {
		statePath = m.config.CLIStatePath
	}
	value, err := m.deps.ReadState(statePath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return lifecycleError(protocol.CodeRouterNotFound, "router state was not found")
		}
		return lifecycleError(protocol.CodeRouterStateStale, "router state is unreadable")
	}
	if desktop {
		if value.Owner != "desktop" {
			return lifecycleError(protocol.CodeRouterNotOwned, "router is not desktop-owned")
		}
		if value.DesktopSessionID != m.config.SessionID || !managerMatches(value, m.config.ManagerIdentity) {
			return lifecycleError(protocol.CodeRouterNotOwned, "desktop router belongs to another manager session")
		}
	} else if value.Owner != "cli" {
		return lifecycleError(protocol.CodeRouterNotOwned, "router is not CLI-owned")
	}
	status, validateErr := m.deps.Validate(routerIdentity(value), value.BinaryPath)
	if validateErr != nil || status != process.StatusGenuine {
		return lifecycleError(protocol.CodeRouterStateStale, "desktop router identity is stale")
	}
	identity := routerIdentity(value)
	run := m.markIntentionalStop(identity)
	if err := m.deps.Signal(identity, value.BinaryPath, gracefulSignal()); err != nil && !errors.Is(err, process.ErrNotFound) {
		m.cancelIntentionalStop(run)
		return lifecycleError(protocol.CodeRouterStateStale, "graceful stop identity validation failed")
	}
	stopCtx, cancel := context.WithTimeout(ctx, m.config.StopTimeout)
	defer cancel()
	for {
		status, _ = m.deps.Validate(identity, value.BinaryPath)
		if status == process.StatusAbsent {
			if err := m.deps.RemoveState(statePath); err != nil && !errors.Is(err, os.ErrNotExist) {
				return lifecycleError(protocol.CodeRouterStateStale, "stopped router state could not be removed")
			}
			m.releaseLock()
			return nil
		}
		if status == process.StatusStale {
			return lifecycleError(protocol.CodeRouterStateStale, "router identity changed while stopping")
		}
		if err := m.deps.Sleep(stopCtx, m.config.PollInterval); err != nil {
			break
		}
	}
	status, err = m.deps.Validate(identity, value.BinaryPath)
	if err != nil || status != process.StatusGenuine {
		return lifecycleError(protocol.CodeRouterStateStale, "router identity changed before force stop")
	}
	if err := m.deps.Signal(identity, value.BinaryPath, os.Kill); err != nil && !errors.Is(err, process.ErrNotFound) {
		return lifecycleError(protocol.CodeRouterStateStale, "force stop identity validation failed")
	}
	forceCtx, forceCancel := context.WithTimeout(context.Background(), m.config.ForceTimeout)
	defer forceCancel()
	for {
		status, _ = m.deps.Validate(identity, value.BinaryPath)
		if status == process.StatusAbsent {
			if err := m.deps.RemoveState(statePath); err != nil && !errors.Is(err, os.ErrNotExist) {
				return lifecycleError(protocol.CodeRouterStateStale, "stopped router state could not be removed")
			}
			m.releaseLock()
			return nil
		}
		if status == process.StatusStale {
			return lifecycleError(protocol.CodeRouterStateStale, "router identity changed after force stop")
		}
		if err := m.deps.Sleep(forceCtx, m.config.PollInterval); err != nil {
			return lifecycleError(protocol.CodeOperationTimeout, "router did not exit after force stop")
		}
	}
}

func (m *Manager) Reclaim() (state.RouterState, *Error) {
	m.operationMu.Lock()
	defer m.operationMu.Unlock()
	if !completeIdentity(m.config.ManagerIdentity) || m.config.SessionID == "" {
		return state.RouterState{}, lifecycleError(protocol.CodeInvalidParams, "complete replacement manager identity is required")
	}
	if err := m.acquireLock(); err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "previous manager still owns the lock")
	}
	success := false
	defer func() {
		if !success {
			m.releaseLock()
		}
	}()
	value, err := m.deps.ReadState(m.config.DesktopStatePath)
	if err != nil || value.Owner != "desktop" || value.DesktopSessionID != m.config.SessionID || !completeDesktopState(value) {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStateStale, "desktop state cannot be reclaimed")
	}
	previousManager := process.Identity{PID: value.ManagerPID, StartedAt: value.ManagerProcessStartedAt, Executable: value.ManagerProcessExecutable}
	managerStatus, managerErr := m.deps.Validate(previousManager, value.ManagerProcessExecutable)
	if managerErr != nil || managerStatus != process.StatusAbsent {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterAlreadyRunning, "previous manager identity is not absent")
	}
	routerStatus, err := m.deps.Validate(routerIdentity(value), value.BinaryPath)
	if err != nil || routerStatus != process.StatusGenuine || value.DeploymentID != m.config.DeploymentID || value.ManagementProtocolVersion != m.config.ManagementProtocolVersion {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStateStale, "router identity cannot be reclaimed")
	}
	value.ManagerPID = m.config.ManagerIdentity.PID
	value.ManagerProcessStartedAt = m.config.ManagerIdentity.StartedAt
	value.ManagerProcessExecutable = m.config.ManagerIdentity.Executable
	value.ManagerVersion = m.config.ManagerVersion
	if err := m.deps.WriteState(m.config.DesktopStatePath, value); err != nil {
		return state.RouterState{}, lifecycleError(protocol.CodeRouterStartFailed, "cannot persist reclaimed ownership")
	}
	success = true
	return value, nil
}

func (m *Manager) MonitorParent(ctx context.Context) *Error {
	if !completeIdentity(m.config.ParentIdentity) {
		return lifecycleError(protocol.CodeInvalidParams, "complete parent identity is required")
	}
	for {
		status, err := m.deps.Validate(m.config.ParentIdentity, m.config.ParentIdentity.Executable)
		if err != nil || status != process.StatusGenuine {
			m.operationMu.Lock()
			stopErr := m.stop(context.Background())
			m.operationMu.Unlock()
			m.releaseLock()
			return stopErr
		}
		if err := m.deps.Sleep(ctx, m.config.PollInterval); err != nil {
			return nil
		}
	}
}

func (m *Manager) RecentOutput() string {
	limit := m.recent.limit
	if data, err := os.ReadFile(m.config.DesktopLogPath); err == nil && len(data) > 0 {
		if len(data) > limit {
			data = data[len(data)-limit:]
		}
		return string(data)
	}
	return m.recent.String()
}
func (m *Manager) UnexpectedExit() <-chan UnexpectedExit { return m.exitCh }

func (m *Manager) reportUnexpectedExit(run *desktopRun) {
	<-run.done
	m.mu.Lock()
	current := m.desktopRun == run
	intentional := run.intentional
	if current {
		m.desktopRun = nil
	}
	m.mu.Unlock()
	if !current || intentional {
		return
	}
	select {
	case m.exitCh <- UnexpectedExit{Identity: run.identity, RecentOutput: run.recentOutput, Err: run.err}:
	default:
	}
}

func (m *Manager) markIntentionalStop(identity process.Identity) *desktopRun {
	m.mu.Lock()
	defer m.mu.Unlock()
	run := m.desktopRun
	if run != nil && run.identity == identity {
		run.intentional = true
		return run
	}
	return nil
}

func (m *Manager) cancelIntentionalStop(run *desktopRun) {
	if run == nil {
		return
	}
	m.mu.Lock()
	if m.desktopRun == run {
		run.intentional = false
	}
	m.mu.Unlock()
}

func (m *Manager) routerState(owner string, identity process.Identity, info discovery.Version) state.RouterState {
	logPath := m.config.DesktopLogPath
	if owner == "cli" {
		logPath = m.config.CLILogPath
	}
	value := state.RouterState{
		PID: identity.PID, ListenAddr: "http://" + m.config.ListenAddr, BinaryPath: identity.Executable, LogPath: logPath,
		StartedAt: m.deps.Now().UTC().Format(time.RFC3339), ProcessStartedAt: identity.StartedAt, ProcessExecutable: identity.Executable,
		Owner: owner, ManagerVersion: m.config.ManagerVersion, RouterVersion: info.Version,
		DeploymentID: m.config.DeploymentID, ManagementProtocolVersion: m.config.ManagementProtocolVersion,
	}
	if owner == "desktop" {
		value.DesktopSessionID = m.config.SessionID
		value.ManagerPID = m.config.ManagerIdentity.PID
		value.ManagerProcessStartedAt = m.config.ManagerIdentity.StartedAt
		value.ManagerProcessExecutable = m.config.ManagerIdentity.Executable
	}
	return value
}

func (m *Manager) cleanupFailedChild(identity process.Identity) {
	if !completeIdentity(identity) {
		return
	}
	status, err := m.deps.Validate(identity, identity.Executable)
	if err == nil && status == process.StatusGenuine {
		_ = m.deps.Signal(identity, identity.Executable, os.Kill)
	}
}

func (m *Manager) cleanupFailedDesktopStart(child foregroundProcess, run *desktopRun, startErr *Error) (*Error, bool) {
	cleanupTimeout := m.config.ForceTimeout
	if cleanupTimeout < 2*foregroundWaitDelay {
		cleanupTimeout = 2 * foregroundWaitDelay
	}
	if killErr := child.Kill(); killErr != nil {
		cleanupTimeout = 2 * foregroundWaitDelay
	}
	timer := time.NewTimer(cleanupTimeout)
	defer timer.Stop()
	select {
	case <-run.done:
		startErr.RecentOutput = run.recentOutput
	case <-timer.C:
		select {
		case <-run.done:
			startErr.RecentOutput = run.recentOutput
			startErr.Launched = true
			return startErr, false
		default:
		}
		// A failed kill or broken process implementation must not hold the
		// manager operation lock forever. Real commands normally complete via
		// WaitDelay before this fallback is reached.
		startErr.RecentOutput = m.recentOutputForRun(run)
		m.trackPendingCleanup(run)
		startErr.Launched = true
		return startErr, true
	}
	startErr.Launched = true
	return startErr, false
}

func (m *Manager) recentOutputForRun(run *desktopRun) string {
	inherited := run.inherited.String()
	file, err := os.Open(m.config.DesktopLogPath)
	if err != nil {
		return inherited
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return inherited
	}
	if run.logBaseline == nil || !os.SameFile(run.logBaseline, info) || info.Size() < run.logBaseline.Size() {
		return inherited
	}
	start := run.logBaseline.Size()
	limit := int64(run.inherited.limit)
	if info.Size()-start > limit {
		start = info.Size() - limit
	}
	if _, err := file.Seek(start, io.SeekStart); err != nil {
		return inherited
	}
	appended, err := readBoundedOutput(file, limit)
	if err != nil || len(appended) == 0 {
		return inherited
	}
	if len(appended) > int(limit) {
		appended = appended[len(appended)-int(limit):]
	}
	return string(appended)
}

func readBoundedOutput(reader io.Reader, limit int64) ([]byte, error) {
	if limit <= 0 {
		return nil, nil
	}
	return io.ReadAll(io.LimitReader(reader, limit))
}

func (m *Manager) acquireDesktopStartLock() (bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.cleanupRun != nil {
		return true, nil
	}
	if m.lock != nil {
		return false, nil
	}
	lock, err := m.deps.AcquireLock(m.config.DesktopLockPath)
	if err != nil {
		return false, err
	}
	m.lock = lock
	return false, nil
}

func (m *Manager) trackPendingCleanup(run *desktopRun) {
	m.mu.Lock()
	m.cleanupRun = run
	m.mu.Unlock()
	go m.finishPendingCleanup(run)
}

func (m *Manager) finishPendingCleanup(run *desktopRun) {
	<-run.done
	m.mu.Lock()
	if m.cleanupRun != run {
		m.mu.Unlock()
		return
	}
	m.cleanupRun = nil
	lock := m.lock
	m.lock = nil
	m.mu.Unlock()
	if lock != nil {
		_ = lock.Close()
	}
}

func (m *Manager) acquireLock() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.cleanupRun != nil {
		return state.ErrLocked
	}
	if m.lock != nil {
		return nil
	}
	lock, err := m.deps.AcquireLock(m.config.DesktopLockPath)
	if err != nil {
		return err
	}
	m.lock = lock
	return nil
}

func (m *Manager) releaseLock() {
	m.mu.Lock()
	if m.cleanupRun != nil {
		m.mu.Unlock()
		return
	}
	lock := m.lock
	m.lock = nil
	m.mu.Unlock()
	if lock != nil {
		_ = lock.Close()
	}
}

func verifyEndpoints(ctx context.Context, baseURL string, pid int, deploymentID, protocolVersion string) (discovery.Version, discovery.Health, error) {
	client := &http.Client{Timeout: time.Second}
	var remote discovery.Version
	if err := getJSON(ctx, client, baseURL+"/version", &remote); err != nil {
		return remote, discovery.Health{}, err
	}
	if remote.PID != pid || remote.DeploymentID != deploymentID || remote.ManagementProtocolVersion != protocolVersion {
		return remote, discovery.Health{}, errors.New("router metadata does not match launched child")
	}
	var health discovery.Health
	if err := getJSON(ctx, client, baseURL+"/health", &health); err != nil {
		return remote, health, err
	}
	if health.Status != "ok" && health.Status != "degraded" {
		return remote, health, errors.New("invalid router health response")
	}
	return remote, health, nil
}

func getJSON(ctx context.Context, client *http.Client, endpoint string, target any) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return err
	}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return errors.New("router endpoint did not return success")
	}
	return json.NewDecoder(resp.Body).Decode(target)
}

func sleepContext(ctx context.Context, delay time.Duration) error {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}

func lifecycleError(code protocol.ErrorCode, message string) *Error {
	return &Error{Code: code, Err: errors.New(message)}
}
func completeIdentity(value process.Identity) bool {
	return value.PID > 0 && value.StartedAt != "" && value.Executable != ""
}
func routerIdentity(value state.RouterState) process.Identity {
	return process.Identity{PID: value.PID, StartedAt: value.ProcessStartedAt, Executable: value.ProcessExecutable}
}
func managerMatches(value state.RouterState, identity process.Identity) bool {
	return value.ManagerPID == identity.PID && value.ManagerProcessStartedAt == identity.StartedAt && value.ManagerProcessExecutable == identity.Executable
}
func completeDesktopState(value state.RouterState) bool {
	return value.PID > 0 && value.ListenAddr != "" && value.BinaryPath != "" && value.LogPath != "" && value.ProcessStartedAt != "" && value.ProcessExecutable != "" && value.DesktopSessionID != "" && value.ManagerPID > 0 && value.ManagerProcessStartedAt != "" && value.ManagerProcessExecutable != "" && value.ManagerVersion != "" && value.RouterVersion != "" && value.DeploymentID != "" && value.ManagementProtocolVersion != ""
}
