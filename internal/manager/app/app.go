// Package app wires the manager services to the private JSON protocol.
package app

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/agent"
	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/lifecycle"
	"github.com/codeasier/mtls-router/internal/manager/metadata"
	"github.com/codeasier/mtls-router/internal/manager/occupant"
	managerpaths "github.com/codeasier/mtls-router/internal/manager/paths"
	"github.com/codeasier/mtls-router/internal/manager/preset"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
	"github.com/codeasier/mtls-router/internal/manager/trustedrouter"
	"github.com/codeasier/mtls-router/internal/version"
)

const (
	defaultListenAddr  = "127.0.0.1:19099"
	defaultLogLines    = 200
	maxLogLines        = 1000
	maxLogReadBytes    = 256 * 1024
	maxLogLineBytes    = 4096
	maxDiagnosticsSize = 16 * 1024
	maxAPIKeySize      = 16 * 1024
	maxModelConfigSize = 2 * 1024 * 1024
)

// Config contains the non-sensitive process and sidecar values supplied to
// manager serve. Zero path and manager identity values are resolved locally.
type Config struct {
	RouterPath      string
	ListenAddr      string
	DesktopSession  string
	ParentIdentity  process.Identity
	ManagerIdentity process.Identity
	Paths           managerpaths.Paths
	AgentDetector   agent.Detector
	Stderr          io.Writer
}

type lifecycleService interface {
	Start(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error)
	Reclaim() (state.RouterState, *lifecycle.Error)
	Stop(context.Context) *lifecycle.Error
	MonitorParent(context.Context) *lifecycle.Error
	RecentOutput() string
	UnexpectedExit() <-chan lifecycle.UnexpectedExit
}

type agentService interface {
	Render(context.Context, []agent.Kind, string, json.RawMessage) (agent.RenderResult, error)
	Write(context.Context, agent.WriteRequest) (agent.WriteResult, error)
}

type agentCatalogBinder interface {
	CatalogBinding(context.Context, []agent.Kind, string, json.RawMessage) (agent.CatalogBinding, error)
}
type agentPreviewValidator interface {
	ValidatePreview(context.Context, agent.WriteRequest) error
}

type agentPreviewerV2 interface {
	PreviewRequest(context.Context, agent.PreviewRequest) (agent.Preview, error)
}
type agentPreviewerLegacy interface {
	Preview(context.Context, []agent.Kind) (agent.Preview, error)
}

type agentModelsService interface {
	DiscoverModels(context.Context, []agent.Kind, []string, modelconfig.CatalogClaims) (agent.ModelsResult, error)
}

type trustedRouterService interface {
	Fetch(context.Context, protocol.RouterOwner, string) (trustedrouter.Result, *protocol.Error)
	Revalidate(context.Context, protocol.RouterOwner, string, trustedrouter.Binding) ([]string, *protocol.Error)
}

type occupantService interface {
	Inspect(context.Context) (occupant.Inspection, error)
	ForceTerminate(context.Context, string) (occupant.Result, error)
}

type dependencies struct {
	info           func() protocol.ManagerInfoResult
	discoverStatus func(context.Context) discovery.Result
	discoverHealth func(context.Context) discovery.Result
	lifecycle      lifecycleService
	detect         func() ([]agent.State, error)
	agent          agentService
	models         agentModelsService
	trusted        trustedRouterService
	occupant       occupantService
	now            func() time.Time
}

// App is one sequential manager protocol session.
type App struct {
	config    Config
	deps      dependencies
	server    *protocol.Server
	failureMu sync.Mutex
	failure   *routerFailure
	active    process.Identity
}

type routerFailure struct {
	identity   process.Identity
	lastError  string
	recentLogs []string
}

type monitorResult struct {
	err        *lifecycle.Error
	parentLost bool
}

// New resolves manager paths, completes Agent transaction recovery, and then
// constructs discovery and lifecycle services. No request can be accepted
// before this function returns.
func New(config Config, simplify bool) (*App, error) {
	loadedPreset, err := preset.Load()
	if err != nil {
		return nil, err
	}
	if config.ListenAddr == "" {
		config.ListenAddr = defaultListenAddr
	}
	listener, err := trustedrouter.NormalizeListener(config.ListenAddr)
	if err != nil {
		return nil, err
	}
	if config.Stderr == nil {
		config.Stderr = io.Discard
	}
	if config.Paths.DesktopDataDir == "" {
		resolved, err := managerpaths.Resolve()
		if err != nil {
			return nil, errors.New("resolve manager paths")
		}
		config.Paths = resolved
	}
	if config.RouterPath == "" {
		path, err := defaultRouterPath()
		if err != nil {
			return nil, err
		}
		config.RouterPath = path
	} else {
		path, err := filepath.Abs(config.RouterPath)
		if err != nil {
			return nil, errors.New("resolve router sidecar path")
		}
		config.RouterPath = filepath.Clean(path)
	}
	if config.ManagerIdentity.PID == 0 {
		identity, err := process.Inspect(os.Getpid())
		if err != nil {
			return nil, errors.New("inspect manager process identity")
		}
		config.ManagerIdentity = identity
	}

	agentStateDir := filepath.Join(config.Paths.DesktopDataDir, "agent-transactions")
	agentManager, recoveryErr := agent.NewService(agent.Options{StateDir: agentStateDir, Detector: config.AgentDetector, Preset: loadedPreset, Simplify: simplify})
	if agentManager == nil {
		return nil, errors.New("initialize Agent service")
	}
	if recoveryErr != nil {
		fmt.Fprintln(config.Stderr, "manager: Agent transaction recovery failed; writes are disabled")
	}

	baseURL := listener.RouterBaseURL
	discoverer := discovery.New(discovery.Config{
		BaseURL:          baseURL,
		DesktopStatePath: config.Paths.DesktopStateFile,
		CLIStatePath:     config.Paths.CLIStateFile,
	})
	lifecycleManager := lifecycle.New(lifecycle.Config{
		RouterPath:        config.RouterPath,
		ListenAddr:        config.ListenAddr,
		DesktopStatePath:  config.Paths.DesktopStateFile,
		CLIStatePath:      config.Paths.CLIStateFile,
		DesktopLockPath:   config.Paths.DesktopLockFile,
		DesktopLogPath:    config.Paths.DesktopLogFile,
		CLILogPath:        config.Paths.CLILogFile,
		SessionID:         config.DesktopSession,
		ManagerIdentity:   config.ManagerIdentity,
		ParentIdentity:    config.ParentIdentity,
		RecentOutputBytes: maxLogReadBytes,
	}, lifecycle.Dependencies{Discover: discoverer.DiscoverStatus})
	occupantManager := occupant.New(occupant.Config{
		ListenAddr: config.ListenAddr, DesktopPID: config.ParentIdentity.PID, ManagerIdentity: config.ManagerIdentity,
		IsProtected: func(candidate occupant.Identity) bool {
			for _, path := range []string{config.Paths.DesktopStateFile, config.Paths.CLIStateFile} {
				value, err := state.Read(path)
				if err != nil {
					continue
				}
				managed := process.Identity{PID: value.PID, StartedAt: value.ProcessStartedAt, Executable: value.ProcessExecutable}
				same, _ := process.SameIdentity(candidate.Process, managed)
				if same {
					return true
				}
			}
			return false
		},
		IsProtectedPID: protectedStatePID(config.Paths.DesktopStateFile, config.Paths.CLIStateFile),
	}, occupant.Dependencies{Discover: discoverer.DiscoverStatus})

	trusted := &trustedrouter.Coordinator{
		Listener: listener, DeploymentID: version.DeploymentID, ProtocolVersion: version.ManagementProtocolVersion,
		Discover: discoverer.DiscoverStatus, Lifecycle: lifecycleManager, Channel: trustedrouter.Channel{Simplify: simplify},
		DesktopEligible: func() bool {
			if config.DesktopSession == "" || !completeIdentity(config.ParentIdentity) {
				return false
			}
			status, err := process.Validate(config.ParentIdentity, config.ParentIdentity.Executable)
			return err == nil && status == process.StatusGenuine
		},
	}
	app := newWithDependencies(config, dependencies{
		info: metadata.Info, discoverStatus: discoverer.DiscoverStatus, discoverHealth: discoverer.Discover, lifecycle: lifecycleManager,
		detect: func() ([]agent.State, error) { return agentManager.Detect(context.Background()) }, agent: agentManager,
		models: agentManager, trusted: trusted, occupant: occupantManager, now: time.Now,
	})
	trusted.AbsentStartOK = app.absentStartOK
	return app, nil
}

