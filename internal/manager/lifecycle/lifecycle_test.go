package lifecycle

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/background"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

func TestDesktopStartUsesControlledForegroundLaunchAndWritesVerifiedState(t *testing.T) {
	fixture := newFixture(t)
	var args, env []string
	fixture.deps.Environ = func() []string {
		return []string{"PATH=/bin", "MTLS_UPSTREAM_URL=https://unsafe", "mtls_backend=true", "OTHER=ok"}
	}
	fixture.deps.LaunchDesktop = func(_ string, gotArgs, gotEnv []string, output io.Writer) (foregroundProcess, error) {
		args, env = gotArgs, gotEnv
		_, _ = output.Write([]byte("started\n"))
		return &fakeChild{pid: 101, wait: make(chan error)}, nil
	}
	value, protocolErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if protocolErr != nil {
		t.Fatal(protocolErr)
	}
	if value.PID != 101 || value.Owner != "desktop" || value.DesktopSessionID != "session" || value.DeploymentID != "prod-a" {
		t.Fatalf("state = %+v", value)
	}
	if containsArg(args, "-backend") || containsArg(args, "--backend") {
		t.Fatalf("desktop args contain backend mode: %#v", args)
	}
	for _, want := range []string{"-listen", "127.0.0.1:19099", "-log", fixture.config.DesktopLogPath, "-tls-min", "tls1.2", "-timeout", "10s", "-debug=false"} {
		if !containsArg(args, want) {
			t.Fatalf("args %#v missing %q", args, want)
		}
	}
	if !reflect.DeepEqual(env, []string{"PATH=/bin", "OTHER=ok"}) {
		t.Fatalf("environment = %#v", env)
	}
	if fixture.writes != 1 {
		t.Fatalf("state writes = %d, want 1", fixture.writes)
	}
	if got := fixture.managerState; got.PID != 101 || got.ProcessStartedAt == "" || got.ManagerProcessStartedAt == "" {
		t.Fatalf("persisted incomplete state: %+v", got)
	}
}

func TestCLIStartUsesDetachedLaunchAndPersistsOnlyAfterVerification(t *testing.T) {
	fixture := newFixture(t)
	fixture.discovered = discovery.Result{Classification: discovery.Absent}
	var args []string
	fixture.deps.LaunchDetached = func(_ string, got []string, log string) (int, error) {
		args = got
		if fixture.writes != 0 {
			t.Fatal("state written before child verification")
		}
		if log != fixture.config.CLILogPath {
			t.Fatalf("log = %q", log)
		}
		return 202, nil
	}
	value, protocolErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerCLI)
	if protocolErr != nil {
		t.Fatal(protocolErr)
	}
	if value.PID != 202 || value.Owner != "cli" {
		t.Fatalf("state = %+v", value)
	}
	if containsArg(args, "-backend") {
		t.Fatalf("manager detached child must not recursively backend: %#v", args)
	}
	if fixture.cliState.PID != 202 || fixture.writes != 1 {
		t.Fatalf("CLI state = %+v, writes=%d", fixture.cliState, fixture.writes)
	}
}

func TestStartScopesDiscoveryToRequestedOwner(t *testing.T) {
	for _, tt := range []struct {
		name  string
		owner protocol.RouterOwner
		want  protocol.ErrorCode
	}{
		{name: "desktop ignores cross-owner stale", owner: protocol.RouterOwnerDesktop},
		{name: "CLI retains same-owner stale", owner: protocol.RouterOwnerCLI, want: protocol.CodeRouterAlreadyRunning},
	} {
		t.Run(tt.name, func(t *testing.T) {
			fixture := newFixture(t)
			var seen protocol.RouterOwner
			fixture.deps.Discover = func(_ context.Context, owner protocol.RouterOwner) discovery.Result {
				seen = owner
				if owner == protocol.RouterOwnerDesktop {
					return discovery.Result{Classification: discovery.Absent}
				}
				return discovery.Result{Classification: discovery.Stale}
			}

			_, startErr := fixture.manager().Start(context.Background(), tt.owner)
			if seen != tt.owner {
				t.Fatalf("discovery owner = %q, want %q", seen, tt.owner)
			}
			if tt.want == "" && startErr != nil {
				t.Fatal(startErr)
			}
			if tt.want != "" && (startErr == nil || startErr.Code != tt.want) {
				t.Fatalf("start error = %v, want %s", startErr, tt.want)
			}
		})
	}
}

func TestRepeatedDesktopStartDoesNotLaunchSecondChild(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = fixture.desktopState(101)
	fixture.managerState.ManagerPID = fixture.config.ManagerIdentity.PID
	fixture.managerState.ManagerProcessStartedAt = fixture.config.ManagerIdentity.StartedAt
	fixture.managerState.ManagerProcessExecutable = fixture.config.ManagerIdentity.Executable
	fixture.deps.LaunchDesktop = func(string, []string, []string, io.Writer) (foregroundProcess, error) {
		t.Fatal("repeated start launched another child")
		return nil, nil
	}
	value, protocolErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if protocolErr != nil {
		t.Fatal(protocolErr)
	}
	if value.PID != 101 {
		t.Fatalf("PID = %d", value.PID)
	}
}

