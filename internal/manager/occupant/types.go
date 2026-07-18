// Package occupant securely inspects and force-terminates an exact loopback
// TCP listener after single-use confirmation.
package occupant

import (
	"errors"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

var (
	ErrNotFound            = errors.New("occupant not found")
	ErrNotOwned            = errors.New("occupant not owned")
	ErrIdentityUnavailable = errors.New("occupant identity unavailable")
	ErrChanged             = errors.New("occupant changed")
	ErrProtected           = errors.New("occupant protected")
	ErrTerminationFailed   = errors.New("occupant termination failed")
	ErrPortReleaseTimeout  = errors.New("port release timeout")
	ErrConfirmationExpired = errors.New("confirmation expired")
)

// Identity is complete internal listener and process identity. It is never
// serialized by the manager protocol.
type Identity struct {
	ListenAddr string
	Network    string
	SocketID   string
	Process    process.Identity
	UserID     string
}

type Inspection struct {
	PID               int       `json:"pid"`
	ProcessName       string    `json:"process_name"`
	Executable        string    `json:"executable"`
	ListenAddr        string    `json:"listen_addr"`
	ConfirmationToken string    `json:"confirmation_token"`
	ExpiresAt         time.Time `json:"expires_at"`
}

type Result struct {
	State string `json:"state"`
}