func newWithDependencies(config Config, deps dependencies) *App {
	app := &App{config: config, deps: deps}
	app.server = protocol.NewServer(map[protocol.Method]protocol.Handler{
		protocol.MethodManagerInfo:                  app.managerInfo,
		protocol.MethodDiagnosticsCollect:           app.diagnosticsCollect,
		protocol.MethodRouterStatus:                 app.routerStatus,
		protocol.MethodRouterStart:                  app.routerStart,
		protocol.MethodRouterStop:                   app.routerStop,
		protocol.MethodRouterHealth:                 app.routerHealth,
		protocol.MethodRouterVersion:                app.routerVersion,
		protocol.MethodRouterLogs:                   app.routerLogs,
		protocol.MethodRouterInspectOccupant:        app.routerInspectOccupant,
		protocol.MethodRouterForceTerminateOccupant: app.routerForceTerminateOccupant,
		protocol.MethodAgentDetect:                  app.agentDetect,
		protocol.MethodAgentModels:                  app.agentModels,
		protocol.MethodAgentRender:                  app.agentRender,
		protocol.MethodAgentPreview:                 app.agentPreview,
		protocol.MethodAgentWrite:                   app.agentWrite,
	})
	return app
}

// Serve processes requests until EOF. When complete parent identity was
// supplied, parent loss closes stdin, allowing a blocked protocol read to exit
// after lifecycle cleanup.
func (a *App) Serve(ctx context.Context, input io.Reader, output io.Writer) error {
	serveCtx, cancel := context.WithCancel(ctx)
	defer cancel()

	var monitorDone chan monitorResult
	if completeIdentity(a.config.ParentIdentity) {
		monitorDone = make(chan monitorResult, 1)
		go func() {
			monitorErr := a.deps.lifecycle.MonitorParent(serveCtx)
			parentLost := serveCtx.Err() == nil
			monitorDone <- monitorResult{err: monitorErr, parentLost: parentLost}
			if parentLost {
				cancel()
				if closer, ok := input.(io.Closer); ok {
					_ = closer.Close()
				}
			}
		}()
	}

	serveErr := a.server.Serve(serveCtx, input, output)
	cancel()
	if monitorDone != nil {
		result := <-monitorDone
		if result.parentLost {
			if result.err != nil {
				return errors.New("parent-loss router cleanup failed")
			}
			return nil
		}
	}
	if a.config.DesktopSession != "" {
		if stopErr := a.deps.lifecycle.Stop(context.Background()); stopErr != nil && stopErr.Code != protocol.CodeRouterNotFound {
			return errors.New("session-close router cleanup failed")
		}
	}
	return serveErr
}

func (a *App) managerInfo(_ context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	return a.deps.info(), nil
}

func (a *App) diagnosticsCollect(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	info := a.deps.info()
	found := a.deps.discoverStatus(ctx)
	if ctx.Err() != nil {
		return nil, timeoutError()
	}
	states, detectErr := a.deps.detect()
	if ctx.Err() != nil {
		return nil, timeoutError()
	}
	var summary strings.Builder
	fmt.Fprintf(&summary, "manager version=%s commit=%s build_date=%s target=%s deployment_id=%s protocol=%s\n",
		info.Version, info.Commit, info.BuildDate, info.Target, info.DeploymentID, info.ManagementProtocolVersion)
	fmt.Fprintf(&summary, "router state=%s owner=%s listen=%s pid=%d version=%s health=%s\n",
		found.Classification, found.Owner, found.ListenAddr, trustedPID(found), trustedVersion(found), trustedHealth(found))
	if detectErr != nil {
		summary.WriteString("agents unavailable\n")
	} else {
		for _, item := range states {
			fmt.Fprintf(&summary, "agent name=%s detected=%t exists=%t writable=%t configured=%t invalid=%t\n",
				item.Agent, item.Detected, item.Exists, item.Writable, item.Configured, item.Invalid)
		}
	}
	return protocol.DiagnosticsResult{Summary: boundText(sanitizeText(summary.String()), maxDiagnosticsSize)}, nil
}

