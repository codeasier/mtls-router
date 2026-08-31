package trustedrouter

import (
	"context"

	"github.com/codeasier/mtls-router/internal/manager/apikeyusage"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/lifecycle"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

// Lifecycle is the absent-only startup capability used by secret discovery.
type Lifecycle interface {
	Start(context.Context, protocol.RouterOwner) (state.RouterState, *lifecycle.Error)
}

// Binding is the trusted identity embedded into catalog tokens and rechecked
// before future secret-bearing write preflight.
type Binding struct {
	RouterBaseURL   string
	APIBaseURL      string
	DeploymentID    string
	ProtocolVersion string
}

type Result struct {
	Models  []string
	Binding Binding
}

type UsageResult struct {
	Snapshot apikeyusage.Snapshot
	Binding  Binding
}

type Coordinator struct {
	Listener        Listener
	DeploymentID    string
	ProtocolVersion string
	Discover        func(context.Context) discovery.Result
	Lifecycle       Lifecycle
	Channel         Channel
	DesktopEligible func() bool
	AbsentStartOK   func() bool
}

// Fetch reuses a trusted router or starts exactly one absent router under the
// requested owner. Stale and unknown states never enter lifecycle startup.
func (c *Coordinator) Fetch(ctx context.Context, owner protocol.RouterOwner, apiKey string) (Result, *protocol.Error) {
	found, err := c.establish(ctx, owner)
	if err != nil {
		return Result{}, err
	}
	models, fetchErr := c.Channel.Fetch(ctx, c.Listener, found, apiKey)
	if fetchErr != nil {
		return Result{}, fetchErr
	}
	return Result{Models: models, Binding: c.binding()}, nil
}

// FetchUsage reuses the same trust path as Fetch, then requests /v1/usage.
func (c *Coordinator) FetchUsage(ctx context.Context, owner protocol.RouterOwner, period apikeyusage.Period, apiKey string) (UsageResult, *protocol.Error) {
	found, err := c.establish(ctx, owner)
	if err != nil {
		return UsageResult{}, err
	}
	snapshot, fetchErr := c.Channel.FetchUsage(ctx, c.Listener, found, period, apiKey)
	if fetchErr != nil {
		return UsageResult{}, fetchErr
	}
	return UsageResult{Snapshot: snapshot, Binding: c.binding()}, nil
}

func (c *Coordinator) binding() Binding {
	return Binding{
		RouterBaseURL: c.Listener.RouterBaseURL, APIBaseURL: c.Listener.APIBaseURL,
		DeploymentID: c.DeploymentID, ProtocolVersion: c.ProtocolVersion,
	}
}

func (c *Coordinator) establish(ctx context.Context, owner protocol.RouterOwner) (discovery.Result, *protocol.Error) {
	if owner == protocol.RouterOwnerDesktop && (c.DesktopEligible == nil || !c.DesktopEligible()) {
		return discovery.Result{}, &protocol.Error{Code: protocol.CodeInvalidParams, Message: "desktop owner requires a verified desktop session"}
	}
	found := c.Discover(ctx)
	if found.Classification == discovery.Absent {
		if c.AbsentStartOK != nil && !c.AbsentStartOK() {
			return discovery.Result{}, &protocol.Error{Code: protocol.CodeRouterStateStale, Message: "router restart requires explicit lifecycle recovery"}
		}
		started, startErr := c.Lifecycle.Start(ctx, owner)
		if startErr != nil && startErr.Code != protocol.CodeRouterDegraded {
			return discovery.Result{}, lifecycleProtocolError(startErr)
		}
		if !stateFromStart(started, c.Listener, c.DeploymentID, c.ProtocolVersion) {
			return discovery.Result{}, staleCatalog()
		}
		found = c.Discover(ctx)
	}
	if !trustedStateMatches(found, c.Listener, c.DeploymentID, c.ProtocolVersion) {
		switch found.Classification {
		case discovery.Stale:
			return discovery.Result{}, &protocol.Error{Code: protocol.CodeRouterStateStale, Message: "router state is stale"}
		case discovery.LegacyManaged:
			return discovery.Result{}, &protocol.Error{Code: protocol.CodeRouterLegacyManaged, Message: "legacy desktop router requires explicit migration"}
		case discovery.UnknownOccupant:
			return discovery.Result{}, &protocol.Error{Code: protocol.CodePortOccupied, Message: "router port is occupied"}
		case discovery.Absent:
			return discovery.Result{}, &protocol.Error{Code: protocol.CodeRouterNotFound, Message: "router was not found"}
		default:
			return discovery.Result{}, staleCatalog()
		}
	}
	return found, nil
}

// Revalidate fetches through the same trust path and rejects any deployment or
// address change before write-time callers create transaction artifacts.
func (c *Coordinator) Revalidate(ctx context.Context, owner protocol.RouterOwner, apiKey string, binding Binding) ([]string, *protocol.Error) {
	if binding.RouterBaseURL != c.Listener.RouterBaseURL || binding.APIBaseURL != c.Listener.APIBaseURL ||
		binding.DeploymentID != c.DeploymentID || binding.ProtocolVersion != c.ProtocolVersion {
		return nil, staleCatalog()
	}
	result, err := c.Fetch(ctx, owner, apiKey)
	if err != nil {
		return nil, err
	}
	if result.Binding != binding {
		return nil, staleCatalog()
	}
	return result.Models, nil
}

func lifecycleProtocolError(err *lifecycle.Error) *protocol.Error {
	if err == nil {
		return nil
	}
	return &protocol.Error{Code: err.Code, Message: "router could not be safely started"}
}
