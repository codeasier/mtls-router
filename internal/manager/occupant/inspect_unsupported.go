//go:build !darwin && !linux && !windows

package occupant

import "context"

func inspectNative(context.Context, string) (Identity, error) {
	return Identity{}, ErrIdentityUnavailable
}
func currentUserNative() (string, error) { return "", ErrIdentityUnavailable }