func (a *App) routerStatus(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	found := a.deps.discoverStatus(ctx)
	a.captureUnexpectedExits()
	if found.Classification == discovery.Absent || found.Classification == discovery.Stale {
		if failed, ok := a.failedStatus(); ok {
			return failed, nil
		}
	}
	return statusResult(found), nil
}

func (a *App) routerStart(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.RouterStartParams
	if err := protocol.DecodeParams(params, &request); err != nil {
		return nil, err
	}
	if request.Owner != protocol.RouterOwnerDesktop && request.Owner != protocol.RouterOwnerCLI {
		return nil, invalidParams("owner must be desktop or cli")
	}
	if sidecarErr := validateSidecar(a.config.RouterPath); sidecarErr != nil {
		return nil, sidecarErr
	}
	a.captureUnexpectedExits()
	value, operationErr := a.deps.lifecycle.Start(ctx, request.Owner)
	if operationErr != nil {
		startErr := operationErr
		if request.Owner == protocol.RouterOwnerDesktop && (operationErr.Code == protocol.CodeRouterAlreadyRunning || operationErr.Code == protocol.CodeRouterStateStale) {
			value, operationErr = a.deps.lifecycle.Reclaim()
			if operationErr == nil {
				a.clearFailureAfterStart(value)
				return statusFromState(value), nil
			}
		}
		if request.Owner == protocol.RouterOwnerDesktop && startErr.Launched {
			a.latchStartupFailure(startErr)
		}
		return nil, mapLifecycleError(operationErr)
	}
	if request.Owner == protocol.RouterOwnerDesktop {
		a.clearFailureAfterStart(value)
	}
	return statusFromState(value), nil
}

func (a *App) routerStop(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	if operationErr := a.deps.lifecycle.Stop(ctx); operationErr != nil {
		return nil, mapLifecycleError(operationErr)
	}
	return protocol.RouterStatusResult{State: string(discovery.Absent)}, nil
}

func (a *App) routerHealth(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	found := a.deps.discoverHealth(ctx)
	if ctx.Err() != nil {
		return nil, timeoutError()
	}
	if err := discoveryError(found, true); err != nil {
		return nil, err
	}
	return protocol.RouterHealthResult{Status: found.Health.Status, CheckedAt: a.deps.now().UTC()}, nil
}

func (a *App) routerVersion(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	found := a.deps.discoverStatus(ctx)
	if ctx.Err() != nil {
		return nil, timeoutError()
	}
	if err := discoveryError(found, false); err != nil {
		return nil, err
	}
	versionValue := found.Version.Version
	deploymentID := found.Version.DeploymentID
	protocolVersion := found.Version.ManagementProtocolVersion
	if versionValue == "" {
		versionValue = found.State.RouterVersion
		deploymentID = found.State.DeploymentID
		protocolVersion = found.State.ManagementProtocolVersion
	}
	return protocol.RouterVersionResult{
		Version: versionValue, DeploymentID: deploymentID, ManagementProtocolVersion: protocolVersion,
	}, nil
}

func (a *App) routerLogs(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.RouterLogsParams
	if err := protocol.DecodeParams(params, &request); err != nil {
		return nil, err
	}
	if request.Limit < 0 || request.Limit > maxLogLines {
		return nil, invalidParams("limit must be between 1 and 1000")
	}
	if request.Limit == 0 {
		request.Limit = defaultLogLines
	}
	found := a.deps.discoverStatus(ctx)
	trustedExternal := trustedResult(found) && found.Owner != string(protocol.RouterOwnerDesktop)
	path := trustedLogPath(found)
	if path == "" && !trustedExternal {
		path = a.config.Paths.DesktopLogFile
	}
	var recent []string
	if !trustedExternal {
		useLifecycleRecent := true
		if found.Classification == discovery.Absent || found.Classification == discovery.Stale {
			a.captureUnexpectedExits()
			var failureFound bool
			recent, failureFound = a.failureLogLines(request.Limit)
			useLifecycleRecent = !failureFound
		}
		if useLifecycleRecent {
			recent = lastLines(sanitizeText(a.deps.lifecycle.RecentOutput()), request.Limit)
		}
	}
	lines, err := readLogLines(ctx, path, request.Limit)
	if err != nil {
		if len(recent) > 0 {
			return protocol.RouterLogsResult{Lines: recent}, nil
		}
		if !errors.Is(err, os.ErrNotExist) {
			return nil, &protocol.Error{Code: protocol.CodeRouterStateStale, Message: "router log is unavailable"}
		}
		return protocol.RouterLogsResult{Lines: []string{}}, nil
	}
	lines = mergeLogLines(lines, recent, request.Limit)
	return protocol.RouterLogsResult{Lines: lines}, nil
}

func (a *App) routerInspectOccupant(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	if a.deps.occupant == nil {
		return nil, mapOccupantError(occupant.ErrIdentityUnavailable)
	}
	inspection, err := a.deps.occupant.Inspect(ctx)
	if err != nil {
		return nil, mapOccupantError(err)
	}
	return protocol.RouterOccupantInspectionResult{
		PID: inspection.PID, VerificationMode: string(inspection.VerificationMode), ProcessName: inspection.ProcessName, Executable: inspection.Executable,
		ListenAddr: inspection.ListenAddr, ConfirmationToken: inspection.ConfirmationToken, ExpiresAt: inspection.ExpiresAt,
	}, nil
}

func protectedStatePID(paths ...string) func(int) bool {
	return func(pid int) bool {
		if pid <= 0 {
			return false
		}
		for _, path := range paths {
			value, err := state.Read(path)
			if err == nil && value.PID > 0 && value.PID == pid {
				return true
			}
		}
		return false
	}
}

