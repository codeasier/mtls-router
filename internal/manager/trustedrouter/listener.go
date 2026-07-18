// Package trustedrouter establishes the channel-bound router trust required by
// secret-bearing manager operations.
package trustedrouter

import (
	"errors"
	"net"
	"net/netip"
	"strconv"
)

// Listener is the canonical numeric loopback endpoint and its derived URLs.
type Listener struct {
	Authority     string
	RouterBaseURL string
	APIBaseURL    string
}

// NormalizeListener accepts only an explicit numeric 127/8 or ::1 listener
// with a non-zero TCP port.
func NormalizeListener(value string) (Listener, error) {
	host, portText, err := net.SplitHostPort(value)
	if err != nil || host == "" || portText == "" {
		return Listener{}, errors.New("listen must be a numeric loopback host:port")
	}
	address, err := netip.ParseAddr(host)
	if err != nil || !((address.Is4() && address.As4()[0] == 127) || address == netip.IPv6Loopback()) {
		return Listener{}, errors.New("listen must use a numeric 127/8 or ::1 address")
	}
	port, err := strconv.ParseUint(portText, 10, 16)
	if err != nil || port == 0 {
		return Listener{}, errors.New("listen port must be between 1 and 65535")
	}
	authority := net.JoinHostPort(address.String(), strconv.FormatUint(port, 10))
	base := "http://" + authority
	return Listener{Authority: authority, RouterBaseURL: base, APIBaseURL: base + "/v1"}, nil
}
