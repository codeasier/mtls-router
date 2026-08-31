//go:build darwin

package process

import "testing"

func TestDarwinSameStartIdentityRFC3339NanoPrecision(t *testing.T) {
	tests := []struct {
		name     string
		expected string
		live     string
		want     bool
	}{
		{
			name:     "exact RFC3339Nano",
			expected: "2026-08-04T16:11:31.349861Z",
			live:     "2026-08-04T16:11:31.349861Z",
			want:     true,
		},
		{
			name:     "trailing zero truncation",
			expected: "2026-08-04T16:11:31.349860000Z",
			live:     "2026-08-04T16:11:31.34986Z",
			want:     true,
		},
		{
			name:     "second precision",
			expected: "2026-08-04T16:11:31Z",
			live:     "2026-08-04T16:11:31.000000Z",
			want:     true,
		},
		{
			name:     "different timestamp",
			expected: "2026-08-04T16:11:31.349861Z",
			live:     "2026-08-04T16:11:32.349861Z",
			want:     false,
		},
		{
			name:     "invalid live format",
			expected: "2026-08-04T16:11:31.349861Z",
			live:     "12345678",
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