func (a *App) routerForceTerminateOccupant(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.RouterForceTerminateOccupantParams
	if err := protocol.DecodeParams(params, &request); err != nil {
		return nil, err
	}
	if strings.TrimSpace(request.ConfirmationToken) == "" {
		return nil, invalidParams("confirmation_token is required")
	}
	if a.deps.occupant == nil {
		return nil, mapOccupantError(occupant.ErrIdentityUnavailable)
	}
	result, err := a.deps.occupant.ForceTerminate(ctx, request.ConfirmationToken)
	request.ConfirmationToken = ""
	if err != nil {
		return nil, mapOccupantError(err)
	}
	return protocol.RouterOccupantTerminationResult{State: result.State}, nil
}

func (a *App) agentDetect(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	if err := decodeEmpty(params); err != nil {
		return nil, err
	}
	states, err := a.deps.detect()
	if ctx.Err() != nil {
		return nil, timeoutError()
	}
	if err != nil {
		return nil, &protocol.Error{Code: protocol.CodeConfigInvalid, Message: "Agent detection failed"}
	}
	result := protocol.AgentDetectResult{Agents: make([]protocol.AgentState, 0, len(states))}
	for _, item := range states {
		recovery := protocol.AgentRecoveryState{Eligible: item.Recovery.Eligible, Reasons: recoveryReasons(item.Recovery.Reasons)}
		for _, file := range item.Recovery.Files {
			recovery.Files = append(recovery.Files, protocol.AgentRecoveryFileState{
				Role: file.Role, Path: file.Path, Format: string(file.Format), Exists: file.Exists, Reasons: recoveryReasons(file.Reasons),
			})
		}
		result.Agents = append(result.Agents, protocol.AgentState{
			Agent: string(item.Agent), Name: item.Name, Detected: item.Detected, Command: item.Command,
			Path: item.Path, AuthPath: item.AuthPath, Format: string(item.Format), Exists: item.Exists,
			Writable: item.Writable, Configured: item.Configured, Invalid: item.Invalid,
			Migratable: item.Migratable, Recovery: recovery,
		})
	}
	return result, nil
}

func recoveryReasons(values []agent.RecoveryReason) []string {
	result := make([]string, len(values))
	for i, value := range values {
		result[i] = string(value)
	}
	return result
}

func (a *App) agentPreview(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.AgentConfigParams
	if err := decodeAgentConfigParams(params, &request); err != nil {
		return nil, err
	}
	selected, err := agentKinds(request.Agents)
	if err != nil {
		return nil, err
	}
	var preview agent.Preview
	var previewErr error
	if service, ok := a.deps.agent.(agentPreviewerV2); ok {
		modes, modeErr := agentModes(request.Modes)
		if modeErr != nil {
			return nil, modeErr
		}
		preview, previewErr = service.PreviewRequest(ctx, agent.PreviewRequest{Agents: selected, Modes: modes, CatalogToken: request.CatalogToken, ModelConfig: request.ModelConfig})
	} else if service, ok := a.deps.agent.(agentPreviewerLegacy); ok {
		preview, previewErr = service.Preview(ctx, selected)
	} else {
		return nil, modelContractUnavailable()
	}
	if previewErr != nil {
		return nil, mapAgentError(previewErr)
	}
	return mapPreview(preview), nil
}

func (a *App) agentWrite(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.AgentWriteParams
	if err := protocol.DecodeParams(params, &request); err != nil {
		return nil, err
	}
	if request.ApproveManagedOverwrite == nil || request.ApproveCodexAuthChange == nil {
		request.APIKey = ""
		return nil, invalidParams("write approval fields are required")
	}
	selected, err := validateAgentConfig(request.Agents, request.CatalogToken, request.ModelConfig)
	if err != nil {
		request.APIKey = ""
		return nil, err
	}
	if strings.TrimSpace(request.RevisionToken) == "" || request.APIKey == "" || len(request.APIKey) > maxAPIKeySize {
		request.APIKey = ""
		return nil, invalidParams("revision_token and bounded api_key are required")
	}
	if a.deps.agent == nil || a.deps.trusted == nil {
		request.APIKey = ""
		return nil, modelContractUnavailable()
	}
	modes, modeErr := agentModes(request.Modes)
	if modeErr != nil {
		request.APIKey = ""
		return nil, modeErr
	}
	approved, approvalErr := approvedAgents(request.ApproveRebuild)
	if approvalErr != nil {
		request.APIKey = ""
		return nil, approvalErr
	}
	writeRequest := agent.WriteRequest{Agents: selected, Modes: modes, ApproveRebuild: approved, CatalogToken: request.CatalogToken, ModelConfig: request.ModelConfig, RevisionToken: request.RevisionToken, ApproveManagedOverwrite: *request.ApproveManagedOverwrite, ApproveCodexAuthChange: *request.ApproveCodexAuthChange}
	validator, ok := a.deps.agent.(agentPreviewValidator)
	if !ok {
		request.APIKey = ""
		return nil, modelContractUnavailable()
	}
	if validateErr := validator.ValidatePreview(ctx, writeRequest); validateErr != nil {
		request.APIKey = ""
		return nil, mapAgentError(validateErr)
	}
	binder, ok := a.deps.agent.(agentCatalogBinder)
	if !ok {
		request.APIKey = ""
		return nil, modelContractUnavailable()
	}
	binding, bindErr := binder.CatalogBinding(ctx, selected, request.CatalogToken, request.ModelConfig)
	if bindErr != nil {
		request.APIKey = ""
		return nil, mapAgentError(bindErr)
	}
	apiBaseURL, parseErr := agentAPIURL(binding.RouterBaseURL)
	if parseErr != nil {
		request.APIKey = ""
		return nil, &protocol.Error{Code: protocol.CodeModelCatalogStale, Message: "Agent model catalog is stale"}
	}
	refreshed, refreshErr := a.deps.trusted.Revalidate(ctx, protocol.RouterOwner(binding.Owner), request.APIKey, trustedrouter.Binding{RouterBaseURL: binding.RouterBaseURL, APIBaseURL: apiBaseURL, DeploymentID: binding.DeploymentID, ProtocolVersion: binding.ProtocolVersion})
	if refreshErr != nil {
		request.APIKey = ""
		return nil, refreshErr
	}
	if err := agent.ValidateRefreshedModels(selected, request.ModelConfig, refreshed); err != nil {
		request.APIKey = ""
		return nil, mapAgentError(err)
	}
	writeRequest.APIKey, request.APIKey = request.APIKey, ""
	result, writeErr := a.deps.agent.Write(ctx, writeRequest)
	writeRequest.APIKey = ""
	if writeErr != nil {
		return nil, mapAgentError(writeErr)
	}
	return mapWrite(result), nil
}