func TestDesktopStartRemovesLegacyStateOnlyAfterPIDIsProvenAbsent(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = state.RouterState{PID: 77, Owner: "desktop"}
	fixture.validate = func(identity process.Identity, _ string) (process.Status, error) {
		if identity.PID == 77 {
			return process.StatusStale, nil
		}
		return process.StatusGenuine, nil
	}
	fixture.deps.Inspect = func(pid int) (process.Identity, error) {
		if pid == 77 {
			return process.Identity{}, process.ErrNotFound
		}
		return process.Identity{PID: pid, StartedAt: "router-start", Executable: "/router"}, nil
	}

	if _, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop); startErr != nil {
		t.Fatal(startErr)
	}
	if !fixture.removed || fixture.desktopLaunches != 1 {
		t.Fatalf("removed=%t desktop launches=%d", fixture.removed, fixture.desktopLaunches)
	}
}

func TestDesktopStartRemovesCompleteStateWhenValidationProvesPIDAbsent(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = fixture.desktopState(77)
	fixture.validate = func(identity process.Identity, _ string) (process.Status, error) {
		if identity.PID == 77 {
			return process.StatusAbsent, nil
		}
		return process.StatusGenuine, nil
	}

	if _, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop); startErr != nil {
		t.Fatal(startErr)
	}
	if !fixture.removed || fixture.desktopLaunches != 1 {
		t.Fatalf("removed=%t desktop launches=%d", fixture.removed, fixture.desktopLaunches)
	}
}

func TestDesktopStartRetainsLegacyStateWhenPIDCannotBeProvenAbsent(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = state.RouterState{PID: 77, Owner: "desktop"}
	fixture.discovered = discovery.Result{Classification: discovery.Stale}
	fixture.validate = func(process.Identity, string) (process.Status, error) {
		return process.StatusStale, nil
	}
	fixture.deps.Inspect = func(pid int) (process.Identity, error) {
		return process.Identity{PID: pid, StartedAt: "unknown", Executable: "/unknown"}, nil
	}

	_, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if startErr == nil || startErr.Code != protocol.CodeRouterStateStale {
		t.Fatalf("start error = %v", startErr)
	}
	if startErr.Stage != StartupStageStateReconcile {
		t.Fatalf("startup stage = %q, want %q", startErr.Stage, StartupStageStateReconcile)
	}
	if fixture.removed || fixture.desktopLaunches != 0 {
		t.Fatalf("removed=%t desktop launches=%d", fixture.removed, fixture.desktopLaunches)
	}
}

func TestDesktopStartRetainsLegacyStateWhenPIDInspectionFails(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = state.RouterState{PID: 77, Owner: "desktop"}
	fixture.discovered = discovery.Result{Classification: discovery.Stale}
	fixture.validate = func(process.Identity, string) (process.Status, error) {
		return process.StatusStale, nil
	}
	fixture.deps.Inspect = func(int) (process.Identity, error) {
		return process.Identity{}, errors.New("inspection unavailable")
	}

	_, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if startErr == nil || startErr.Code != protocol.CodeRouterStateStale || startErr.Stage != StartupStageStateReconcile {
		t.Fatalf("start error = %+v", startErr)
	}
	if fixture.removed || fixture.desktopLaunches != 0 {
		t.Fatalf("removed=%t desktop launches=%d", fixture.removed, fixture.desktopLaunches)
	}
}

func TestDesktopStartReportsLegacyStateRemovalFailure(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = fixture.desktopState(77)
	fixture.validate = func(process.Identity, string) (process.Status, error) {
		return process.StatusAbsent, nil
	}
	fixture.deps.RemoveState = func(string) error { return syscall.Errno(5) }

	_, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if startErr == nil || startErr.Code != protocol.CodeRouterStateStale || startErr.Stage != StartupStageStateReconcile || startErr.OSErrorCode != 5 {
		t.Fatalf("start error = %+v", startErr)
	}
	if fixture.desktopLaunches != 0 {
		t.Fatalf("desktop launches = %d", fixture.desktopLaunches)
	}
}

func TestLegacyStateWithoutPIDRemainsFailClosed(t *testing.T) {
	manager := New(Config{}, Dependencies{Inspect: func(int) (process.Identity, error) {
		t.Fatal("zero PID must not be inspected")
		return process.Identity{}, nil
	}})
	if manager.recordedProcessAbsent(state.RouterState{}, process.StatusStale) {
		t.Fatal("legacy state without PID was treated as safely removable")
	}
}

