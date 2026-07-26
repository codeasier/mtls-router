//go:build darwin

package occupant

import (
	"bytes"
	"context"
	"encoding/binary"
	"net"
	"syscall"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
)

func TestDarwinVerifiedTargetRecoveryDiagnostics(t *testing.T) {
	identity := testIdentity()
	serviceFor := func(target Target, config Config, currentUser string) *Service {
		config.ListenAddr = identity.ListenAddr
		return New(config, Dependencies{
			Discover: func(context.Context) discovery.Result {
				return discovery.Result{Classification: discovery.UnknownOccupant}
			},
			Inspect:     func(context.Context, string) (Target, error) { return target, nil },
			CurrentUser: func() (string, error) { return currentUser, nil },
			Random:      bytes.NewReader(make([]byte, 32)),
		})
	}

	t.Run("same user remains forceable", func(t *testing.T) {
		inspection, err := serviceFor(verifiedTarget(identity), Config{}, identity.UserID).Inspect(context.Background())
		if err != nil {
			t.Fatal(err)
		}
		if inspection.PID != identity.Process.PID || inspection.Recovery.Action != RecoveryActionForceTerminate || inspection.ConfirmationToken == "" || inspection.Supervisor != nil {
			t.Fatalf("inspection = %+v", inspection)
		}
	})

	tests := []struct {
		name        string
		config      Config
		currentUser string
		wantReason  RecoveryReason
	}{
		{name: "different user", currentUser: "different-user", wantReason: RecoveryReasonDifferentUser},
		{name: "protected desktop", config: Config{DesktopPID: identity.Process.PID}, currentUser: identity.UserID, wantReason: RecoveryReasonProtectedProcess},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			inspection, err := serviceFor(verifiedTarget(identity), test.config, test.currentUser).Inspect(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if inspection.PID != identity.Process.PID || inspection.Recovery.Reason != test.wantReason || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil || inspection.Supervisor != nil {
				t.Fatalf("inspection = %+v", inspection)
			}
		})
	}
}

func TestDecodeDarwinTCP4LoopbackListener(t *testing.T) {
	info := darwinTCP4TestRecord()

	record, ok := decodeDarwinTCP4Record(info)
	if !ok {
		t.Fatal("decodeDarwinTCP4Record rejected a TCP4 loopback listener")
	}
	if record.socketID != 0x1020304050607080 {
		t.Fatalf("socket ID = %#x, want %#x", record.socketID, uint64(0x1020304050607080))
	}
	if record.ip != [4]byte{127, 0, 0, 1} {
		t.Fatalf("IP = %v, want 127.0.0.1", record.ip)
	}
	if record.port != 20128 {
		t.Fatalf("port = %d, want 20128", record.port)
	}
	if record.state != 1 {
		t.Fatalf("state = %d, want 1", record.state)
	}
	if got := matchDarwinTCP4Listener(record, net.ParseIP("127.0.0.1"), 20128); got != darwinListenerExact {
		t.Fatalf("match = %d, want exact", got)
	}
}

func TestDarwinTCP4RecordRejection(t *testing.T) {
	t.Run("short", func(t *testing.T) {
		if _, ok := decodeDarwinTCP4Record(make([]byte, 347)); ok {
			t.Fatal("short record was accepted")
		}
	})

	t.Run("unknown", func(t *testing.T) {
		info := darwinTCP4TestRecord()
		binary.LittleEndian.PutUint32(info[180:184], uint32(syscall.IPPROTO_UDP))
		if _, ok := decodeDarwinTCP4Record(info); ok {
			t.Fatal("unknown record was accepted")
		}
	})

	t.Run("wildcard", func(t *testing.T) {
		info := darwinTCP4TestRecord()
		copy(info[324:328], net.IPv4zero.To4())
		record, ok := decodeDarwinTCP4Record(info)
		if !ok {
			t.Fatal("wildcard TCP4 record was not decoded")
		}
		if got := matchDarwinTCP4Listener(record, net.ParseIP("127.0.0.1"), 20128); got != darwinListenerWildcard {
			t.Fatalf("match = %d, want wildcard rejection", got)
		}
	})

	t.Run("non-listener", func(t *testing.T) {
		info := darwinTCP4TestRecord()
		binary.LittleEndian.PutUint32(info[344:348], 4)
		record, ok := decodeDarwinTCP4Record(info)
		if !ok {
			t.Fatal("non-listener TCP4 record was not decoded")
		}
		if got := matchDarwinTCP4Listener(record, net.ParseIP("127.0.0.1"), 20128); got != darwinListenerRejected {
			t.Fatalf("match = %d, want rejection", got)
		}
	})
}

func darwinTCP4TestRecord() []byte {
	info := make([]byte, 348)
	binary.LittleEndian.PutUint64(info[160:168], 0x1020304050607080)
	binary.LittleEndian.PutUint32(info[180:184], uint32(syscall.IPPROTO_TCP))
	binary.LittleEndian.PutUint32(info[184:188], uint32(syscall.AF_INET))
	binary.LittleEndian.PutUint32(info[256:260], 2)
	binary.BigEndian.PutUint16(info[268:270], 20128)
	copy(info[324:328], net.ParseIP("127.0.0.1").To4())
	binary.LittleEndian.PutUint32(info[344:348], 1)
	return info
}
