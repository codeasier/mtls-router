// Package discovery classifies the process listening on the router endpoint by
// correlating HTTP metadata with durable state and operating-system identity.
package discovery

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"github.com/codeasier/mtls-router/internal/manager/state"
	"github.com/codeasier/mtls-router/internal/version"
)

type Classification string

const (
	DesktopOwned       Classification = "desktop_owned"
	ExternalCompatible Classification = "external_compatible"
	Degraded           Classification = "degraded"
	Stale              Classification = "stale"
	Absent             Classification = "absent"
	UnknownOccupant    Classification = "unknown_occupant"
)

type Version struct {
	Version                   string `json:"version"`
	PID                       int    `json:"pid"`
	DeploymentID              string `json:"deployment_id"`
	ManagementProtocolVersion string `json:"management_protocol_version"`
}

type Health struct {
	Status   string `json:"status"`
	Upstream string `json:"upstream"`
}

type Result struct {
	Classification Classification    `json:"classification"`
	Owner          string            `json:"owner,omitempty"`
	ListenAddr     string            `json:"listen_addr,omitempty"`
	Version        Version           `json:"version,omitempty"`
	Health         Health            `json:"health,omitempty"`
	State          state.RouterState `json:"state,omitempty"`
}

type Config struct {
	BaseURL                   string
	DesktopStatePath          string
	CLIStatePath              string
	DeploymentID              string
	ManagementProtocolVersion string
	PortTimeout               time.Duration
	RequestTimeout            time.Duration
	DialContext               func(context.Context, string, string) (net.Conn, error)
	HTTPClient                *http.Client
	ReadState                 func(string) (state.RouterState, error)
	ValidateProcess           func(process.Identity, string) (process.Status, error)
}

type Discoverer struct {
	config Config
}

func New(config Config) *Discoverer {
	if config.BaseURL == "" {
		config.BaseURL = "http://127.0.0.1:19099"
	}
	if config.DeploymentID == "" {
		config.DeploymentID = version.DeploymentID
	}
	if config.ManagementProtocolVersion == "" {
		config.ManagementProtocolVersion = version.ManagementProtocolVersion
	}
	if config.PortTimeout <= 0 {
		config.PortTimeout = 250 * time.Millisecond
	}
	if config.RequestTimeout <= 0 {
		config.RequestTimeout = time.Second
	}
	if config.DialContext == nil {
		config.DialContext = (&net.Dialer{Timeout: config.PortTimeout}).DialContext
	}
	if config.HTTPClient == nil {
		config.HTTPClient = &http.Client{Timeout: config.RequestTimeout}
	}
	if config.ReadState == nil {
		config.ReadState = state.Read
	}
	if config.ValidateProcess == nil {
		config.ValidateProcess = process.Validate
	}
	return &Discoverer{config: config}
}

func (d *Discoverer) Discover(ctx context.Context) Result {
	result := Result{Classification: UnknownOccupant, ListenAddr: d.config.BaseURL}
	desktop, desktopErr := d.read(d.config.DesktopStatePath)
	cli, cliErr := d.read(d.config.CLIStatePath)

	u, err := url.Parse(d.config.BaseURL)
	if err != nil || u.Host == "" {
		return result
	}
	portCtx, cancel := context.WithTimeout(ctx, d.config.PortTimeout)
	conn, dialErr := d.config.DialContext(portCtx, "tcp", u.Host)
	cancel()
	if dialErr != nil {
		desktopStatus := d.validate(desktop, desktopErr)
		desktopManagerStatus := d.validateManager(desktop, desktopErr)
		cliStatus := d.validate(cli, cliErr)
		if staleState(desktopStatus) || (desktopStatus == process.StatusGenuine && desktopManagerStatus != process.StatusGenuine) || staleState(cliStatus) {
			result.Classification = Stale
		} else if desktopStatus == process.StatusGenuine && desktopManagerStatus == process.StatusGenuine && completeDesktop(desktop) && stateMetadataMatches(desktop, d.config) {
			result.Classification = Degraded
			result.Owner = "desktop"
			result.State = desktop
		} else if cliStatus == process.StatusGenuine && completeCLI(cli) && stateMetadataMatches(cli, d.config) && !isDevelopmentID(d.config.DeploymentID) {
			result.Classification = Degraded
			result.Owner = "cli"
			result.State = cli
		} else {
			result.Classification = Absent
		}
		return result
	}
	_ = conn.Close()

	versionErr := d.getJSON(ctx, "/version", &result.Version)
	healthErr := d.getJSON(ctx, "/health", &result.Health)
	if versionErr != nil {
		return result
	}
	desktopStatus := d.validate(desktop, desktopErr)
	desktopManagerStatus := d.validateManager(desktop, desktopErr)
	cliStatus := d.validate(cli, cliErr)
	matched, owner := d.matchState(result.Version, desktop, desktopStatus, desktopManagerStatus, cli, cliStatus)
	if matched == nil {
		if stateCorrelates(result.Version, desktop, desktopStatus, d.config) || (desktopStatus == process.StatusGenuine && desktopManagerStatus != process.StatusGenuine && endpointCorrelates(result.Version, desktop, d.config)) || stateCorrelates(result.Version, cli, cliStatus, d.config) {
			result.Classification = Stale
		}
		return result
	}
	result.State = *matched
	result.Owner = owner
	if healthErr != nil || result.Health.Status != "ok" {
		result.Classification = Degraded
		return result
	}
	if owner == "desktop" {
		result.Classification = DesktopOwned
	} else {
		result.Classification = ExternalCompatible
	}
	return result
}