func TestFailedAndTimedOutStartCleanUpWithoutWritingState(t *testing.T) {
	for _, tt := range []struct {
		name           string
		verify         func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error)
		want           protocol.ErrorCode
		wantOwnedKills int
	}{
		{name: "not ready", verify: func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error) {
			return discovery.Version{}, discovery.Health{}, errors.New("not ready")
		}, want: protocol.CodeRouterNotReady, wantOwnedKills: 1},
		{name: "identity mismatch", verify: func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error) {
			return discovery.Version{PID: 101}, discovery.Health{Status: "ok"}, nil
		}, want: protocol.CodeRouterStateStale, wantOwnedKills: 1},
	} {
		t.Run(tt.name, func(t *testing.T) {
			fixture := newFixture(t)
			fixture.config.StartupTimeout = time.Millisecond
			fixture.deps.Verify = tt.verify
			if tt.name == "identity mismatch" {
				fixture.validate = func(process.Identity, string) (process.Status, error) { return process.StatusStale, nil }
			}
			fixture.deps.Sleep = func(context.Context, time.Duration) error { return context.DeadlineExceeded }
			_, protocolErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
			if protocolErr == nil || protocolErr.Code != tt.want {
				t.Fatalf("error = %v, want %s", protocolErr, tt.want)
			}
			if !protocolErr.Launched || protocolErr.RecentOutput != "owned child output" {
				t.Fatalf("failure metadata = %+v", protocolErr)
			}
			if strings.Contains(protocolErr.Error(), protocolErr.RecentOutput) {
				t.Fatalf("generic error contains raw output: %q", protocolErr.Error())
			}
			if fixture.writes != 0 {
				t.Fatalf("state writes = %d", fixture.writes)
			}
			if fixture.ownedChildKills != tt.wantOwnedKills {
				t.Fatalf("owned child kills = %d, want %d", fixture.ownedChildKills, tt.wantOwnedKills)
			}
			if fixture.killSignals != 0 {
				t.Fatalf("identity-based cleanup kill signals = %d", fixture.killSignals)
			}
			if !fixture.lockClosed {
				t.Fatal("failed start retained ownership lock")
			}
		})
	}
}

func TestDesktopStateWriteFailureDrainsOwnedChild(t *testing.T) {
	fixture := newFixture(t)
	fixture.deps.WriteState = func(string, state.RouterState) error {
		fixture.writes++
		return errors.New("write failed")
	}
	_, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if startErr == nil || startErr.Code != protocol.CodeRouterStartFailed {
		t.Fatalf("error = %v", startErr)
	}
	if !startErr.Launched || startErr.RecentOutput != "owned child output" {
		t.Fatalf("failure metadata = %+v", startErr)
	}
	if startErr.Stage != StartupStageStatePersist {
		t.Fatalf("startup stage = %q, want %q", startErr.Stage, StartupStageStatePersist)
	}
	if got := startErr.Error(); got != "ROUTER_START_FAILED: cannot persist verified router state" {
		t.Fatalf("generic error text = %q", got)
	}
	if fixture.writes != 1 || fixture.managerState.PID != 0 {
		t.Fatalf("writes=%d persisted state=%+v", fixture.writes, fixture.managerState)
	}
	if fixture.ownedChildKills != 1 || !fixture.lockClosed {
		t.Fatalf("owned child kills=%d lock closed=%t", fixture.ownedChildKills, fixture.lockClosed)
	}
}

func TestDesktopLaunchFailureRemainsUnmarked(t *testing.T) {
	fixture := newFixture(t)
	fixture.deps.LaunchDesktop = func(string, []string, []string, io.Writer) (foregroundProcess, error) {
		return nil, fmt.Errorf("launch failed: %w", syscall.Errno(5))
	}
	_, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if startErr == nil || startErr.Code != protocol.CodeRouterStartFailed {
		t.Fatalf("error = %v", startErr)
	}
	if startErr.Stage != StartupStageProcessLaunch || startErr.OSErrorCode != 5 {
		t.Fatalf("pre-launch stage metadata = %+v", startErr)
	}
	if startErr.Launched || startErr.RecentOutput != "" {
		t.Fatalf("pre-launch output metadata = %+v", startErr)
	}
	if got := startErr.Error(); got != "ROUTER_START_FAILED: router launch failed" {
		t.Fatalf("generic error text = %q", got)
	}
	if !fixture.lockClosed {
		t.Fatal("failed launch retained ownership lock")
	}
}

func TestStartupErrorUsesClosedStageAndNumericErrno(t *testing.T) {
	for _, stage := range []StartupStage{
		StartupStageLogDirectory,
		StartupStageLogOpen,
		StartupStageProcessLaunch,
		StartupStageProcessInspect,
		StartupStageReadiness,
		StartupStageIdentity,
		StartupStageStateReconcile,
		StartupStageStatePersist,
		StartupStageProcessExit,
	} {
		t.Run(string(stage), func(t *testing.T) {
			got := startupError(stage, protocol.CodeRouterStartFailed, "safe message", fmt.Errorf("wrapped: %w", syscall.Errno(5)))
			if got.Stage != stage || got.OSErrorCode != 5 {
				t.Fatalf("startupError(%q) = %+v", stage, got)
			}
			if got.Err.Error() != "safe message" || strings.Contains(got.Error(), "wrapped") {
				t.Fatalf("startup error exposed cause: %q", got.Error())
			}
		})
	}

	got := startupError(StartupStageLogOpen, protocol.CodeRouterStartFailed, "safe message", errors.New("no errno"))
	if got.OSErrorCode != 0 {
		t.Fatalf("plain error code = %d", got.OSErrorCode)
	}
}

func TestDesktopLogOpenFailureHasStableStage(t *testing.T) {
	fixture := newFixture(t)
	fixture.deps.OpenLog = func(string) (*os.File, error) {
		return nil, &os.PathError{Op: "open", Path: "/secret/router.log", Err: syscall.Errno(13)}
	}

	_, startErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if startErr == nil || startErr.Stage != StartupStageLogOpen || startErr.OSErrorCode != 13 {
		t.Fatalf("log open failure = %+v", startErr)
	}
	if strings.Contains(startErr.Error(), "/secret") {
		t.Fatalf("log open failure exposed path: %q", startErr.Error())
	}
}

