// Package routerenroll implements the independent personal-certificate control
// plane. It does not receive model requests or business API keys.
package routerenroll

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/sha256"
	"crypto/subtle"
	"crypto/x509"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"time"
	"unicode"
)

const (
	ProtocolVersion     = "1"
	EnrollOrigin        = "https://enroll.codeasier.top"
	RelayOrigin         = "https://relay.codeasier.top"
	EnrollmentTTL       = 10 * time.Minute
	PollInterval        = 5 * time.Second
	RecoveryWindow      = 24 * time.Hour
	MaxCSRBytes         = 8 * 1024
	ClientCertRequired  = "CLIENT_CERT_REQUIRED"
	ClientCertExpired   = "CLIENT_CERT_EXPIRED"
	ClientCertRejected  = "CLIENT_CERT_REJECTED"
	ServiceUnavailable  = "CERT_SERVICE_UNAVAILABLE"
	MembershipRequired  = "ORG_MEMBERSHIP_REQUIRED"
	InvalidRequest      = "ENROLL_INVALID_REQUEST"
	InvalidProof        = "ENROLL_INVALID_PROOF"
	EnrollmentExpired   = "ENROLL_EXPIRED"
	EnrollmentCancelled = "ENROLL_CANCELLED"
	EnrollmentDenied    = "ENROLL_DENIED"
	SlowDown            = "ENROLL_SLOW_DOWN"
	CapacityReached     = "ENROLL_CAPACITY_REACHED"
	OperationConflict   = "CERT_OPERATION_CONFLICT"
	ReauthRequired      = "CERT_REAUTH_REQUIRED"
)

type publicError struct {
	status  int
	message string
}

var publicErrors = map[string]publicError{
	ClientCertRequired:  {401, "client certificate required"},
	ClientCertExpired:   {401, "client certificate expired"},
	ClientCertRejected:  {403, "client certificate rejected"},
	ServiceUnavailable:  {503, "certificate service unavailable"},
	MembershipRequired:  {403, "organization membership required"},
	InvalidRequest:      {400, "invalid enrollment request"},
	InvalidProof:        {400, "invalid enrollment proof"},
	EnrollmentExpired:   {410, "enrollment expired"},
	EnrollmentCancelled: {410, "enrollment cancelled"},
	EnrollmentDenied:    {403, "enrollment denied"},
	SlowDown:            {429, "polling too quickly"},
	CapacityReached:     {429, "enrollment capacity reached"},
	OperationConflict:   {409, "certificate operation conflict"},
	ReauthRequired:      {401, "certificate login required"},
}

// WriteError never includes a caller/provider supplied diagnostic.
func WriteError(w http.ResponseWriter, code string) {
	value, ok := publicErrors[code]
	if !ok {
		code, value = ServiceUnavailable, publicErrors[ServiceUnavailable]
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(value.status)
	_ = json.NewEncoder(w).Encode(map[string]string{"code": code, "error": value.message})
}

type NativeStart struct {
	Version         string `json:"version"`
	CSRPEM          string `json:"csr_pem"`
	DeviceID        string `json:"device_id"`
	DeviceKind      string `json:"device_kind"`
	DeviceLabel     string `json:"device_label"`
	Challenge       string `json:"challenge"`
	ChallengeMethod string `json:"challenge_method"`
}

// Identity contains only server-verified stable identifiers. Display labels do
// not participate in certificate authorization.
type Identity struct {
	Version        string `json:"version"`
	OrganizationID string `json:"organization_id"`
	PolicyEpoch    int64  `json:"policy_epoch"`
	UserID         string `json:"user_id"`
	DeviceID       string `json:"device_id"`
}

type CSRBinding struct {
	Request    *x509.CertificateRequest
	CSRDigest  string
	SPKIDigest string
}

var devicePattern = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`)

// ParseCSR accepts exactly one signed P-256 PKCS#10 PEM block. Neither the
// request's subject nor any requested extensions are authorization inputs.
func ParseCSR(value string) (CSRBinding, string) {
	if len(value) == 0 || len(value) > MaxCSRBytes {
		return CSRBinding{}, InvalidRequest
	}
	value = strings.ReplaceAll(value, "\r\n", "\n")
	block, rest := pem.Decode([]byte(value))
	if block == nil || block.Type != "CERTIFICATE REQUEST" || len(block.Headers) != 0 || strings.TrimSpace(string(rest)) != "" || strings.TrimSpace(value) != strings.TrimSpace(string(pem.EncodeToMemory(block))) {
		return CSRBinding{}, InvalidRequest
	}
	csr, err := x509.ParseCertificateRequest(block.Bytes)
	if err != nil || csr.CheckSignature() != nil {
		return CSRBinding{}, InvalidRequest
	}
	pub, ok := csr.PublicKey.(*ecdsa.PublicKey)
	if !ok || pub.Curve != elliptic.P256() || csr.SignatureAlgorithm != x509.ECDSAWithSHA256 {
		return CSRBinding{}, InvalidRequest
	}
	return CSRBinding{csr, Digest(csr.Raw), Digest(csr.RawSubjectPublicKeyInfo)}, ""
}

func (s NativeStart) Validate() (CSRBinding, string) {
	if s.Version != ProtocolVersion || !devicePattern.MatchString(s.DeviceID) || (s.DeviceKind != "desktop" && s.DeviceKind != "linux_cli") || len(s.DeviceLabel) == 0 || len(s.DeviceLabel) > 128 || strings.IndexFunc(s.DeviceLabel, unicode.IsControl) >= 0 || s.ChallengeMethod != "S256" || !validProof(s.Challenge) {
		return CSRBinding{}, InvalidRequest
	}
	return ParseCSR(s.CSRPEM)
}

func Digest(value []byte) string { sum := sha256.Sum256(value); return hex.EncodeToString(sum[:]) }
func Challenge(verifier string) string {
	sum := sha256.Sum256([]byte(verifier))
	return base64.RawURLEncoding.EncodeToString(sum[:])
}
func VerifyProof(verifier, challenge string) bool {
	return validProof(verifier) && validProof(challenge) && subtle.ConstantTimeCompare([]byte(Challenge(verifier)), []byte(challenge)) == 1
}
func validProof(value string) bool {
	decoded, err := base64.RawURLEncoding.Strict().DecodeString(value)
	return len(value) == 43 && err == nil && len(decoded) == 32
}

// VerificationURL has no secret proof, CSR, account or caller-selected origin.
func VerificationURL(transaction string) (string, string) {
	if !validProof(transaction) {
		return "", InvalidRequest
	}
	return EnrollOrigin + "/enroll/native/verify?" + url.Values{"transaction": {transaction}}.Encode(), ""
}
