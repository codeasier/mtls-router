package version

import (
	"encoding/json"
	"testing"
)

func TestInfoExposesBuildMetadata(t *testing.T) {
	info := Info()
	if info.Version == "" {
		t.Fatal("Info().Version should not be empty (defaults to \"dev\")")
	}
	if info.Commit == "" {
		t.Fatal("Info().Commit should not be empty (defaults to \"unknown\")")
	}
	if info.BuildDate == "" {
		t.Fatal("Info().BuildDate should not be empty (defaults to \"unknown\")")
	}
	if info.DeploymentID == "" {
		t.Fatal("Info().DeploymentID should not be empty")
	}
	if info.ManagementProtocolVersion == "" {
		t.Fatal("Info().ManagementProtocolVersion must be a non-empty code-owned constant")
	}
}

func TestInfoUsesInjectedValues(t *testing.T) {
	oldVersion, oldCommit, oldBuildDate := Version, Commit, BuildDate
	t.Cleanup(func() {
		Version, Commit, BuildDate = oldVersion, oldCommit, oldBuildDate
	})

	Version = "vTEST"
	Commit = "abc123test"
	BuildDate = "2026-06-21T00:00:00Z"

	info := Info()
	if info.Version != Version {
		t.Fatalf("Info().Version = %q, want %q", info.Version, Version)
	}
	if info.Commit != Commit {
		t.Fatalf("Info().Commit = %q, want %q", info.Commit, Commit)
	}
	if info.BuildDate != BuildDate {
		t.Fatalf("Info().BuildDate = %q, want %q", info.BuildDate, BuildDate)
	}
}

func TestInfoJSONRoundTrips(t *testing.T) {
	b, err := InfoJSON()
	if err != nil {
		t.Fatalf("InfoJSON() error: %v", err)
	}
	var decoded map[string]string
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("InfoJSON() not valid JSON: %v", err)
	}
	for _, key := range []string{"version", "commit", "build_date", "deployment_id", "management_protocol_version"} {
		if _, ok := decoded[key]; !ok {
			t.Fatalf("InfoJSON() missing key %q (got %s)", key, b)
		}
	}
}
