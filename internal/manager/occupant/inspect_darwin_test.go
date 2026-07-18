//go:build darwin

package occupant

import (
	"encoding/binary"
	"net"
	"syscall"
	"testing"
)

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
