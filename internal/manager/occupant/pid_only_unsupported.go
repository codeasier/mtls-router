//go:build !windows

package occupant

import "context"

func supportsPIDOnlyNative() bool { return false }

func inspectPIDOwnerNative(context.Context, string) (int, error) {
	return 0, ErrIdentityUnavailable
}

func signalPIDNative(int) error { return ErrIdentityUnavailable }