func TestDesktopInspectFailureWaitsForChildAndDrainsRecentOutput(t *testing.T) {
	fixture := newFixture(t)
	fixture.config.RecentOutputBytes = 64
	fixture.deps.Inspect = func(int) (process.Identity, error) {
		return process.Identity{}, errors.New("inspect failed")
	}
	outputWritten := make(chan struct{})
	releaseWait := make(chan struct{})
	terminated := make(chan struct{})
	fixture.deps.LaunchDesktop = func(_ string, _ []string, _ []string, output io.Writer) (foregroundProcess, error) {
		return &fakeChild{
			pid: 101,
			waitFunc: func() error {
				<-terminated
				_, _ = output.Write([]byte("complete startup failure output"))
				close(outputWritten)
				<-releaseWait
				return errors.New("startup failed")
			},
			killFunc: func() error {
				close(terminated)
				return nil
			},
		}, nil
	}

	manager := fixture.manager()
	result := make(chan *Error, 1)
	go func() {
		_, startErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop)
		result <- startErr
	}()

	select {
	case <-outputWritten:
	case <-time.After(time.Second):
		t.Fatal("owned child was not terminated")
	}
	select {
	case startErr := <-result:
		t.Fatalf("Start returned before child Wait completed: %v", startErr)
	default:
	}
	close(releaseWait)

	var startErr *Error
	select {
	case startErr = <-result:
	case <-time.After(time.Second):
		t.Fatal("Start did not return after child Wait completed")
	}
	if startErr == nil || startErr.Code != protocol.CodeRouterStartFailed {
		t.Fatalf("error = %v", startErr)
	}
	if !startErr.Launched {
		t.Fatal("post-launch failure was not marked launched")
	}
	if startErr.RecentOutput != "complete startup failure output" {
		t.Fatalf("recent output = %q", startErr.RecentOutput)
	}
	if got := startErr.Error(); got != "ROUTER_START_FAILED: cannot inspect launched router" {
		t.Fatalf("generic error text = %q", got)
	}
	if fixture.writes != 0 {
		t.Fatalf("state writes = %d", fixture.writes)
	}
	if !fixture.lockClosed {
		t.Fatal("failed start retained ownership lock")
	}
	select {
	case event := <-manager.UnexpectedExit():
		t.Fatalf("startup failure reported as unexpected: %+v", event)
	default:
	}
}

func TestDesktopFailureCleanupWaitsAfterKillFailureAndCapturesFinalTail(t *testing.T) {
	fixture := newFixture(t)
	fixture.config.RecentOutputBytes = 64
	fixture.deps.Inspect = func(int) (process.Identity, error) {
		return process.Identity{}, errors.New("inspect failed")
	}
	releaseWait := make(chan struct{})
	waitStarted := make(chan struct{})
	fixture.deps.LaunchDesktop = func(_ string, _ []string, _ []string, output io.Writer) (foregroundProcess, error) {
		return &fakeChild{
			pid: 101,
			waitFunc: func() error {
				close(waitStarted)
				<-releaseWait
				_, _ = output.Write([]byte("final drained tail"))
				return errors.New("eventually exited")
			},
			killFunc: func() error { return errors.New("kill failed") },
		}, nil
	}

	manager := fixture.manager()
	result := make(chan *Error, 1)
	go func() {
		_, startErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop)
		result <- startErr
	}()

	select {
	case <-waitStarted:
	case <-time.After(time.Second):
		t.Fatal("child Wait did not start")
	}
	select {
	case startErr := <-result:
		t.Fatalf("Start returned before child Wait completed: %+v", startErr)
	case <-time.After(2 * foregroundWaitDelay):
	}
	close(releaseWait)
	select {
	case startErr := <-result:
		if startErr == nil || !startErr.Launched || startErr.RecentOutput != "final drained tail" {
			t.Fatalf("failure metadata = %+v", startErr)
		}
	case <-time.After(time.Second):
		t.Fatal("Start did not return after child Wait completed")
	}
}

func TestDesktopFailureOutputIsScopedToCurrentLaunch(t *testing.T) {
	fixture := newFixture(t)
	fixture.config.RecentOutputBytes = 256
	if err := os.WriteFile(fixture.config.DesktopLogPath, []byte("historical output\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	fixture.deps.OpenLog = background.OpenLogFile
	fixture.deps.Inspect = func(int) (process.Identity, error) {
		return process.Identity{}, errors.New("inspect failed")
	}
	launch := 0
	fixture.deps.LaunchDesktop = func(_ string, _ []string, _ []string, output io.Writer) (foregroundProcess, error) {
		launch++
		_, _ = fmt.Fprintf(output, "inherited attempt %d\n", launch)
		direct, err := background.OpenLogFile(fixture.config.DesktopLogPath)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = fmt.Fprintf(direct, "direct attempt %d\n", launch)
		_ = direct.Close()
		wait := make(chan error, 1)
		return &fakeChild{pid: 100 + launch, wait: wait, killFunc: func() error {
			wait <- errors.New("killed")
			return nil
		}}, nil
	}

	manager := fixture.manager()
	_, firstErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop)
	if firstErr == nil {
		t.Fatal("first launch unexpectedly succeeded")
	}
	if firstErr.RecentOutput != "inherited attempt 1\ndirect attempt 1\n" {
		t.Fatalf("first failure output = %q", firstErr.RecentOutput)
	}
	_, secondErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop)
	if secondErr == nil {
		t.Fatal("second launch unexpectedly succeeded")
	}
	if secondErr.RecentOutput != "inherited attempt 2\ndirect attempt 2\n" {
		t.Fatalf("second failure output = %q", secondErr.RecentOutput)
	}
	fullLog, err := os.ReadFile(fixture.config.DesktopLogPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"historical output", "inherited attempt 1", "direct attempt 1", "inherited attempt 2", "direct attempt 2"} {
		if !strings.Contains(string(fullLog), want) {
			t.Fatalf("full log %q missing %q", fullLog, want)
		}
	}
	if got := manager.RecentOutput(); got != string(fullLog) {
		t.Fatalf("manager recent output = %q, want full log %q", got, fullLog)
	}
}

