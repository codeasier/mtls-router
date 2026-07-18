//go:build windows

package occupant

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"syscall"
	"unsafe"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"golang.org/x/sys/windows"
)

var getExtendedTCPTable = windows.NewLazySystemDLL("iphlpapi.dll").NewProc("GetExtendedTcpTable")

const windowsTCPRowOwnerPIDSize = 24

func inspectNative(_ context.Context, listenAddr string) (Identity, error) {
	ip, port, err := validateAddress(listenAddr)
	if err != nil {
		return Identity{}, err
	}
	var size uint32
	result, _, _ := getExtendedTCPTable.Call(0, uintptr(unsafe.Pointer(&size)), 1, windows.AF_INET, 3, 0)
	if syscall.Errno(result) != windows.ERROR_INSUFFICIENT_BUFFER || size < 4 {
		return Identity{}, ErrIdentityUnavailable
	}
	buffer := make([]byte, size)
	result, _, _ = getExtendedTCPTable.Call(uintptr(unsafe.Pointer(&buffer[0])), uintptr(unsafe.Pointer(&size)), 1, windows.AF_INET, 3, 0)
	if result != 0 || size > uint32(len(buffer)) {
		return Identity{}, ErrIdentityUnavailable
	}
	pid, err := selectTCP4ListenerOwner(buffer[:size], ip, port)
	if err != nil {
		return Identity{}, err
	}
	userID, err := processSID(pid)
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	identity, err := process.Inspect(int(pid))
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	return Identity{ListenAddr: listenAddr, Network: "tcp4", SocketID: windowsSocketID(listenAddr, pid), Process: identity, UserID: userID}, nil
}

func selectTCP4ListenerOwner(buffer []byte, ip []byte, port int) (uint32, error) {
	if len(buffer) < 4 || len(ip) != 4 || port <= 0 || port > 65535 {
		return 0, ErrIdentityUnavailable
	}
	count := uint64(binary.LittleEndian.Uint32(buffer[:4]))
	wantedSize := uint64(4) + count*windowsTCPRowOwnerPIDSize
	if wantedSize != uint64(len(buffer)) {
		return 0, ErrIdentityUnavailable
	}
	wantedAddress := binary.LittleEndian.Uint32(ip)
	var owner uint32
	for offset := 4; offset < len(buffer); offset += windowsTCPRowOwnerPIDSize {
		row := buffer[offset : offset+windowsTCPRowOwnerPIDSize]
		localPort := int(binary.BigEndian.Uint16(row[8:10]))
		if row[10] != 0 || row[11] != 0 || binary.LittleEndian.Uint32(row[0:4]) != 2 || binary.LittleEndian.Uint32(row[12:16]) != 0 || binary.LittleEndian.Uint32(row[16:20]) != 0 || binary.LittleEndian.Uint32(row[20:24]) == 0 {
			return 0, ErrIdentityUnavailable
		}
		localAddress := binary.LittleEndian.Uint32(row[4:8])
		if localPort == port && localAddress == 0 {
			return 0, ErrIdentityUnavailable
		}
		if localPort != port || localAddress != wantedAddress {
			continue
		}
		if owner != 0 {
			return 0, ErrIdentityUnavailable
		}
		owner = binary.LittleEndian.Uint32(row[20:24])
	}
	if owner == 0 {
		return 0, ErrNotFound
	}
	return owner, nil
}

func windowsSocketID(listenAddr string, pid uint32) string {
	return fmt.Sprintf("tcp4:%s:%d", listenAddr, pid)
}

func processSID(pid uint32) (string, error) {
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		return "", err
	}
	defer windows.CloseHandle(handle)
	var token windows.Token
	if err := windows.OpenProcessToken(handle, windows.TOKEN_QUERY, &token); err != nil {
		return "", err
	}
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil || user == nil || user.User.Sid == nil {
		return "", errors.New("process SID unavailable")
	}
	return user.User.Sid.String(), nil
}

func currentUserNative() (string, error) {
	token := windows.GetCurrentProcessToken()
	user, err := token.GetTokenUser()
	if err != nil || user == nil || user.User.Sid == nil {
		return "", errors.New("current SID unavailable")
	}
	return user.User.Sid.String(), nil
}
