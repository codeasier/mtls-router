//go:build !darwin && !linux && !windows

package occupant

import "context"

func inspectNative(context.Context, string) (Target, error) {
	return Target{}, ErrIdentityUnavailable
}
func currentUserNative() (string, error) { return "", ErrIdentityUnavailable }