func TestReadBoundedOutputStopsAtLimit(t *testing.T) {
	reader := &countingInfiniteReader{}
	output, err := readBoundedOutput(reader, 32)
	if err != nil {
		t.Fatal(err)
	}
	if len(output) != 32 {
		t.Fatalf("output length = %d, want 32", len(output))
	}
	if reader.read != 32 {
		t.Fatalf("underlying bytes read = %d, want 32", reader.read)
	}
}

func TestRecentOutputForRunBoundsOversizedFileDelta(t *testing.T) {
	fixture := newFixture(t)
	fixture.config.RecentOutputBytes = 32
	if err := os.WriteFile(fixture.config.DesktopLogPath, []byte("historical"), 0o600); err != nil {
		t.Fatal(err)
	}
	baselineFile, err := os.Open(fixture.config.DesktopLogPath)
	if err != nil {
		t.Fatal(err)
	}
	baseline, err := baselineFile.Stat()
	_ = baselineFile.Close()
	if err != nil {
		t.Fatal(err)
	}
	direct, err := background.OpenLogFile(fixture.config.DesktopLogPath)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = direct.Write([]byte(strings.Repeat("x", 1024*1024) + "current-tail"))
	_ = direct.Close()
	run := &desktopRun{logBaseline: baseline, inherited: newBoundedOutput(32)}

	started := time.Now()
	got := fixture.manager().recentOutputForRun(run)
	if elapsed := time.Since(started); elapsed > 250*time.Millisecond {
		t.Fatalf("bounded snapshot took %s", elapsed)
	}
	if len(got) != 32 || !strings.HasSuffix(got, "current-tail") {
		t.Fatalf("snapshot = %q, length %d", got, len(got))
	}
}

func TestRecentOutputForRunRejectsReplacementAndTruncation(t *testing.T) {
	for _, tt := range []struct {
		name   string
		mutate func(*testing.T, string)
	}{
		{name: "replacement", mutate: func(t *testing.T, path string) {
			if err := os.Rename(path, path+".rotated"); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(path, []byte("replacement bytes"), 0o600); err != nil {
				t.Fatal(err)
			}
		}},
		{name: "truncated", mutate: func(t *testing.T, path string) {
			if err := os.WriteFile(path, []byte("short"), 0o600); err != nil {
				t.Fatal(err)
			}
		}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			fixture := newFixture(t)
			if err := os.WriteFile(fixture.config.DesktopLogPath, []byte("long historical output"), 0o600); err != nil {
				t.Fatal(err)
			}
			baselineFile, err := os.Open(fixture.config.DesktopLogPath)
			if err != nil {
				t.Fatal(err)
			}
			baseline, err := baselineFile.Stat()
			_ = baselineFile.Close()
			if err != nil {
				t.Fatal(err)
			}
			inherited := newBoundedOutput(64)
			_, _ = inherited.Write([]byte("current inherited output"))
			run := &desktopRun{logBaseline: baseline, inherited: inherited}
			tt.mutate(t, fixture.config.DesktopLogPath)
			if got := fixture.manager().recentOutputForRun(run); got != "current inherited output" {
				t.Fatalf("snapshot = %q", got)
			}
		})
	}
}

func TestDegradedStartPersistsUsableChild(t *testing.T) {
	fixture := newFixture(t)
	fixture.deps.Verify = func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error) {
		return discovery.Version{Version: "v1", PID: 101}, discovery.Health{Status: "degraded"}, nil
	}
	value, protocolErr := fixture.manager().Start(context.Background(), protocol.RouterOwnerDesktop)
	if protocolErr == nil || protocolErr.Code != protocol.CodeRouterDegraded {
		t.Fatalf("error = %v", protocolErr)
	}
	if value.PID != 101 || fixture.writes != 1 || fixture.killSignals != 0 {
		t.Fatalf("state=%+v writes=%d kills=%d", value, fixture.writes, fixture.killSignals)
	}
}