func (a *App) agentModels(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.AgentModelsParams
	if err := protocol.DecodeParams(params, &request); err != nil {
		return nil, err
	}
	if request.Owner != protocol.RouterOwnerCLI && request.Owner != protocol.RouterOwnerDesktop {
		request.APIKey = ""
		return nil, invalidParams("owner must be cli or desktop")
	}
	selected, err := agentKinds(request.Agents)
	if err != nil {
		request.APIKey = ""
		return nil, err
	}
	if request.APIKey == "" || len(request.APIKey) > maxAPIKeySize {
		request.APIKey = ""
		return nil, invalidParams("bounded api_key is required")
	}
	if a.deps.trusted == nil || a.deps.models == nil {
		request.APIKey = ""
		return nil, &protocol.Error{Code: protocol.CodeModelDiscoveryFailed, Message: "model discovery is unavailable"}
	}
	trusted, trustedErr := a.deps.trusted.Fetch(ctx, request.Owner, request.APIKey)
	request.APIKey = ""
	if trustedErr != nil {
		return nil, trustedErr
	}
	modelAgents := make([]modelconfig.Agent, 0, len(selected))
	for _, kind := range selected {
		modelAgents = append(modelAgents, modelconfig.Agent(kind))
	}
	modelsResult, modelsErr := a.deps.models.DiscoverModels(ctx, selected, trusted.Models, modelconfig.CatalogClaims{
		Models: trusted.Models, Agents: modelAgents, Owner: string(request.Owner),
		RouterBaseURL: trusted.Binding.RouterBaseURL, DeploymentID: trusted.Binding.DeploymentID,
		ProtocolVersion: trusted.Binding.ProtocolVersion,
	})
	if modelsErr != nil {
		return nil, mapAgentError(modelsErr)
	}
	if len(modelsResult.Preset.ModelConfig) == 0 {
		modelsResult.Preset.ModelConfig = json.RawMessage(`{}`)
	}
	if modelsResult.Preset.UnavailableAgents == nil {
		modelsResult.Preset.UnavailableAgents = map[string][]string{}
	}
	unavailablePreset := make(map[string]protocol.AgentPresetUnavailable, len(modelsResult.Preset.UnavailableAgents))
	for kind, models := range modelsResult.Preset.UnavailableAgents {
		unavailablePreset[kind] = protocol.AgentPresetUnavailable{Code: protocol.CodeModelNotAvailable, Models: models}
	}
	return protocol.AgentModelsResult{
		Models: trusted.Models, CatalogToken: modelsResult.CatalogToken,
		RouterBaseURL: trusted.Binding.RouterBaseURL, APIBaseURL: trusted.Binding.APIBaseURL,
		Existing: protocol.AgentModelsExisting{
			ModelConfig: modelsResult.Existing.ModelConfig, UnavailableModels: modelsResult.Existing.UnavailableModels,
			DriftedAgents: modelsResult.Existing.DriftedAgents,
		},
		Preset: protocol.AgentModelsPreset{ModelConfig: modelsResult.Preset.ModelConfig, UnavailableAgents: unavailablePreset},
	}, nil
}

func (a *App) agentRender(ctx context.Context, params json.RawMessage) (any, *protocol.Error) {
	var request protocol.AgentConfigParams
	if err := decodeAgentConfigParams(params, &request); err != nil {
		return nil, err
	}
	if len(request.Modes) != 0 {
		return nil, invalidParams("modes are supported only by Agent preview and write")
	}
	if a.deps.agent == nil {
		return nil, modelContractUnavailable()
	}
	selected, err := agentKinds(request.Agents)
	if err != nil {
		return nil, err
	}
	result, renderErr := a.deps.agent.Render(ctx, selected, request.CatalogToken, request.ModelConfig)
	if renderErr != nil {
		return nil, mapAgentError(renderErr)
	}
	fragments := make([]protocol.AgentFragment, len(result.Fragments))
	for i, fragment := range result.Fragments {
		fragments[i] = protocol.AgentFragment{Agent: string(fragment.Agent), Role: fragment.Role, Path: fragment.Path, Format: string(fragment.Format), Content: fragment.Content}
	}
	return protocol.AgentRenderResult{ModelConfig: result.ModelConfig, Fragments: fragments}, nil
}

func decodeAgentConfigParams(params json.RawMessage, request *protocol.AgentConfigParams) *protocol.Error {
	if err := protocol.DecodeParams(params, request); err != nil {
		return err
	}
	_, err := validateAgentConfig(request.Agents, request.CatalogToken, request.ModelConfig)
	return err
}

func validateAgentConfig(agents []string, catalogToken string, modelConfig json.RawMessage) ([]agent.Kind, *protocol.Error) {
	selected, err := agentKinds(agents)
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(catalogToken) == "" {
		return nil, invalidParams("catalog_token is required")
	}
	trimmed := bytes.TrimSpace(modelConfig)
	if len(trimmed) == 0 || len(trimmed) > maxModelConfigSize || trimmed[0] != '{' || !json.Valid(trimmed) {
		return nil, invalidParams("model_config must be a bounded JSON object")
	}
	return selected, nil
}

func modelContractUnavailable() *protocol.Error {
	return &protocol.Error{Code: protocol.CodeModelCatalogStale, Message: "model catalog validation is unavailable"}
}

func agentAPIURL(base string) (string, error) {
	parsed, err := url.Parse(base)
	if err != nil {
		return "", err
	}
	parsed.Path = "/v1"
	return strings.TrimSuffix(parsed.String(), "/"), nil
}

