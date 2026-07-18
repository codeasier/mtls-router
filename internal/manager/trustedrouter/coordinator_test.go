package trustedrouter

import (
	"context"
	"errors"
	"net"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/lifecycle"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

type lifecycleStub struct {
	calls int
	owner protocol.RouterOwner
	state state.RouterState
	err   *lifecycle.Error
}

func (s *lifecycleStub) Start(_ context.Context, owner protocol.RouterOwner) (state.RouterState, *lifecycle.Error) {
	s.calls++
	s.owner = owner
	return s.state, s.err
}

func TestCoordinatorAbsentStartsOnceUnderRequestedOwner(t *testing.T) {
	listener, _ := NormalizeListener("127.0.0.1:19099")
	trusted := trustedFixture(listener)
	discoveries := 0
	starter := &lifecycleStub{state: trusted.State}
	coordinator := Coordinator{
		Listener: listener, DeploymentID: "prod-a", ProtocolVersion: "2", Lifecycle: starter,
		DesktopEligible: func() bool { return true }, AbsentStartOK: func() bool { return true },
		Discover: func(context.Context) discovery.Result {
			discoveries++
			if discoveries == 1 {
				return discovery.Result{Classification: discovery.Absent}
			}
			return trusted
		},
		Channel: Channel{
			DialContext: func(context.Context, string, string) (net.Conn, error) {
				return nil, errors.New("stop after trust test")
			},
			ValidateProcess: genuineProcess,
		},
	}
	_, err := coordinator.Fetch(context.Background(), protocol.RouterOwnerDesktop, "secret")
	if err == nil || err.Code != protocol.CodeModelDiscoveryFailed || starter.calls != 1 || starter.owner != protocol.RouterOwnerDesktop || discoveries != 2 {
		t.Fatalf("error=%+v starts=%d owner=%q discoveries=%d", err, starter.calls, starter.owner, discoveries)
	}
}

func TestCoordinatorRejectsUnsafeStatesWithoutStart(t *testing.T) {
	listener, _ := NormalizeListener("127.0.0.1:19099")
	for _, test := range []struct {
		name           string
		classification discovery.Classification
		startOK        bool
		code           protocol.ErrorCode
	}{
		{name: "unknown", classification: discovery.UnknownOccupant, startOK: true, code: protocol.CodePortOccupied},
		{name: "stale", classification: discovery.Stale, startOK: true, code: protocol.CodeRouterStateStale},
		{name: "unexpected exit latch", classification: discovery.Absent, startOK: false, code: protocol.CodeRouterStateStale},
	} {
		t.Run(test.name, func(t *testing.T) {
			starter := &lifecycleStub{}
			coordinator := Coordinator{
				Listener: listener, DeploymentID: "prod-a", ProtocolVersion: "2", Lifecycle: starter,
				DesktopEligible: func() bool { return true }, AbsentStartOK: func() bool { return test.startOK },
				Discover: func(context.Context) discovery.Result { return discovery.Result{Classification: test.classification} },
			}
			_, err := coordinator.Fetch(context.Background(), protocol.RouterOwnerCLI, "secret")
			if err == nil || err.Code != test.code || starter.calls != 0 {
				t.Fatalf("error=%+v starts=%d", err, starter.calls)
			}
		})
	}
}

func TestCoordinatorRejectsDesktopOwnerBeforeDiscovery(t *testing.T) {
	discoveries := 0
	coordinator := Coordinator{DesktopEligible: func() bool { return false }, Discover: func(context.Context) discovery.Result {
		discoveries++
		return discovery.Result{}
	}}
	_, err := coordinator.Fetch(context.Background(), protocol.RouterOwnerDesktop, channelKeyCanary)
	if err == nil || err.Code != protocol.CodeInvalidParams || discoveries != 0 {
		t.Fatalf("error=%+v discoveries=%d", err, discoveries)
	}
}

func TestCoordinatorRejectsChangedWriteBindingBeforeDiscovery(t *testing.T) {
	listener, _ := NormalizeListener("127.0.0.1:19099")
	discoveries := 0
	coordinator := Coordinator{Listener: listener, DeploymentID: "prod-a", ProtocolVersion: "2", Discover: func(context.Context) discovery.Result {
		discoveries++
		return discovery.Result{}
	}}
	_, err := coordinator.Revalidate(context.Background(), protocol.RouterOwnerCLI, channelKeyCanary, Binding{
		RouterBaseURL: "http://127.0.0.1:19100", APIBaseURL: "http://127.0.0.1:19100/v1", DeploymentID: "prod-a", ProtocolVersion: "2",
	})
	if err == nil || err.Code != protocol.CodeModelCatalogStale || discoveries != 0 {
		t.Fatalf("error=%+v discoveries=%d", err, discoveries)
	}
}

func TestTrustedStateMatchesOnlyExactOwnedOrDegradedBinding(t *testing.T) {
	listener, _ := NormalizeListener("127.0.0.1:19099")
	base := trustedFixture(listener)
	for _, classification := range []discovery.Classification{discovery.ExternalCompatible, discovery.DesktopOwned, discovery.Degraded} {
		found := base
		found.Classification = classification
		if classification == discovery.DesktopOwned {
			found.Owner = "desktop"
			found.State.Owner = "desktop"
		}
		if !trustedStateMatches(found, listener, "prod-a", "2") {
			t.Fatalf("classification %q was not trusted", classification)
		}
	}
	for _, mutate := range []func(*discovery.Result){
		func(found *discovery.Result) { found.ListenAddr = "http://127.0.0.1:19100" },
		func(found *discovery.Result) { found.State.ListenAddr = "http://127.0.0.1:19100" },
		func(found *discovery.Result) { found.Version.DeploymentID = "other" },
		func(found *discovery.Result) { found.State.DeploymentID = "other" },
		func(found *discovery.Result) { found.Version.ManagementProtocolVersion = "1" },
		func(found *discovery.Result) { found.State.ManagementProtocolVersion = "1" },
		func(found *discovery.Result) { found.State.Owner = "unknown" },
	} {
		found := base
		mutate(&found)
		if trustedStateMatches(found, listener, "prod-a", "2") {
			t.Fatalf("mismatched state was trusted: %+v", found)
		}
	}
}

func TestCoordinatorTrustedRouterStateOwnerAddressDeploymentMatrix(t *testing.T) {
	listener, _ := NormalizeListener("127.0.0.1:19099")
	base := trustedFixture(listener)
	tests := []struct {
		name string
		edit func(*discovery.Result)
		want bool
	}{
		{name: "existing CLI", want: true},
		{name: "existing desktop", edit: func(found *discovery.Result) {
			found.Classification = discovery.DesktopOwned
			found.Owner = "desktop"
			found.State.Owner = "desktop"
		}, want: true},
		{name: "degraded CLI", edit: func(found *discovery.Result) { found.Classification = discovery.Degraded }, want: true},
		{name: "unknown owner", edit: func(found *discovery.Result) { found.State.Owner = "service" }},
		{name: "changed advertised address", edit: func(found *discovery.Result) { found.ListenAddr = "http://127.0.0.1:19100" }},
		{name: "changed state address", edit: func(found *discovery.Result) { found.State.ListenAddr = "http://127.0.0.1:19100" }},
		{name: "wrong deployment response", edit: func(found *discovery.Result) { found.Version.DeploymentID = "other" }},
		{name: "wrong deployment state", edit: func(found *discovery.Result) { found.State.DeploymentID = "other" }},
		{name: "mixed protocol response", edit: func(found *discovery.Result) { found.Version.ManagementProtocolVersion = "1" }},
		{name: "mixed protocol state", edit: func(found *discovery.Result) { found.State.ManagementProtocolVersion = "1" }},
		{name: "missing PID", edit: func(found *discovery.Result) { found.State.PID = 0 }},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			found := base
			if test.edit != nil {
				test.edit(&found)
			}
			if got := trustedStateMatches(found, listener, "prod-a", "2"); got != test.want {
				t.Fatalf("trustedStateMatches() = %t, want %t: %+v", got, test.want, found)
			}
		})
	}
}