func TestStopGracefulAndForcedAreIdentitySafe(t *testing.T) {
	for _, tt := range []struct {
		name        string
		statuses    []process.Status
		wantSignals int
	}{
		{name: "graceful", statuses: []process.Status{process.StatusGenuine, process.StatusAbsent}, wantSignals: 1},
		{name: "forced", statuses: []process.Status{process.StatusGenuine, process.StatusGenuine, process.StatusGenuine, process.StatusAbsent}, wantSignals: 2},
	} {
		t.Run(tt.name, func(t *testing.T) {
			fixture := newFixture(t)
			fixture.managerState = fixture.desktopState(101)
			fixture.ownState()
			index := 0
			fixture.validate = func(process.Identity, string) (process.Status, error) {
				if index >= len(tt.statuses) {
					return tt.statuses[len(tt.statuses)-1], nil
				}
				status := tt.statuses[index]
				index++
				return status, nil
			}
			fixture.deps.Sleep = func(context.Context, time.Duration) error {
				if tt.name == "forced" && index < 3 {
					return context.DeadlineExceeded
				}
				return nil
			}
			manager := fixture.manager()
			if protocolErr := manager.Stop(context.Background()); protocolErr != nil {
				t.Fatal(protocolErr)
			}
			if fixture.signals != tt.wantSignals {
				t.Fatalf("signals = %d, want %d", fixture.signals, tt.wantSignals)
			}
			if !fixture.removed {
				t.Fatal("stopped state was not removed")
			}
		})
	}
}

func TestStopCLIRouterUsesCLIStateAndIdentityValidation(t *testing.T) {
	fixture := newFixture(t)
	fixture.config.SessionID = ""
	fixture.cliState = state.RouterState{
		PID: 202, Owner: "cli", BinaryPath: "/router", ProcessStartedAt: "router-start", ProcessExecutable: "/router",
	}
	statuses := []process.Status{process.StatusGenuine, process.StatusAbsent}
	fixture.validate = func(process.Identity, string) (process.Status, error) {
		status := statuses[0]
		statuses = statuses[1:]
		return status, nil
	}
	if protocolErr := fixture.manager().Stop(context.Background()); protocolErr != nil {
		t.Fatal(protocolErr)
	}
	if fixture.signals != 1 || !fixture.removed {
		t.Fatalf("signals=%d removed=%t", fixture.signals, fixture.removed)
	}
}

func TestStopRejectsExternalAndStaleWithoutSignal(t *testing.T) {
	for _, tt := range []struct {
		name, owner string
		status      process.Status
		want        protocol.ErrorCode
	}{
		{name: "external", owner: "cli", status: process.StatusGenuine, want: protocol.CodeRouterNotOwned},
		{name: "stale", owner: "desktop", status: process.StatusStale, want: protocol.CodeRouterStateStale},
	} {
		t.Run(tt.name, func(t *testing.T) {
			fixture := newFixture(t)
			fixture.managerState = fixture.desktopState(101)
			fixture.managerState.Owner = tt.owner
			fixture.ownState()
			fixture.validate = func(process.Identity, string) (process.Status, error) { return tt.status, nil }
			protocolErr := fixture.manager().Stop(context.Background())
			if protocolErr == nil || protocolErr.Code != tt.want {
				t.Fatalf("error = %v", protocolErr)
			}
			if fixture.signals != 0 {
				t.Fatalf("signals = %d", fixture.signals)
			}
		})
	}
}

func TestStopRejectsAnotherDesktopManagerSession(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = fixture.desktopState(101)
	protocolErr := fixture.manager().Stop(context.Background())
	if protocolErr == nil || protocolErr.Code != protocol.CodeRouterNotOwned {
		t.Fatalf("error = %v", protocolErr)
	}
	if fixture.signals != 0 {
		t.Fatalf("signals = %d", fixture.signals)
	}
}

func TestReclaimRequiresLockAbsentManagerSessionAndRouterIdentity(t *testing.T) {
	for _, tt := range []struct {
		name     string
		session  string
		statuses []process.Status
		lockErr  error
		wantOK   bool
	}{
		{name: "success", session: "session", statuses: []process.Status{process.StatusAbsent, process.StatusGenuine}, wantOK: true},
		{name: "competing manager lock", session: "session", lockErr: state.ErrLocked},
		{name: "previous manager alive", session: "session", statuses: []process.Status{process.StatusGenuine}},
		{name: "wrong session", session: "other"},
		{name: "stale router", session: "session", statuses: []process.Status{process.StatusAbsent, process.StatusStale}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			fixture := newFixture(t)
			fixture.config.SessionID = tt.session
			fixture.managerState = fixture.desktopState(101)
			fixture.deps.AcquireLock = func(string) (io.Closer, error) {
				if tt.lockErr != nil {
					return nil, tt.lockErr
				}
				fixture.lockHeld = true
				return closerFunc(func() error { fixture.lockClosed = true; return nil }), nil
			}
			index := 0
			fixture.validate = func(process.Identity, string) (process.Status, error) {
				if index >= len(tt.statuses) {
					return process.StatusStale, nil
				}
				status := tt.statuses[index]
				index++
				return status, nil
			}
			value, protocolErr := fixture.manager().Reclaim()
			if tt.wantOK {
				if protocolErr != nil {
					t.Fatal(protocolErr)
				}
				if value.ManagerPID != fixture.config.ManagerIdentity.PID || value.ManagerProcessStartedAt != fixture.config.ManagerIdentity.StartedAt || value.ManagerProcessExecutable != fixture.config.ManagerIdentity.Executable || fixture.writes != 1 {
					t.Fatalf("value=%+v writes=%d", value, fixture.writes)
				}
				if fixture.lockClosed {
					t.Fatal("successful reclaim released ownership lock")
				}
			} else if protocolErr == nil {
				t.Fatal("reclaim unexpectedly succeeded")
			} else {
				if fixture.writes != 0 || fixture.signals != 0 || fixture.desktopLaunches != 0 {
					t.Fatalf("failed reclaim writes=%d signals=%d launches=%d", fixture.writes, fixture.signals, fixture.desktopLaunches)
				}
			}
		})
	}
}

