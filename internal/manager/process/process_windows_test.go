//go:build windows

package process

import "testing"

func TestWindowsSameStartIdentityRFC3339NanoPrecision(t *testing.T) {
	tests := []struct {
		name     string
		expected string
		live     string
		want     bool
	}{
		{
			name:     "exact RFC3339Nano with 7 decimal places (Windows 100ns precision)",
			expected: "2026-08-04T16:11:31.3498616Z",
			live:     "2026-08-04T16:11:31.3498616Z",
			want:     true,
		},
		{
			name:     "trailing zero truncation difference",
			expected: "2026-08-04T16:11:31.3498600Z",
			live:     "2026-08-04T16:11:31.34986Z",
			want:     true,
		},
		{
			name:     "second-level precision vs nano precision",
			expected: "2026-08-04T16:11:31Z",
			live:     "2026-08-04T16:11:31.0000000Z",
			want:     true,
		},
		{
			name:     "historical release fixture v0.1.8 timestamp",
			expected: "2026-03-05T12:34:56.1234567Z",
			live:     "2026-03-05T12:34:56.1234567Z",
			want:     true,
		},
		{
			name:     "historical release fixture v0.2.0 timestamp",
			expected: "2026-04-18T08:15:30.4567891Z",
			live:     "2026-04-18T08:15:30.4567891Z",
			want:     true,
		},
		{
			name:     "different timestamp",
			expected: "2026-08-04T16:11:31.3498616Z",
			live:     "2026-08-04T16:11:32.3498616Z",
			want:     false,
		},
		{
			name:     "invalid expected format",
			expected: "134211753300000000",
			live:     "2026-08-04T16:11:31.3498616Z",
			want:     false,
		},
		{
			name:     "invalid live format",
			expected: "2026-08-04T16:11:31.3498616Z",
			live:     "134211753300000000",
			want:     false,
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := sameStartIdentity(tc.expected, tc.live)
			if got != tc.want {
				t.Errorf("sameStartIdentity(%q, %q) = %t, want %t", tc.expected, tc.live, got, tc.want)
			}
		})
	}
}