func mapPreview(value agent.Preview) protocol.AgentPreviewResult {
	result := protocol.AgentPreviewResult{RevisionToken: value.RevisionToken, ModelConfig: value.ModelConfig, ManagedConfigDrift: value.ManagedConfigDrift, RequiresCodexAuthApproval: value.RequiresCodexAuthApproval}
	for _, fragment := range value.Fragments {
		result.Fragments = append(result.Fragments, protocol.AgentFragment{Agent: string(fragment.Agent), Role: fragment.Role, Path: fragment.Path, Format: string(fragment.Format), Content: fragment.Content})
	}
	for _, item := range value.Agents {
		for _, file := range item.Files {
			result.Files = append(result.Files, protocol.AgentFileEffect{
				Agent: string(item.Agent), Mode: string(item.Mode), Path: file.Path, Role: file.Role, Format: string(file.Format), Operation: string(file.Operation),
				BackupRequired: file.Backup.Required, BackupPattern: file.Backup.Pattern, BackupSensitive: file.Backup.Sensitive,
				Preserves: append([]string(nil), file.Preserves...), Warning: file.Warning,
			})
		}
	}
	for _, kind := range value.DriftedAgents {
		result.DriftedAgents = append(result.DriftedAgents, string(kind))
	}
	for _, collision := range value.ManagedCollisions {
		result.ManagedCollisions = append(result.ManagedCollisions, protocol.ManagedCollision{Agent: string(collision.Agent), Path: collision.Path, Type: collision.Type, Action: collision.Action})
	}
	if value.StateChange != nil {
		result.StateChange = &protocol.AgentFileEffect{
			Path: value.StateChange.Path, Role: "state", Format: string(value.StateChange.Format), Operation: string(value.StateChange.Operation),
			BackupRequired: value.StateChange.Backup.Required, BackupPattern: value.StateChange.Backup.Pattern,
			BackupSensitive: value.StateChange.Backup.Sensitive, Warning: value.StateChange.Backup.Warning,
		}
	}
	if value.StateBackup != nil {
		result.StateBackup = &protocol.AgentFileEffect{Path: value.StateBackup.Path, Role: "state", Format: string(value.StateBackup.Format), Operation: "backup"}
	}
	return result
}

func mapWrite(value agent.WriteResult) protocol.AgentWriteResult {
	result := protocol.AgentWriteResult{TransactionID: value.TransactionID}
	for _, status := range value.Agents {
		result.Agents = append(result.Agents, protocol.AgentWriteStatus{Agent: string(status.Agent), Success: status.Success, Changed: status.Changed, Backups: status.Backups, ErrorCode: protocol.ErrorCode(status.ErrorCode)})
	}
	if value.StateChange != nil {
		operation := "create"
		if value.StateBackup != nil {
			operation = "replace"
		}
		result.StateChange = &protocol.AgentFileEffect{Path: value.StateChange.Path, Role: "state", Format: "json", Operation: operation}
	}
	if value.StateBackup != nil {
		result.StateBackup = &protocol.AgentFileEffect{Path: value.StateBackup.Path, Role: "state", Format: "json", Operation: "backup"}
	}
	return result
}

func agentModes(values map[string]string) (map[agent.Kind]agent.ConfigMode, *protocol.Error) {
	if len(values) == 0 {
		return nil, nil
	}
	result := make(map[agent.Kind]agent.ConfigMode, len(values))
	for key, value := range values {
		kind := agent.Kind(key)
		if kind != agent.ClaudeCode && kind != agent.OpenCode && kind != agent.Codex {
			return nil, invalidParams("unsupported Agent mode")
		}
		mode := agent.ConfigMode(value)
		if mode != agent.ConfigModeMerge && mode != agent.ConfigModeRebuild {
			return nil, invalidParams("unsupported Agent mode")
		}
		result[kind] = mode
	}
	return result, nil
}

func approvedAgents(values []string) ([]agent.Kind, *protocol.Error) {
	result := make([]agent.Kind, 0, len(values))
	seen := map[agent.Kind]bool{}
	for _, value := range values {
		kind := agent.Kind(value)
		if (kind != agent.ClaudeCode && kind != agent.OpenCode && kind != agent.Codex) || seen[kind] {
			return nil, invalidParams("invalid rebuild approval")
		}
		seen[kind] = true
		result = append(result, kind)
	}
	return result, nil
}

func decodeEmpty(params json.RawMessage) *protocol.Error {
	return protocol.DecodeParams(params, &struct{}{})
}

func agentKinds(values []string) ([]agent.Kind, *protocol.Error) {
	if len(values) == 0 {
		return nil, invalidParams("at least one Agent must be selected")
	}
	seen := make(map[agent.Kind]bool, len(values))
	result := make([]agent.Kind, 0, len(values))
	for _, value := range values {
		kind := agent.Kind(value)
		switch kind {
		case agent.ClaudeCode, agent.OpenCode, agent.Codex:
		default:
			return nil, invalidParams("unsupported Agent selection")
		}
		if seen[kind] {
			return nil, invalidParams("duplicate Agent selection")
		}
		seen[kind] = true
		result = append(result, kind)
	}
	return result, nil
}

func statusResult(found discovery.Result) protocol.RouterStatusResult {
	result := protocol.RouterStatusResult{
		State: string(found.Classification), Owner: found.Owner, ListenAddr: found.ListenAddr,
	}
	if trustedResult(found) {
		result.ProcessID = found.State.PID
		if result.ListenAddr == "" {
			result.ListenAddr = found.State.ListenAddr
		}
	}
	if found.Classification == discovery.Degraded {
		result.LastError = "router health is degraded"
	}
	return result
}

func statusFromState(value state.RouterState) protocol.RouterStatusResult {
	classification := discovery.ExternalCompatible
	if value.Owner == "desktop" {
		classification = discovery.DesktopOwned
	}
	return protocol.RouterStatusResult{
		State: string(classification), Owner: value.Owner, ListenAddr: value.ListenAddr, ProcessID: value.PID,
	}
}

func (a *App) captureUnexpectedExits() {
	exits := a.deps.lifecycle.UnexpectedExit()
	if exits == nil {
		return
	}
	for {
		select {
		case event, ok := <-exits:
			if !ok {
				return
			}
			a.failureMu.Lock()
			if completeIdentity(a.active) && a.active != event.Identity {
				a.failureMu.Unlock()
				continue
			}
			a.failure = &routerFailure{
				identity:   event.Identity,
				lastError:  "desktop-owned router exited unexpectedly",
				recentLogs: lastLines(sanitizeText(event.RecentOutput), defaultLogLines),
			}
			a.active = process.Identity{}
			a.failureMu.Unlock()
		default:
			return
		}
	}
}