func TestMonitorParentStopsOnCompleteIdentityMismatch(t *testing.T) {
	fixture := newFixture(t)
	fixture.managerState = fixture.desktopState(101)
	fixture.ownState()
	call := 0
	fixture.validate = func(identity process.Identity, _ string) (process.Status, error) {
		call++
		if identity.PID == fixture.config.ParentIdentity.PID {
			return process.StatusStale, nil
		}
		if call > 2 {
			return process.StatusAbsent, nil
		}
		return process.StatusGenuine, nil
	}
	protocolErr := fixture.manager().MonitorParent(context.Background())
	if protocolErr != nil {
		t.Fatal(protocolErr)
	}
	if fixture.signals != 1 {
		t.Fatalf("signals = %d, want graceful router stop", fixture.signals)
	}
}

func TestUnexpectedExitAndRecentOutputAreBounded(t *testing.T) {
	fixture := newFixture(t)
	fixture.config.RecentOutputBytes = 8
	wait := make(chan error, 1)
	fixture.deps.LaunchDesktop = func(_ string, _ []string, _ []string, output io.Writer) (foregroundProcess, error) {
		_, _ = output.Write([]byte("0123456789"))
		return &fakeChild{pid: 101, wait: wait}, nil
	}
	manager := fixture.manager()
	if _, protocolErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop); protocolErr != nil {
		t.Fatal(protocolErr)
	}
	wait <- errors.New("crashed")
	select {
	case event := <-manager.UnexpectedExit():
		if event.Err == nil || !strings.Contains(event.Err.Error(), "crashed") || event.Identity.PID != 101 {
			t.Fatalf("exit = %+v", event)
		}
		if event.RecentOutput != "23456789" {
			t.Fatalf("event recent output = %q", event.RecentOutput)
		}
	case <-time.After(time.Second):
		t.Fatal("unexpected exit was not reported")
	}
	if got := manager.RecentOutput(); got != "23456789" {
		t.Fatalf("recent output = %q", got)
	}
}

func TestStartupFailureAndIntentionalStopDoNotReportUnexpectedExit(t *testing.T) {
	t.Run("startup failure", func(t *testing.T) {
		fixture := newFixture(t)
		wait := make(chan error, 1)
		fixture.deps.LaunchDesktop = func(string, []string, []string, io.Writer) (foregroundProcess, error) {
			wait <- errors.New("startup crash")
			return &fakeChild{pid: 101, wait: wait}, nil
		}
		fixture.deps.Verify = func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error) {
			return discovery.Version{}, discovery.Health{}, errors.New("not ready")
		}
		manager := fixture.manager()
		if _, protocolErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop); protocolErr == nil || protocolErr.Code != protocol.CodeRouterStartFailed {
			t.Fatalf("start error = %v", protocolErr)
		}
		select {
		case event := <-manager.UnexpectedExit():
			t.Fatalf("startup failure reported as unexpected: %+v", event)
		default:
		}
	})

	t.Run("intentional stop", func(t *testing.T) {
		fixture := newFixture(t)
		wait := make(chan error, 1)
		fixture.deps.LaunchDesktop = func(string, []string, []string, io.Writer) (foregroundProcess, error) {
			return &fakeChild{pid: 101, wait: wait}, nil
		}
		statuses := []process.Status{process.StatusGenuine, process.StatusGenuine, process.StatusAbsent}
		fixture.validate = func(process.Identity, string) (process.Status, error) {
			if len(statuses) == 0 {
				return process.StatusAbsent, nil
			}
			status := statuses[0]
			statuses = statuses[1:]
			return status, nil
		}
		manager := fixture.manager()
		if _, protocolErr := manager.Start(context.Background(), protocol.RouterOwnerDesktop); protocolErr != nil {
			t.Fatal(protocolErr)
		}
		fixture.managerState = fixture.desktopState(101)
		fixture.ownState()
		fixture.deps.Signal = func(process.Identity, string, os.Signal) error {
			wait <- nil
			return nil
		}
		// Stop uses the dependencies captured when Manager was constructed.
		manager.deps.Signal = fixture.deps.Signal
		if protocolErr := manager.Stop(context.Background()); protocolErr != nil {
			t.Fatal(protocolErr)
		}
		select {
		case event := <-manager.UnexpectedExit():
			t.Fatalf("intentional stop reported as unexpected: %+v", event)
		case <-time.After(20 * time.Millisecond):
		}
	})
}

type fixture struct {
	t                             *testing.T
	config                        Config
	deps                          Dependencies
	discovered                    discovery.Result
	managerState                  state.RouterState
	cliState                      state.RouterState
	writes, signals, killSignals  int
	ownedChildKills               int
	desktopLaunches               int
	removed, lockHeld, lockClosed bool
	validate                      func(process.Identity, string) (process.Status, error)
	mu                            sync.Mutex
}

