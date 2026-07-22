//go:build darwin || linux || windows

package occupant

import (
	"context"
	"net"
	"os"
	"testing"
	"time"
)

func TestNativeInspectOwnLoopbackListener(t *testing.T) {
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	target, err := inspectNative(ctx, listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	if target.Mode != VerificationModeVerifiedIdentity {
		t.Fatalf("mode = %q", target.Mode)
	}
	identity := target.Identity
	if identity.Process.PID != os.Getpid() || identity.ListenAddr != listener.Addr().String() || identity.Network != "tcp4" || identity.SocketID == "" || identity.UserID == "" {
		t.Fatalf("identity = %+v", identity)
	}
}