func (a *App) failedStatus() (protocol.RouterStatusResult, bool) {
	a.failureMu.Lock()
	defer a.failureMu.Unlock()
	if a.failure == nil {
		return protocol.RouterStatusResult{}, false
	}
	return protocol.RouterStatusResult{
		State:      "start_failed",
		Owner:      string(protocol.RouterOwnerDesktop),
		LastError:  a.failure.lastError,
		RecentLogs: append([]string(nil), a.failure.recentLogs...),
	}, true
}

func (a *App) failureLogLines(limit int) ([]string, bool) {
	a.failureMu.Lock()
	defer a.failureMu.Unlock()
	if a.failure == nil {
		return nil, false
	}
	lines := a.failure.recentLogs
	if len(lines) > limit {
		lines = lines[len(lines)-limit:]
	}
	return append([]string(nil), lines...), true
}

func (a *App) latchStartupFailure(startErr *lifecycle.Error) {
	a.failureMu.Lock()
	defer a.failureMu.Unlock()
	a.failure = &routerFailure{
		lastError:  "desktop-owned router failed during startup",
		recentLogs: lastLines(sanitizeText(startErr.RecentOutput), defaultLogLines),
	}
	a.active = process.Identity{}
}

func (a *App) clearFailureAfterStart(value state.RouterState) {
	a.captureUnexpectedExits()
	a.failureMu.Lock()
	defer a.failureMu.Unlock()
	started := process.Identity{PID: value.PID, StartedAt: value.ProcessStartedAt, Executable: value.ProcessExecutable}
	a.active = started
	if a.failure != nil && a.failure.identity != started {
		a.failure = nil
	}
}

func discoveryError(found discovery.Result, requireHealth bool) *protocol.Error {
	switch found.Classification {
	case discovery.DesktopOwned, discovery.ExternalCompatible:
		if requireHealth && found.Health.Status == "" {
			return &protocol.Error{Code: protocol.CodeRouterDegraded, Message: "router health is unavailable"}
		}
		return nil
	case discovery.Degraded:
		if (!requireHealth && (found.Version.Version != "" || found.State.RouterVersion != "")) ||
			(requireHealth && found.Health.Status != "") {
			return nil
		}
		return &protocol.Error{Code: protocol.CodeRouterDegraded, Message: "router endpoint is unavailable"}
	case discovery.Absent:
		return &protocol.Error{Code: protocol.CodeRouterNotFound, Message: "router was not found"}
	case discovery.Stale:
		return &protocol.Error{Code: protocol.CodeRouterStateStale, Message: "router state is stale"}
	default:
		return &protocol.Error{Code: protocol.CodePortOccupied, Message: "router port is occupied by an unknown process"}
	}
}

func mapLifecycleError(err *lifecycle.Error) *protocol.Error {
	if err == nil {
		return nil
	}
	messages := map[protocol.ErrorCode]string{
		protocol.CodeInvalidParams:        "invalid router lifecycle parameters",
		protocol.CodeRouterNotFound:       "router was not found",
		protocol.CodeRouterAlreadyRunning: "router is already running",
		protocol.CodeRouterStartFailed:    "router could not be started",
		protocol.CodeRouterNotReady:       "router did not become ready",
		protocol.CodeRouterDegraded:       "router upstream is unavailable",
		protocol.CodeRouterNotOwned:       "router is not owned by this desktop session",
		protocol.CodeRouterStateStale:     "router state is stale",
		protocol.CodePortOccupied:         "router port is occupied",
		protocol.CodeOperationTimeout:     "router operation timed out",
	}
	message, ok := messages[err.Code]
	if !ok {
		return &protocol.Error{Code: protocol.CodeRouterStartFailed, Message: "router operation failed"}
	}
	return &protocol.Error{Code: err.Code, Message: message}
}

func mapAgentError(err error) *protocol.Error {
	code := protocol.ErrorCode(agent.CodeOf(err))
	var validationErr *modelconfig.ValidationError
	if code == "" && errors.As(err, &validationErr) {
		code = protocol.CodeModelConfigInvalid
	}
	messages := map[protocol.ErrorCode]string{
		protocol.CodeInvalidParams:        "invalid Agent parameters",
		protocol.CodeAgentNotFound:        "selected Agent was not found",
		protocol.CodeConfigInvalid:        "Agent configuration is invalid",
		protocol.CodeConfigNotWritable:    "Agent configuration is not writable",
		protocol.CodePreviewStale:         "Agent preview is stale",
		protocol.CodeBackupFailed:         "Agent configuration backup failed",
		protocol.CodeWriteFailed:          "Agent configuration write failed",
		protocol.CodeRollbackFailed:       "Agent configuration rollback failed",
		protocol.CodeOperationTimeout:     "Agent operation timed out",
		protocol.CodeAgentOperationBusy:   "Another Agent operation is in progress",
		protocol.CodeModelStateInvalid:    "Agent model state is invalid",
		protocol.CodeModelCatalogStale:    "Agent model catalog is stale",
		protocol.CodeModelConfigInvalid:   "Agent model configuration is invalid",
		protocol.CodeModelNotAvailable:    "A selected model is no longer available",
		protocol.CodeManagedConfigDrift:   "Managed Agent configuration drift requires approval",
		protocol.CodeCodexAuthUnsupported: "Codex authentication policy is unsupported",
	}
	message, ok := messages[code]
	if !ok {
		return &protocol.Error{Code: protocol.CodeWriteFailed, Message: "Agent operation failed"}
	}
	result := &protocol.Error{Code: code, Message: message}
	if errors.As(err, &validationErr) {
		result.Details = &protocol.ErrorDetails{Path: validationErr.Path, Rule: validationErr.Rule}
	}
	return result
}

