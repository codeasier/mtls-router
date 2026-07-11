package tlspolicy

import (
	"crypto/tls"
	"testing"
)

func TestMinVersion(t *testing.T) {
	for input, want := range map[string]uint16{
		"":       tls.VersionTLS12,
		"tls1.2": tls.VersionTLS12,
		"tls1.3": tls.VersionTLS13,
	} {
		got, err := MinVersion(input)
		if err != nil {
			t.Fatalf("MinVersion(%q): %v", input, err)
		}
		if got != want {
			t.Fatalf("MinVersion(%q) = %d, want %d", input, got, want)
		}
	}

	if _, err := MinVersion("tls1.1"); err == nil {
		t.Fatal("MinVersion(tls1.1) returned nil error")
	}
}
