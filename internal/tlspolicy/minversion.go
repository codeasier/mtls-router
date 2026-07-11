package tlspolicy

import (
	"crypto/tls"
	"fmt"
)

func MinVersion(version string) (uint16, error) {
	switch version {
	case "", "tls1.2":
		return tls.VersionTLS12, nil
	case "tls1.3":
		return tls.VersionTLS13, nil
	default:
		return 0, fmt.Errorf("invalid TLS minimum version: %s", version)
	}
}