func mapOccupantError(err error) *protocol.Error {
	for _, item := range []struct {
		err     error
		code    protocol.ErrorCode
		message string
	}{
		{occupant.ErrNotFound, protocol.CodeOccupantNotFound, "port occupant was not found"},
		{occupant.ErrNotOwned, protocol.CodeOccupantNotOwned, "port occupant belongs to another user"},
		{occupant.ErrIdentityUnavailable, protocol.CodeOccupantIdentityUnavailable, "port occupant identity is unavailable"},
		{occupant.ErrChanged, protocol.CodeOccupantChanged, "port occupant changed"},
		{occupant.ErrProtected, protocol.CodeOccupantProtected, "port occupant is protected"},
		{occupant.ErrTerminationFailed, protocol.CodeOccupantTerminationFailed, "port occupant could not be terminated"},
		{occupant.ErrPortReleaseTimeout, protocol.CodePortReleaseTimeout, "router port was not released"},
		{occupant.ErrConfirmationExpired, protocol.CodeConfirmationExpired, "occupant confirmation expired"},
	} {
		if errors.Is(err, item.err) {
			return &protocol.Error{Code: item.code, Message: item.message}
		}
	}
	return &protocol.Error{Code: protocol.CodeOccupantIdentityUnavailable, Message: "port occupant identity is unavailable"}
}

func validateSidecar(path string) *protocol.Error {
	info, err := os.Stat(path)
	if errors.Is(err, os.ErrNotExist) {
		return &protocol.Error{Code: protocol.CodeSidecarMissing, Message: "router sidecar is missing"}
	}
	if err != nil || !info.Mode().IsRegular() {
		return &protocol.Error{Code: protocol.CodeSidecarInvalid, Message: "router sidecar is invalid"}
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0o111 == 0 {
		return &protocol.Error{Code: protocol.CodeSidecarInvalid, Message: "router sidecar is not executable"}
	}
	return nil
}

func validateListenAddr(value string) error {
	_, err := trustedrouter.NormalizeListener(value)
	return err
}

func defaultRouterPath() (string, error) {
	executable, err := os.Executable()
	if err != nil {
		return "", errors.New("resolve manager executable")
	}
	name := "mtls-router"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	return filepath.Join(filepath.Dir(executable), name), nil
}

func readLogLines(ctx context.Context, path string, limit int) ([]string, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return nil, err
	}
	start := info.Size() - maxLogReadBytes
	if start < 0 {
		start = 0
	}
	if _, err := file.Seek(start, io.SeekStart); err != nil {
		return nil, err
	}
	data, err := io.ReadAll(io.LimitReader(file, maxLogReadBytes))
	if err != nil {
		return nil, err
	}
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	if start > 0 {
		if newline := strings.IndexByte(string(data), '\n'); newline >= 0 {
			data = data[newline+1:]
		}
	}
	return lastLines(sanitizeText(string(data)), limit), nil
}

func lastLines(value string, limit int) []string {
	value = strings.TrimRight(value, "\r\n")
	if value == "" {
		return []string{}
	}
	lines := strings.Split(value, "\n")
	if len(lines) > limit {
		lines = lines[len(lines)-limit:]
	}
	for i := range lines {
		lines[i] = strings.TrimSuffix(lines[i], "\r")
		lines[i] = boundText(lines[i], maxLogLineBytes)
	}
	return lines
}

func mergeLogLines(disk, recent []string, limit int) []string {
	overlap := 0
	for size := min(len(disk), len(recent)); size > 0; size-- {
		if slices.Equal(disk[len(disk)-size:], recent[:size]) {
			overlap = size
			break
		}
	}
	merged := append(append([]string(nil), disk...), recent[overlap:]...)
	if len(merged) > limit {
		merged = merged[len(merged)-limit:]
	}
	return merged
}

var (
	pemPattern         = regexp.MustCompile(`(?s)-----BEGIN [^-\r\n]+-----.*?(?:-----END [^-\r\n]+-----|$)`)
	authHeaderPattern  = regexp.MustCompile(`(?i)((?:authorization|proxy-authorization|authentication)\s*[:=]\s*)[^\r\n]*`)
	keyPattern         = regexp.MustCompile(`(?i)((?:api[_-]?key|auth(?:entication|orization)?|token|secret|password)["']?\s*[:=]\s*["']?)([^"'\s,}]+)`)
	skPattern          = regexp.MustCompile(`\bsk-[A-Za-z0-9_-]{8,}\b`)
	urlUserinfoPattern = regexp.MustCompile(`(?i)(https?://)[^/\s?#"']+@`)
	urlPattern         = regexp.MustCompile(`(?i)(https?://[^\s?"']+)\?[^\s"']+`)
)

func sanitizeText(value string) string {
	value = pemPattern.ReplaceAllString(value, "[REDACTED PEM]")
	value = authHeaderPattern.ReplaceAllString(value, `${1}[REDACTED]`)
	value = keyPattern.ReplaceAllString(value, `${1}[REDACTED]`)
	value = skPattern.ReplaceAllString(value, "[REDACTED KEY]")
	value = urlUserinfoPattern.ReplaceAllString(value, `${1}[REDACTED]@`)
	return urlPattern.ReplaceAllString(value, `${1}?[REDACTED]`)
}

func boundText(value string, limit int) string {
	if len(value) <= limit {
		return value
	}
	return value[:limit] + "[truncated]"
}

func trustedResult(found discovery.Result) bool {
	return found.Classification == discovery.DesktopOwned || found.Classification == discovery.ExternalCompatible || found.Classification == discovery.Degraded
}

func trustedPID(found discovery.Result) int {
	if trustedResult(found) {
		return found.State.PID
	}
	return 0
}

func trustedVersion(found discovery.Result) string {
	if !trustedResult(found) {
		return ""
	}
	if found.Version.Version != "" {
		return found.Version.Version
	}
	return found.State.RouterVersion
}

func trustedHealth(found discovery.Result) string {
	if trustedResult(found) {
		return found.Health.Status
	}
	return ""
}

func trustedLogPath(found discovery.Result) string {
	if trustedResult(found) {
		return found.State.LogPath
	}
	return ""
}

func completeIdentity(value process.Identity) bool {
	return value.PID > 0 && value.StartedAt != "" && value.Executable != ""
}

func (a *App) absentStartOK() bool {
	a.captureUnexpectedExits()
	a.failureMu.Lock()
	defer a.failureMu.Unlock()
	return a.failure == nil
}

func invalidParams(message string) *protocol.Error {
	return &protocol.Error{Code: protocol.CodeInvalidParams, Message: message}
}

func timeoutError() *protocol.Error {
	return &protocol.Error{Code: protocol.CodeOperationTimeout, Message: "operation timed out"}
}
