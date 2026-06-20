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
	for _, key := range []string{"version", "commit", "build_date"} {
		if _, ok := decoded[key]; !ok {
			t.Fatalf("InfoJSON() missing key %q (got %s)", key, b)
		}
	}
}
