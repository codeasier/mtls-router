// Package metadata defines the manager/router/desktop build identity handshake.
package metadata

import (
	"errors"
	"fmt"
	"runtime"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
	"github.com/codeasier/mtls-router/internal/version"
)

type Identity struct {
	DeploymentID              string
	ManagementProtocolVersion string
}

// Info returns the code-owned manager handshake metadata.
func Info() protocol.ManagerInfoResult {
	return protocol.ManagerInfoResult{
		Version:                   version.Version,
		Commit:                    version.Commit,
		BuildDate:                 version.BuildDate,
		Target:                    runtime.GOOS + "/" + runtime.GOARCH,
		DeploymentID:              version.DeploymentID,
		ManagementProtocolVersion: version.ManagementProtocolVersion,
	}
}

// ValidateProduction rejects development/default identities and any artifact
// mismatch. Release workflows can call this before packaging router, manager,
// and desktop artifacts.
func ValidateProduction(artifacts ...Identity) error {
	if len(artifacts) == 0 {
		return errors.New("no artifact identities supplied")
	}
	want := artifacts[0]
	if defaultValue(want.DeploymentID) {
		return errors.New("production deployment ID is empty or default")
	}
	if strings.TrimSpace(want.ManagementProtocolVersion) == "" {
		return errors.New("management protocol version is empty")
	}
	if want.ManagementProtocolVersion != version.ManagementProtocolVersion {
		return errors.New("management protocol version is not code-owned version")
	}
	for i, artifact := range artifacts[1:] {
		if artifact != want {
			return fmt.Errorf("artifact %d identity mismatch", i+2)
		}
	}
	return nil
}

func defaultValue(value string) bool {
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "", "dev", "unknown":
		return true
	default:
		return false
	}
}
