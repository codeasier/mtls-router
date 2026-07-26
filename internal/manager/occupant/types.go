// Package occupant securely inspects and force-terminates an exact loopback
// TCP listener after single-use confirmation.
package occupant

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

const (
	maxSupervisorIdentifiers    = 16
	maxSupervisorIdentifierSize = 256
	maxSupervisorSize           = 4 * 1024
	maxSystemdUnitSize          = 255
)

var (
	ErrNotFound            = errors.New("occupant not found")
	ErrNotOwned            = errors.New("occupant not owned")
	ErrIdentityUnavailable = errors.New("occupant identity unavailable")
	ErrChanged             = errors.New("occupant changed")
	ErrProtected           = errors.New("occupant protected")
	ErrPermissionDenied    = errors.New("occupant permission denied")
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

type VerificationMode string

const (
	VerificationModeVerifiedIdentity VerificationMode = "verified_identity"
	VerificationModeWindowsPIDOnly   VerificationMode = "windows_pid_only"
)

type RecoveryAction string

const (
	RecoveryActionForceTerminate     RecoveryAction = "force_terminate"
	RecoveryActionManualStopRequired RecoveryAction = "manual_stop_required"
	RecoveryActionUnavailable        RecoveryAction = "unavailable"
)

type RecoveryReason string

const (
	RecoveryReasonServiceManaged        RecoveryReason = "service_managed"
	RecoveryReasonInsufficientPrivilege RecoveryReason = "insufficient_privilege"
	RecoveryReasonDifferentUser         RecoveryReason = "different_user"
	RecoveryReasonProtectedProcess      RecoveryReason = "protected_process"
	RecoveryReasonIdentityUnavailable   RecoveryReason = "identity_unavailable"
)

type SupervisorKind string

const (
	SupervisorWindowsService SupervisorKind = "windows_service"
	SupervisorSystemdUser    SupervisorKind = "systemd_user"
	SupervisorSystemdSystem  SupervisorKind = "systemd_system"
)

type SupervisorScope string

const (
	SupervisorScopeUser   SupervisorScope = "user"
	SupervisorScopeSystem SupervisorScope = "system"
)

type Recovery struct {
	Action RecoveryAction `json:"action"`
	Reason RecoveryReason `json:"reason,omitempty"`
}

type Supervisor struct {
	Kind        SupervisorKind  `json:"kind"`
	Scope       SupervisorScope `json:"scope"`
	Identifiers []string        `json:"identifiers"`
}

func validSupervisor(supervisor *Supervisor) bool {
	if supervisor == nil || len(supervisor.Identifiers) == 0 || len(supervisor.Identifiers) > maxSupervisorIdentifiers {
		return false
	}
	switch supervisor.Kind {
	case SupervisorWindowsService:
		if supervisor.Scope != SupervisorScopeSystem {
			return false
		}
	case SupervisorSystemdUser:
		if supervisor.Scope != SupervisorScopeUser {
			return false
		}
	case SupervisorSystemdSystem:
		if supervisor.Scope != SupervisorScopeSystem {
			return false
		}
	default:
		return false
	}
	for index, identifier := range supervisor.Identifiers {
		if identifier == "" || !utf8.ValidString(identifier) || len(identifier) > maxSupervisorIdentifierSize || (index > 0 && supervisor.Identifiers[index-1] >= identifier) {
			return false
		}
		if supervisor.Kind == SupervisorWindowsService {
			if !validWindowsServiceName(identifier) {
				return false
			}
		} else if !validSystemdServiceUnit(identifier) {
			return false
		}
	}
	var encoded bytes.Buffer
	encoder := json.NewEncoder(&encoded)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(supervisor); err != nil {
		return false
	}
	return encoded.Len()-1 <= maxSupervisorSize
}

func validWindowsServiceName(identifier string) bool {
	if strings.TrimSpace(identifier) == "" {
		return false
	}
	for _, character := range identifier {
		if unicode.IsControl(character) || strings.ContainsRune(`/\,"`, character) {
			return false
		}
	}
	return true
}

func validSystemdServiceUnit(identifier string) bool {
	return validSystemdUnit(identifier, ".service")
}

func validSystemdUnit(unit, suffix string) bool {
	if len(unit) > maxSystemdUnitSize || !strings.HasSuffix(unit, suffix) {
		return false
	}
	stem := unit[:len(unit)-len(suffix)]
	if stem == "" {
		return false
	}

	atCount := 0
	for index := 0; index < len(stem); {
		character := stem[index]
		if isASCIIAlphanumeric(character) || strings.ContainsRune(":_.-", rune(character)) {
			index++
			continue
		}
		if character == '@' {
			if index == 0 || atCount == 1 {
				return false
			}
			atCount++
			index++
			continue
		}
		if character != '\\' || index+3 >= len(stem) || stem[index+1] != 'x' || !isASCIIHex(stem[index+2]) || !isASCIIHex(stem[index+3]) {
			return false
		}
		index += 4
	}
	return true
}

func isASCIIAlphanumeric(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9'
}

func isASCIIHex(character byte) bool {
	return character >= 'a' && character <= 'f' || character >= 'A' && character <= 'F' || character >= '0' && character <= '9'
}

// Target is the authorization subject bound to a confirmation token.
type Target struct {
	Mode        VerificationMode
	Identity    Identity
	PID         int
	ListenAddr  string
	Supervisor  *Supervisor
	BlockReason RecoveryReason
}

type Inspection struct {
	PID               int              `json:"pid"`
	VerificationMode  VerificationMode `json:"verification_mode"`
	ProcessName       string           `json:"process_name,omitempty"`
	Executable        string           `json:"executable,omitempty"`
	ListenAddr        string           `json:"listen_addr"`
	Recovery          Recovery         `json:"recovery"`
	Supervisor        *Supervisor      `json:"supervisor,omitempty"`
	ConfirmationToken string           `json:"confirmation_token,omitempty"`
	ExpiresAt         *time.Time       `json:"expires_at,omitempty"`
}

type Result struct {
	Termination string `json:"termination"`
	PortState   string `json:"port_state"`
}
