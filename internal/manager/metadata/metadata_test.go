package metadata

import (
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/version"
)

func TestInfoReportsNonemptyCodeOwnedProtocol(t *testing.T) {
	info := Info()
	if info.ManagementProtocolVersion == "" || info.ManagementProtocolVersion != version.ManagementProtocolVersion {
		t.Fatalf("manager info protocol = %q", info.ManagementProtocolVersion)
	}
	if info.DeploymentID == "" || !strings.Contains(info.Target, "/") {
		t.Fatalf("manager info = %+v", info)
	}
}

func TestValidateProduction(t *testing.T) {
	valid := Identity{DeploymentID: "service-prod-a", ManagementProtocolVersion: version.ManagementProtocolVersion}
	if err := ValidateProduction(valid, valid, valid); err != nil {
		t.Fatal(err)
	}
	for _, artifacts := range [][]Identity{
		nil,
		{{DeploymentID: "dev", ManagementProtocolVersion: version.ManagementProtocolVersion}},
		{{DeploymentID: "unknown", ManagementProtocolVersion: version.ManagementProtocolVersion}},
		{{DeploymentID: "service-prod-a"}},
		{{DeploymentID: "service-prod-a", ManagementProtocolVersion: "different"}},
		{valid, {DeploymentID: "service-prod-b", ManagementProtocolVersion: version.ManagementProtocolVersion}},
	} {
		if err := ValidateProduction(artifacts...); err == nil {
			t.Fatalf("ValidateProduction(%+v) unexpectedly succeeded", artifacts)
		}
	}
}
