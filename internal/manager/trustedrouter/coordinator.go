package trustedrouter

import (
	"context"

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
	if owner == protocol.RouterOwnerDesktop && (c.DesktopEligible == nil || !c.DesktopEligible()) {
		return Result{}, &protocol.Error{Code: protocol.CodeInvalidParams, Message: "desktop owner requires a verified desktop session"}
	}
	found := c.Discover(ctx)
	if found.Classification == discovery.Absent {
		if c.AbsentStartOK != nil && !c.AbsentStartOK() {
			return Result{}, &protocol.Error{Code: protocol.CodeRouterStateStale, Message: "router restart requires explicit lifecycle recovery"}
		}
		started, startErr := c.Lifecycle.Start(ctx, owner)
		if startErr != nil && startErr.Code != protocol.CodeRouterDegraded {
			return Result{}, lifecycleProtocolError(startErr)
		}
		if !stateFromStart(started, c.Listener, c.DeploymentID, c.ProtocolVersion) {
			return Result{}, staleCatalog()
		}
		found = c.Discover(ctx)
	}
	if !trustedStateMatches(found, c.Listener, c.DeploymentID, c.ProtocolVersion) {
		switch found.Classification {
		case discovery.Stale:
			return Result{}, &protocol.Error{Code: protocol.CodeRouterStateStale, Message: "router state is stale"}
		case discovery.LegacyManaged:
			return Result{}, &protocol.Error{Code: protocol.CodeRouterLegacyManaged, Message: "legacy desktop router requires explicit migration"}
		case discovery.UnknownOccupant:
			return Result{}, &protocol.Error{Code: protocol.CodePortOccupied, Message: "router port is occupied"}
		case discovery.Absent:
			return Result{}, &protocol.Error{Code: protocol.CodeRouterNotFound, Message: "router was not found"}
		default:
			return Result{}, staleCatalog()
		}
	}
	models, fetchErr := c.Channel.Fetch(ctx, c.Listener, found, apiKey)
	if fetchErr != nil {
		return Result{}, fetchErr
	}
	return Result{Models: models, Binding: Binding{
		RouterBaseURL: c.Listener.RouterBaseURL, APIBaseURL: c.Listener.APIBaseURL,
		DeploymentID: c.DeploymentID, ProtocolVersion: c.ProtocolVersion,
	}}, nil
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