func stateCorrelates(remote Version, value state.RouterState, status process.Status, config Config) bool {
	return status == process.StatusStale && value.PID > 0 && remote.PID == value.PID && remote.DeploymentID == config.DeploymentID && remote.ManagementProtocolVersion == config.ManagementProtocolVersion
}

func (d *Discoverer) read(path string) (state.RouterState, error) {
	if path == "" {
		return state.RouterState{}, os.ErrNotExist
	}
	return d.config.ReadState(path)
}

func (d *Discoverer) validate(value state.RouterState, err error) process.Status {
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return process.StatusAbsent
		}
		return process.StatusStale
	}
	status, err := d.config.ValidateProcess(identity(value), value.BinaryPath)
	if err != nil {
		return process.StatusStale
	}
	return status
}

func (d *Discoverer) validateManager(value state.RouterState, stateErr error) process.Status {
	if stateErr != nil || value.Owner != "desktop" {
		return process.StatusAbsent
	}
	identity := process.Identity{PID: value.ManagerPID, StartedAt: value.ManagerProcessStartedAt, Executable: value.ManagerProcessExecutable}
	status, err := d.config.ValidateProcess(identity, value.ManagerProcessExecutable)
	if err != nil {
		return process.StatusStale
	}
	return status
}

func staleState(status process.Status) bool { return status == process.StatusStale }

func (d *Discoverer) matchState(remote Version, desktop state.RouterState, desktopStatus, desktopManagerStatus process.Status, cli state.RouterState, cliStatus process.Status) (*state.RouterState, string) {
	metadataMatches := remote.PID > 0 && remote.DeploymentID == d.config.DeploymentID && remote.ManagementProtocolVersion == d.config.ManagementProtocolVersion
	if !metadataMatches {
		return nil, ""
	}
	if desktopStatus == process.StatusGenuine && desktopManagerStatus == process.StatusGenuine && completeDesktop(desktop) && remote.PID == desktop.PID && stateMetadataMatches(desktop, d.config) && stateAddressMatches(desktop, d.config.BaseURL) {
		return &desktop, "desktop"
	}
	// Development identities deliberately cannot authorize reuse of a router
	// owned by another manager or setup process.
	if isDevelopmentID(d.config.DeploymentID) {
		return nil, ""
	}
	if cliStatus == process.StatusGenuine && completeCLI(cli) && remote.PID == cli.PID && stateMetadataMatches(cli, d.config) && stateAddressMatches(cli, d.config.BaseURL) {
		return &cli, "cli"
	}
	return nil, ""
}

func endpointCorrelates(remote Version, value state.RouterState, config Config) bool {
	return value.PID > 0 && remote.PID == value.PID && remote.DeploymentID == config.DeploymentID && remote.ManagementProtocolVersion == config.ManagementProtocolVersion
}

func completeCLI(value state.RouterState) bool {
	return value.Owner == "cli" && value.PID > 0 && value.ListenAddr != "" && value.BinaryPath != "" && value.ProcessStartedAt != "" && value.ProcessExecutable != "" && value.RouterVersion != "" && value.DeploymentID != "" && value.ManagementProtocolVersion != ""
}

func completeDesktop(value state.RouterState) bool {
	return value.Owner == "desktop" && value.PID > 0 && value.ListenAddr != "" && value.BinaryPath != "" && value.LogPath != "" && value.ProcessStartedAt != "" && value.ProcessExecutable != "" && value.DesktopSessionID != "" && value.ManagerPID > 0 && value.ManagerProcessStartedAt != "" && value.ManagerProcessExecutable != "" && value.ManagerVersion != "" && value.RouterVersion != "" && value.DeploymentID != "" && value.ManagementProtocolVersion != ""
}

func stateMetadataMatches(value state.RouterState, config Config) bool {
	return value.DeploymentID == config.DeploymentID && value.ManagementProtocolVersion == config.ManagementProtocolVersion
}

func stateAddressMatches(value state.RouterState, baseURL string) bool {
	return strings.TrimRight(value.ListenAddr, "/") == strings.TrimRight(baseURL, "/")
}

func isDevelopmentID(value string) bool {
	value = strings.TrimSpace(strings.ToLower(value))
	return value == "" || value == "dev" || value == "unknown"
}

func identity(value state.RouterState) process.Identity {
	return process.Identity{PID: value.PID, StartedAt: value.ProcessStartedAt, Executable: value.ProcessExecutable}
}

func (d *Discoverer) getJSON(ctx context.Context, path string, target any) error {
	requestCtx, cancel := context.WithTimeout(ctx, d.config.RequestTimeout)
	defer cancel()
	req, err := http.NewRequestWithContext(requestCtx, http.MethodGet, strings.TrimRight(d.config.BaseURL, "/")+path, nil)
	if err != nil {
		return err
	}
	resp, err := d.config.HTTPClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("endpoint returned HTTP %d", resp.StatusCode)
	}
	decoder := json.NewDecoder(resp.Body)
	if err := decoder.Decode(target); err != nil {
		return err
	}
	return nil
}