func newFixture(t *testing.T) *fixture {
	t.Helper()
	dir := t.TempDir()
	f := &fixture{t: t}
	f.config = Config{
		RouterPath: "/router", ListenAddr: "127.0.0.1:19099", DesktopStatePath: filepath.Join(dir, "desktop.json"),
		CLIStatePath: filepath.Join(dir, "cli.json"), DesktopLockPath: filepath.Join(dir, "owner.lock"), DesktopLogPath: filepath.Join(dir, "desktop.log"),
		CLILogPath: filepath.Join(dir, "cli.log"), SessionID: "session", ManagerIdentity: process.Identity{PID: 7, StartedAt: "manager-new", Executable: "/manager"},
		ParentIdentity: process.Identity{PID: 8, StartedAt: "parent-start", Executable: "/desktop"}, ManagerVersion: "v1", DeploymentID: "prod-a",
		ManagementProtocolVersion: "1", PollInterval: time.Millisecond,
	}
	f.discovered = discovery.Result{Classification: discovery.Absent}
	f.validate = func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil }
	f.deps = Dependencies{
		Discover: func(context.Context, protocol.RouterOwner) discovery.Result { return f.discovered },
		Inspect: func(pid int) (process.Identity, error) {
			return process.Identity{PID: pid, StartedAt: "router-start", Executable: "/router"}, nil
		},
		Validate: func(identity process.Identity, binary string) (process.Status, error) {
			return f.validate(identity, binary)
		},
		Signal: func(_ process.Identity, _ string, signal os.Signal) error {
			f.signals++
			if signal == os.Kill {
				f.killSignals++
			}
			return nil
		},
		LaunchDesktop: func(_ string, _ []string, _ []string, output io.Writer) (foregroundProcess, error) {
			f.desktopLaunches++
			_, _ = output.Write([]byte("owned child output"))
			wait := make(chan error, 1)
			return &fakeChild{pid: 101, wait: wait, killFunc: func() error {
				f.ownedChildKills++
				wait <- errors.New("killed")
				return nil
			}}, nil
		},
		LaunchDetached: func(string, []string, string) (int, error) { return 202, nil },
		ReadState: func(path string) (state.RouterState, error) {
			if path == f.config.DesktopStatePath && f.managerState.PID != 0 {
				return f.managerState, nil
			}
			if path == f.config.CLIStatePath && f.cliState.PID != 0 {
				return f.cliState, nil
			}
			return state.RouterState{}, os.ErrNotExist
		},
		WriteState: func(path string, value state.RouterState) error {
			if f.lockHeld && f.lockClosed {
				t.Fatal("state written after ownership lock was released")
			}
			f.writes++
			if path == f.config.DesktopStatePath {
				f.managerState = value
			} else {
				f.cliState = value
			}
			return nil
		},
		RemoveState: func(string) error { f.removed = true; return nil },
		AcquireLock: func(string) (io.Closer, error) {
			f.lockHeld = true
			return closerFunc(func() error { f.lockClosed = true; return nil }), nil
		},
		Environ: func() []string { return []string{"PATH=/bin"} },
		OpenLog: func(string) (*os.File, error) {
			return os.OpenFile(filepath.Join(dir, "output.log"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
		},
		Verify: func(context.Context, string, int, string, string) (discovery.Version, discovery.Health, error) {
			return discovery.Version{Version: "v1", PID: 101}, discovery.Health{Status: "ok"}, nil
		},
		Sleep: func(context.Context, time.Duration) error { return nil },
		Now:   func() time.Time { return time.Date(2026, 7, 12, 0, 0, 0, 0, time.UTC) },
	}
	return f
}

func (f *fixture) manager() *Manager { return New(f.config, f.deps) }
func (f *fixture) desktopState(pid int) state.RouterState {
	return state.RouterState{PID: pid, Owner: "desktop", ListenAddr: "http://127.0.0.1:19099", BinaryPath: "/router", LogPath: f.config.DesktopLogPath,
		ProcessStartedAt: "router-start", ProcessExecutable: "/router", DesktopSessionID: "session", ManagerPID: 6,
		ManagerProcessStartedAt: "manager-old", ManagerProcessExecutable: "/manager", ManagerVersion: "v1", RouterVersion: "v1", DeploymentID: "prod-a", ManagementProtocolVersion: "1"}
}

func (f *fixture) ownState() {
	f.managerState.DesktopSessionID = f.config.SessionID
	f.managerState.ManagerPID = f.config.ManagerIdentity.PID
	f.managerState.ManagerProcessStartedAt = f.config.ManagerIdentity.StartedAt
	f.managerState.ManagerProcessExecutable = f.config.ManagerIdentity.Executable
}

type fakeChild struct {
	pid      int
	wait     chan error
	waitFunc func() error
	killFunc func() error
	killOnce sync.Once
}

type countingInfiniteReader struct {
	read int
}

func (r *countingInfiniteReader) Read(p []byte) (int, error) {
	for i := range p {
		p[i] = 'x'
	}
	r.read += len(p)
	return len(p), nil
}

func (p *fakeChild) PID() int { return p.pid }
func (p *fakeChild) Wait() error {
	if p.waitFunc != nil {
		return p.waitFunc()
	}
	return <-p.wait
}
func (p *fakeChild) Kill() error {
	if p.killFunc != nil {
		return p.killFunc()
	}
	p.killOnce.Do(func() {
		if p.wait != nil {
			p.wait <- errors.New("killed")
		}
	})
	return nil
}

type closerFunc func() error

func (f closerFunc) Close() error { return f() }
func containsArg(args []string, want string) bool {
	for _, arg := range args {
		if arg == want {
			return true
		}
	}
	return false
}
