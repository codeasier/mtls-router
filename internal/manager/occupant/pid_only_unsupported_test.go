//go:build !windows

package occupant

import (
	"context"
	"errors"
	"testing"
)

func TestPIDOnlyNativeUnsupportedFailsClosed(t *testing.T) {
	if supportsPIDOnlyNative() {
		t.Fatal("PID-only native support enabled outside Windows")
	}
	if _, err := inspectPIDOwnerNative(context.Background(), "127.0.0.1:19099"); !errors.Is(err, ErrIdentityUnavailable) {
		t.Fatalf("inspect error = %v, want %v", err, ErrIdentityUnavailable)
	}
	if err := signalPIDNative(42); !errors.Is(err, ErrIdentityUnavailable) {
		t.Fatalf("signal error = %v, want %v", err, ErrIdentityUnavailable)
	}
}
