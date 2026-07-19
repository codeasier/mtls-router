// Package preset loads the immutable build-injected Agent model preset.
package preset

import (
	"encoding/base64"
	"errors"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

// Encoded is set only on manager binaries with -ldflags -X.
var Encoded string

var errInvalid = errors.New("invalid embedded Agent model preset")

// Load strictly decodes and structurally validates the embedded preset. The
// returned error is intentionally constant so preset contents cannot leak.
func Load() (*modelconfig.Config, error) {
	if Encoded == "" {
		return nil, nil
	}
	maxEncodedSize := base64.StdEncoding.EncodedLen(modelconfig.MaxConfigSize)
	if len(Encoded) > maxEncodedSize || strings.IndexFunc(Encoded, func(r rune) bool {
		return r == '\r' || r == '\n' || r == ' ' || r == '\t'
	}) >= 0 {
		return nil, errInvalid
	}
	decoded, err := base64.StdEncoding.Strict().DecodeString(Encoded)
	if err != nil || len(decoded) > modelconfig.MaxConfigSize {
		return nil, errInvalid
	}
	config, err := modelconfig.DecodeStructural(decoded)
	if err != nil {
		return nil, errInvalid
	}
	return config, nil
}
