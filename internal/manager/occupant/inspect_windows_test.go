//go:build windows

package occupant

import (
	"encoding/binary"
	"errors"
	"testing"
)

type windowsTCPRow struct {
	state       uint32
	localIP     [4]byte
	localPort   uint16
	portPadding uint16
	remoteIP    [4]byte
	remotePort  uint32
	pid         uint32
}

func TestSelectTCP4ListenerOwner(t *testing.T) {
	wantedIP := []byte{127, 0, 0, 1}
	buffer := windowsTCPTable(
		windowsTCPRow{state: 5, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, remoteIP: [4]byte{127, 0, 0, 2}, remotePort: 19100, pid: 10},
		windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 2}, localPort: 19099, pid: 11},
		windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, pid: 42},
		windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 1}, localPort: 19100, pid: 12},
	)

	pid, err := selectTCP4ListenerOwner(buffer, wantedIP, 19099)
	if err != nil {
		t.Fatal(err)
	}
	if pid != 42 {
		t.Fatalf("pid = %d, want 42", pid)
	}
}

func TestSelectTCP4ListenerOwnerNotFound(t *testing.T) {
	tests := []struct {
		name   string
		buffer []byte
	}{
		{name: "other listener", buffer: windowsTCPTable(windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 2}, localPort: 19099, pid: 11})},
		{name: "connected row", buffer: windowsTCPTable(windowsTCPRow{state: 5, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, remoteIP: [4]byte{127, 0, 0, 2}, remotePort: 19100, pid: 42})},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			_, err := selectTCP4ListenerOwner(test.buffer, []byte{127, 0, 0, 1}, 19099)
			if !errors.Is(err, ErrNotFound) {
				t.Fatalf("error = %v, want %v", err, ErrNotFound)
			}
		})
	}
}

func TestSelectTCP4ListenerOwnerIgnoresCapacitySlack(t *testing.T) {
	exact := windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, pid: 42}
	tests := []struct {
		name   string
		buffer []byte
	}{
		{name: "single trailing byte", buffer: append(windowsTCPTable(exact), 0xff)},
		{name: "arbitrary trailing bytes", buffer: append(windowsTCPTable(exact), 0xde, 0xad, 0xbe, 0xef)},
		{name: "unused row capacity", buffer: append(windowsTCPTable(exact), windowsTCPTable(exact)[4:]...)},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			pid, err := selectTCP4ListenerOwner(test.buffer, []byte{127, 0, 0, 1}, 19099)
			if err != nil {
				t.Fatal(err)
			}
			if pid != 42 {
				t.Fatalf("pid = %d, want 42", pid)
			}
		})
	}
}

func TestSelectTCP4ListenerOwnerCountDecreased(t *testing.T) {
	buffer := windowsTCPTable(
		windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, pid: 42},
		windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, pid: 43},
	)
	binary.LittleEndian.PutUint32(buffer[:4], 1)

	pid, err := selectTCP4ListenerOwner(buffer, []byte{127, 0, 0, 1}, 19099)
	if err != nil {
		t.Fatal(err)
	}
	if pid != 42 {
		t.Fatalf("pid = %d, want 42", pid)
	}
}

func TestSelectTCP4ListenerOwnerFailsClosed(t *testing.T) {
	exact := windowsTCPRow{state: 2, localIP: [4]byte{127, 0, 0, 1}, localPort: 19099, pid: 42}
	impossibleCount := windowsTCPTable(exact)
	binary.LittleEndian.PutUint32(impossibleCount[:4], ^uint32(0))
	tests := []struct {
		name   string
		buffer []byte
		ip     []byte
		port   int
	}{
		{name: "missing header", buffer: []byte{1, 0, 0}, ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "short row", buffer: windowsTCPTable(exact)[:27], ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "impossible count", buffer: impossibleCount, ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "invalid address", buffer: windowsTCPTable(exact), ip: []byte{127, 0, 0}, port: 19099},
		{name: "invalid port", buffer: windowsTCPTable(exact), ip: []byte{127, 0, 0, 1}, port: 0},
		{name: "duplicate same owner", buffer: windowsTCPTable(exact, exact), ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "duplicate owners", buffer: windowsTCPTable(exact, windowsTCPRow{state: 2, localIP: exact.localIP, localPort: 19099, pid: 43}), ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "wildcard", buffer: windowsTCPTable(windowsTCPRow{state: 2, localPort: 19099, pid: 42}), ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "unsupported port encoding", buffer: windowsTCPTable(windowsTCPRow{state: 2, localIP: exact.localIP, localPort: 19099, portPadding: 1, pid: 42}), ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "listener with remote endpoint", buffer: windowsTCPTable(windowsTCPRow{state: 2, localIP: exact.localIP, localPort: 19099, remoteIP: [4]byte{127, 0, 0, 1}, remotePort: 1, pid: 42}), ip: []byte{127, 0, 0, 1}, port: 19099},
		{name: "missing owner", buffer: windowsTCPTable(windowsTCPRow{state: 2, localIP: exact.localIP, localPort: 19099}), ip: []byte{127, 0, 0, 1}, port: 19099},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			_, err := selectTCP4ListenerOwner(test.buffer, test.ip, test.port)
			if !errors.Is(err, ErrIdentityUnavailable) {
				t.Fatalf("error = %v, want %v", err, ErrIdentityUnavailable)
			}
		})
	}
}

func TestWindowsSocketID(t *testing.T) {
	if got := windowsSocketID("127.0.0.1:19099", 42); got != "tcp4:127.0.0.1:19099:42" {
		t.Fatalf("socket ID = %q", got)
	}
	if windowsSocketID("127.0.0.1:19099", 42) == windowsSocketID("127.0.0.1:19099", 43) {
		t.Fatal("socket ID does not distinguish owners")
	}
	if windowsSocketID("127.0.0.1:19099", 42) == windowsSocketID("127.0.0.1:19100", 42) {
		t.Fatal("socket ID does not distinguish endpoints")
	}
}

func TestWindowsSupportsPIDOnlyNative(t *testing.T) {
	if !supportsPIDOnlyNative() {
		t.Fatal("PID-only native support disabled on Windows")
	}
}

func windowsTCPTable(rows ...windowsTCPRow) []byte {
	buffer := make([]byte, 4+len(rows)*windowsTCPRowOwnerPIDSize)
	binary.LittleEndian.PutUint32(buffer[:4], uint32(len(rows)))
	for index, value := range rows {
		row := buffer[4+index*windowsTCPRowOwnerPIDSize : 4+(index+1)*windowsTCPRowOwnerPIDSize]
		binary.LittleEndian.PutUint32(row[0:4], value.state)
		copy(row[4:8], value.localIP[:])
		binary.BigEndian.PutUint16(row[8:10], value.localPort)
		binary.LittleEndian.PutUint16(row[10:12], value.portPadding)
		copy(row[12:16], value.remoteIP[:])
		binary.LittleEndian.PutUint32(row[16:20], value.remotePort)
		binary.LittleEndian.PutUint32(row[20:24], value.pid)
	}
	return buffer
}