func TestCoordinatorAbsentStartAndRestartMatrix(t *testing.T) {
	listener, _ := NormalizeListener("127.0.0.1:19099")
	trusted := trustedFixture(listener)
	for _, test := range []struct {
		name       string
		owner      protocol.RouterOwner
		eligible   bool
		restartOK  bool
		started    state.RouterState
		after      discovery.Result
		wantStarts int
		wantCode   protocol.ErrorCode
	}{
		{name: "CLI absent starts", owner: protocol.RouterOwnerCLI, eligible: true, restartOK: true, started: trusted.State, after: trusted, wantStarts: 1, wantCode: protocol.CodeModelDiscoveryFailed},
		{name: "desktop absent starts", owner: protocol.RouterOwnerDesktop, eligible: true, restartOK: true, started: func() state.RouterState { value := trusted.State; value.Owner = "desktop"; return value }(), after: func() discovery.Result {
			value := trusted
			value.Classification = discovery.DesktopOwned
			value.Owner = "desktop"
			value.State.Owner = "desktop"
			return value
		}(), wantStarts: 1, wantCode: protocol.CodeModelDiscoveryFailed},
		{name: "desktop ineligible", owner: protocol.RouterOwnerDesktop, restartOK: true, wantCode: protocol.CodeInvalidParams},
		{name: "unexpected-exit restart prohibited", owner: protocol.RouterOwnerCLI, eligible: true, wantCode: protocol.CodeRouterStateStale},
		{name: "start returns wrong owner", owner: protocol.RouterOwnerCLI, eligible: true, restartOK: true, started: func() state.RouterState { value := trusted.State; value.Owner = "other"; return value }(), wantStarts: 1, wantCode: protocol.CodeModelCatalogStale},
		{name: "start returns wrong address", owner: protocol.RouterOwnerCLI, eligible: true, restartOK: true, started: func() state.RouterState {
			value := trusted.State
			value.ListenAddr = "http://127.0.0.1:19100"
			return value
		}(), wantStarts: 1, wantCode: protocol.CodeModelCatalogStale},
		{name: "start returns wrong deployment", owner: protocol.RouterOwnerCLI, eligible: true, restartOK: true, started: func() state.RouterState { value := trusted.State; value.DeploymentID = "other"; return value }(), wantStarts: 1, wantCode: protocol.CodeModelCatalogStale},
		{name: "start returns wrong protocol", owner: protocol.RouterOwnerCLI, eligible: true, restartOK: true, started: func() state.RouterState { value := trusted.State; value.ManagementProtocolVersion = "1"; return value }(), wantStarts: 1, wantCode: protocol.CodeModelCatalogStale},
	} {
		t.Run(test.name, func(t *testing.T) {
			starter := &lifecycleStub{state: test.started}
			discoveries := 0
			coordinator := Coordinator{
				Listener: listener, DeploymentID: "prod-a", ProtocolVersion: "2", Lifecycle: starter,
				DesktopEligible: func() bool { return test.eligible }, AbsentStartOK: func() bool { return test.restartOK },
				Discover: func(context.Context) discovery.Result {
					discoveries++
					if discoveries == 1 {
						return discovery.Result{Classification: discovery.Absent}
					}
					return test.after
				},
				Channel: Channel{DialContext: func(context.Context, string, string) (net.Conn, error) {
					return nil, errors.New("stop after trust validation")
				}, ValidateProcess: genuineProcess},
			}
			_, err := coordinator.Fetch(context.Background(), test.owner, channelKeyCanary)
			if err == nil || err.Code != test.wantCode || starter.calls != test.wantStarts {
				t.Fatalf("error=%+v starts=%d, want code=%q starts=%d", err, starter.calls, test.wantCode, test.wantStarts)
			}
		})
	}
}
