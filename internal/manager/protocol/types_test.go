package protocol

import "testing"

func TestIsLegacyLineageVersion(t *testing.T) {
	cases := []struct {
		version string
		want    bool
	}{
		{"1", true},
		{"3", true},
		{"", false},
		{"0", false},
		{"2", false},
		{"4", false},
		{"5", false},
		{"01", false},
		{" 1", false},
	}
	for _, tc := range cases {
		if got := IsLegacyLineageVersion(tc.version); got != tc.want {
			t.Errorf("IsLegacyLineageVersion(%q) = %v, want %v", tc.version, got, tc.want)
		}
	}
}
