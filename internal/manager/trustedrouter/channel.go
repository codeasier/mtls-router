package trustedrouter

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"sync"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/apikeyusage"
	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/modelcatalog"
	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/manager/state"
)

const versionBodyLimit = 64 << 10

// Channel validates router identity and fetches a catalog over one TCP
// connection. A server-requested redial is rejected before a second request can
// transmit its Authorization header.
type Channel struct {
	Simplify        bool
	DialContext     func(context.Context, string, string) (net.Conn, error)
	ValidateProcess func(process.Identity, string) (process.Status, error)
}

// Fetch performs the channel-bound /version and /v1/models exchange.
func (c Channel) Fetch(ctx context.Context, listener Listener, trusted discovery.Result, apiKey string) ([]string, *protocol.Error) {
	transport, bindErr := c.bind(ctx, listener, trusted)
	if bindErr != nil {
		return nil, bindErr
	}
	defer transport.CloseIdleConnections()
	models, fetchErr := modelcatalog.New(transport, c.Simplify).Fetch(ctx, modelcatalog.Request{
		URL: listener.APIBaseURL + "/models", APIKey: apiKey,
	})
	if fetchErr != nil {
		return nil, &protocol.Error{Code: modelcatalog.CodeOf(fetchErr), Message: fetchErr.Error()}
	}
	return models, nil
}

// FetchUsage performs the channel-bound /version and /v1/usage exchange.
func (c Channel) FetchUsage(ctx context.Context, listener Listener, trusted discovery.Result, period apikeyusage.Period, apiKey string) (apikeyusage.Snapshot, *protocol.Error) {
	transport, bindErr := c.bind(ctx, listener, trusted)
	if bindErr != nil {
		return apikeyusage.Snapshot{}, bindErr
	}
	defer transport.CloseIdleConnections()
	snapshot, fetchErr := apikeyusage.New(transport).Fetch(ctx, apikeyusage.Request{
		URL: listener.APIBaseURL + "/usage", Period: period, APIKey: apiKey,
	})
	if fetchErr != nil {
		return apikeyusage.Snapshot{}, &protocol.Error{Code: apikeyusage.CodeOf(fetchErr), Message: fetchErr.Error()}
	}
	return snapshot, nil
}

func (c Channel) bind(ctx context.Context, listener Listener, trusted discovery.Result) (*http.Transport, *protocol.Error) {
	dial := c.DialContext
	if dial == nil {
		dial = (&net.Dialer{Timeout: 5 * time.Second}).DialContext
	}
	validate := c.ValidateProcess
	if validate == nil {
		validate = process.Validate
	}

	var dialMu sync.Mutex
	dialed := false
	transport := &http.Transport{
		Proxy: nil,
		DialContext: func(dialCtx context.Context, network, address string) (net.Conn, error) {
			dialMu.Lock()
			if dialed || network != "tcp" || address != listener.Authority {
				dialMu.Unlock()
				return nil, errors.New("trusted router connection cannot redial")
			}
			dialed = true
			dialMu.Unlock()
			return dial(dialCtx, network, address)
		},
		MaxConnsPerHost:     1,
		MaxIdleConns:        1,
		MaxIdleConnsPerHost: 1,
		IdleConnTimeout:     5 * time.Second,
		DisableCompression:  true,
	}
	httpClient := &http.Client{
		Transport: transport,
		Timeout:   15 * time.Second,
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}

	versionRequest, err := http.NewRequestWithContext(ctx, http.MethodGet, listener.RouterBaseURL+"/version", nil)
	if err != nil {
		transport.CloseIdleConnections()
		return nil, discoveryFailed()
	}
	response, err := httpClient.Do(versionRequest)
	if err != nil {
		transport.CloseIdleConnections()
		return nil, discoveryFailed()
	}
	if response.StatusCode != http.StatusOK {
		response.Body.Close()
		transport.CloseIdleConnections()
		return nil, discoveryFailed()
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, versionBodyLimit+1))
	closeErr := response.Body.Close()
	if err != nil || closeErr != nil || len(body) > versionBodyLimit {
		transport.CloseIdleConnections()
		return nil, discoveryFailed()
	}
	var versionInfo discovery.Version
	decoderErr := json.Unmarshal(body, &versionInfo)
	if decoderErr != nil || !versionMatches(versionInfo, trusted) {
		transport.CloseIdleConnections()
		return nil, staleCatalog()
	}
	identity := process.Identity{PID: trusted.State.PID, StartedAt: trusted.State.ProcessStartedAt, Executable: trusted.State.ProcessExecutable}
	status, validateErr := validate(identity, trusted.State.BinaryPath)
	if validateErr != nil || status != process.StatusGenuine {
		transport.CloseIdleConnections()
		return nil, staleCatalog()
	}
	return transport, nil
}

func versionMatches(remote discovery.Version, trusted discovery.Result) bool {
	value := trusted.State
	return remote.PID > 0 && remote.PID == value.PID &&
		remote.DeploymentID == value.DeploymentID && remote.DeploymentID == trusted.Version.DeploymentID &&
		remote.ManagementProtocolVersion == value.ManagementProtocolVersion &&
		remote.ManagementProtocolVersion == trusted.Version.ManagementProtocolVersion
}

func trustedStateMatches(found discovery.Result, listener Listener, deploymentID, protocolVersion string) bool {
	if found.Classification != discovery.DesktopOwned && found.Classification != discovery.ExternalCompatible && found.Classification != discovery.Degraded {
		return false
	}
	value := found.State
	return (value.Owner == "cli" || value.Owner == "desktop") && value.PID > 0 &&
		value.ListenAddr == listener.RouterBaseURL && found.ListenAddr == listener.RouterBaseURL &&
		value.DeploymentID == deploymentID && found.Version.DeploymentID == deploymentID &&
		value.ManagementProtocolVersion == protocolVersion && found.Version.ManagementProtocolVersion == protocolVersion
}

func stateFromStart(value state.RouterState, listener Listener, deploymentID, protocolVersion string) bool {
	return value.PID > 0 && (value.Owner == "cli" || value.Owner == "desktop") &&
		value.ListenAddr == listener.RouterBaseURL && value.DeploymentID == deploymentID &&
		value.ManagementProtocolVersion == protocolVersion
}

func discoveryFailed() *protocol.Error {
	return &protocol.Error{Code: protocol.CodeModelDiscoveryFailed, Message: "model catalog request failed"}
}

func staleCatalog() *protocol.Error {
	return &protocol.Error{Code: protocol.CodeModelCatalogStale, Message: "trusted router identity changed"}
}
