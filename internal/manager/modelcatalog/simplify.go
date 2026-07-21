package modelcatalog

import (
	"errors"
	"strings"
)

// Simplify is the link-time catalog policy value and defaults to enabled.
var Simplify = "True"

// ParseSimplify validates and parses the link-time catalog policy.
func ParseSimplify() (bool, error) {
	for i := 0; i < len(Simplify); i++ {
		if Simplify[i] > 0x7f {
			return false, errors.New("invalid embedded simplify value")
		}
	}

	switch {
	case strings.EqualFold(Simplify, "true"):
		return true, nil
	case strings.EqualFold(Simplify, "false"):
		return false, nil
	default:
		return false, errors.New("invalid embedded simplify value")
	}
}
