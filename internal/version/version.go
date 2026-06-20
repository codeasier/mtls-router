// Package version exposes the build-time metadata of the running mtls-router
// binary. The three variables are intended to be set via -ldflags at link
// time. See Dockerfile, scripts/build.sh, and .github/workflows/release.yml
// for the three injection sites.
package version

import "encoding/json"

var (
	// Version is the semantic version or git tag of the build (e.g. "v0.1.1").
	// Falls back to "dev" for local builds without a VERSION env var.
	Version = "dev"
	// Commit is the short git SHA the binary was built from.
	Commit = "unknown"
	// BuildDate is the UTC ISO-8601 timestamp the binary was built at.
	BuildDate = "unknown"
)

// BuildInfo is a snapshot of the build metadata.
type BuildInfo struct {
	Version   string `json:"version"`
	Commit    string `json:"commit"`
	BuildDate string `json:"build_date"`
}

// Info returns the current build metadata.
func Info() BuildInfo {
	return BuildInfo{
		Version:   Version,
		Commit:    Commit,
		BuildDate: BuildDate,
	}
}

// InfoJSON encodes the build metadata as JSON for the /version endpoint.
func InfoJSON() ([]byte, error) {
	return json.Marshal(Info())
}
