//go:build darwin

package occupant

import (
	"context"
	"encoding/binary"
	"fmt"
	"net"
	"os"
	"strconv"
	"syscall"
	"unsafe"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"golang.org/x/sys/unix"
)

const (
	procInfoCallListPIDs  = 1
	procInfoCallPIDInfo   = 2
	procInfoCallPIDFDInfo = 3
	procAllPIDs           = 1
	procPIDListFDs        = 1
	procPIDFDSocketInfo   = 3
	procFDTypeSocket      = 2
	darwinSocketInfoSize  = 792
)

// The fixed record size and offsets below follow socket_fdinfo, socket_info,
// tcp_sockinfo, and in_sockinfo in the target macOS SDK's sys/proc_info.h.
// TestNativeInspectOwnLoopbackListener validates the layout against the running
// Darwin kernel rather than relying only on synthetic decoder fixtures.

type darwinTCP4Record struct {
	socketID uint64
	ip       [4]byte
	port     int
	state    int32
}

type darwinListenerMatch uint8

const (
	darwinListenerRejected darwinListenerMatch = iota
	darwinListenerExact
	darwinListenerWildcard
)

func decodeDarwinTCP4Record(info []byte) (darwinTCP4Record, bool) {
	if len(info) < 348 ||
		int32(binary.LittleEndian.Uint32(info[180:184])) != syscall.IPPROTO_TCP ||
		int32(binary.LittleEndian.Uint32(info[184:188])) != syscall.AF_INET ||
		int32(binary.LittleEndian.Uint32(info[256:260])) != 2 {
		return darwinTCP4Record{}, false
	}
	return darwinTCP4Record{
		socketID: binary.LittleEndian.Uint64(info[160:168]),
		ip:       [4]byte(info[324:328]),
		port:     int(binary.BigEndian.Uint16(info[268:270])),
		state:    int32(binary.LittleEndian.Uint32(info[344:348])),
	}, true
}

func matchDarwinTCP4Listener(record darwinTCP4Record, ip net.IP, port int) darwinListenerMatch {
	if record.port == port && net.IP(record.ip[:]).IsUnspecified() {
		return darwinListenerWildcard
	}
	if record.port == port && net.IP(record.ip[:]).Equal(ip) && record.state == 1 {
		return darwinListenerExact
	}
	return darwinListenerRejected
}

func inspectNative(ctx context.Context, listenAddr string) (Target, error) {
	identity, err := inspectDarwin(ctx, listenAddr)
	if err != nil {
		return Target{}, err
	}
	return Target{Mode: VerificationModeVerifiedIdentity, Identity: identity, PID: identity.Process.PID, ListenAddr: identity.ListenAddr}, nil
}

func inspectDarwin(ctx context.Context, listenAddr string) (Identity, error) {
	ip, port, err := validateAddress(listenAddr)
	if err != nil {
		return Identity{}, err
	}
	pids, err := darwinPIDs()
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	var matches []Identity
	for _, pid := range pids {
		if ctx.Err() != nil {
			return Identity{}, ErrIdentityUnavailable
		}
		fds, err := darwinFDs(pid)
		if err != nil {
			continue
		}
		for _, fd := range fds {
			info, err := darwinSocket(pid, fd)
			if err != nil {
				continue
			}
			record, ok := decodeDarwinTCP4Record(info)
			if !ok {
				continue
			}
			switch matchDarwinTCP4Listener(record, ip, port) {
			case darwinListenerWildcard:
				return Identity{}, ErrIdentityUnavailable
			case darwinListenerExact:
			default:
				continue
			}
			processIdentity, err := process.Inspect(pid)
			if err != nil {
				return Identity{}, ErrIdentityUnavailable
			}
			userID, err := darwinProcessUID(pid)
			if err != nil {
				return Identity{}, ErrIdentityUnavailable
			}
			if record.socketID == 0 {
				return Identity{}, ErrIdentityUnavailable
			}
			matches = append(matches, Identity{ListenAddr: listenAddr, Network: "tcp4", SocketID: fmt.Sprintf("%x", record.socketID), Process: processIdentity, UserID: userID})
		}
	}
	if len(matches) == 0 {
		return Identity{}, ErrNotFound
	}
	if len(matches) != 1 {
		return Identity{}, ErrIdentityUnavailable
	}
	return matches[0], nil
}

func procInfo(call, pid, flavor uintptr, arg uint64, buffer []byte) (int, error) {
	var pointer uintptr
	if len(buffer) > 0 {
		pointer = uintptr(unsafe.Pointer(&buffer[0]))
	}
	result, _, errno := unix.Syscall6(unix.SYS_PROC_INFO, call, pid, flavor, uintptr(arg), pointer, uintptr(len(buffer)))
	if errno != 0 {
		return 0, errno
	}
	return int(result), nil
}

func darwinPIDs() ([]int, error) {
	buffer := make([]byte, 64*1024)
	length, err := procInfo(procInfoCallListPIDs, procAllPIDs, 0, 0, buffer)
	if err != nil || length%4 != 0 {
		return nil, ErrIdentityUnavailable
	}
	result := make([]int, 0, length/4)
	for offset := 0; offset < length; offset += 4 {
		if pid := int(int32(binary.LittleEndian.Uint32(buffer[offset : offset+4]))); pid > 0 {
			result = append(result, pid)
		}
	}
	return result, nil
}

func darwinFDs(pid int) ([]int, error) {
	buffer := make([]byte, 64*1024)
	length, err := procInfo(procInfoCallPIDInfo, uintptr(pid), procPIDListFDs, 0, buffer)
	if err != nil || length%8 != 0 {
		return nil, ErrIdentityUnavailable
	}
	var result []int
	for offset := 0; offset < length; offset += 8 {
		if binary.LittleEndian.Uint32(buffer[offset+4:offset+8]) == procFDTypeSocket {
			result = append(result, int(int32(binary.LittleEndian.Uint32(buffer[offset:offset+4]))))
		}
	}
	return result, nil
}

func darwinSocket(pid, fd int) ([]byte, error) {
	buffer := make([]byte, darwinSocketInfoSize)
	length, err := procInfo(procInfoCallPIDFDInfo, uintptr(pid), procPIDFDSocketInfo, uint64(fd), buffer)
	if err != nil || length < 372 {
		return nil, ErrIdentityUnavailable
	}
	return buffer[:length], nil
}

func darwinProcessUID(pid int) (string, error) {
	info, err := unix.SysctlKinfoProc("kern.proc.pid", pid)
	if err != nil || info == nil || info.Proc.P_pid == 0 {
		return "", ErrIdentityUnavailable
	}
	return strconv.FormatUint(uint64(info.Eproc.Ucred.Uid), 10), nil
}

func currentUserNative() (string, error) { return strconv.Itoa(os.Geteuid()), nil }
