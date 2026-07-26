//go:build windows

package occupant

import (
	"context"
	"encoding/binary"
	"errors"
	"syscall"
	"unicode/utf16"
	"unsafe"

	"github.com/codeasier/mtls-router/internal/manager/process"
	"golang.org/x/sys/windows"
)

var getExtendedTCPTable = windows.NewLazySystemDLL("iphlpapi.dll").NewProc("GetExtendedTcpTable")

const (
	windowsTCPRowOwnerPIDSize = 24
	windowsSCMBufferLimit     = 1024 * 1024
)

type windowsSCMDependencies struct {
	open      func() (windows.Handle, error)
	enumerate func(windows.Handle, []byte) (uint32, uint32, error)
	close     func(windows.Handle) error
}

func inspectNative(ctx context.Context, listenAddr string) (Target, error) {
	return inspectWindowsTarget(ctx, listenAddr, windowsTargetDependencies{
		inspectPIDOwner:  inspectPIDOwnerNative,
		servicesForPID:   servicesForPIDNative,
		processSessionID: processSessionIDNative,
		processSID: func(pid int) (string, error) {
			windowsPID, err := windowsPID(pid)
			if err != nil {
				return "", err
			}
			return processSID(windowsPID)
		},
		currentSID:         currentUserNative,
		inspectProcess:     process.Inspect,
		preflightTerminate: preflightTerminatePIDNative,
	})
}

func processSessionIDNative(pid int) (uint32, error) {
	windowsPID, err := windowsPID(pid)
	if err != nil {
		return 0, err
	}
	var sessionID uint32
	if err := windows.ProcessIdToSessionId(windowsPID, &sessionID); err != nil {
		return 0, mapWindowsProcessError(err)
	}
	return sessionID, nil
}

func servicesForPIDNative(pid int) ([]string, error) {
	return servicesForPIDWithDependencies(pid, windowsSCMDependencies{
		open: func() (windows.Handle, error) {
			return windows.OpenSCManager(nil, nil, windows.SC_MANAGER_ENUMERATE_SERVICE)
		},
		enumerate: func(handle windows.Handle, buffer []byte) (uint32, uint32, error) {
			var data *byte
			if len(buffer) > 0 {
				data = &buffer[0]
			}
			var needed, returned uint32
			err := windows.EnumServicesStatusEx(handle, windows.SC_ENUM_PROCESS_INFO, windows.SERVICE_WIN32, windows.SERVICE_STATE_ALL, data, uint32(len(buffer)), &needed, &returned, nil, nil)
			return needed, returned, err
		},
		close: windows.CloseServiceHandle,
	})
}

func servicesForPIDWithDependencies(pid int, deps windowsSCMDependencies) (names []string, err error) {
	if _, err := windowsPID(pid); err != nil {
		return nil, err
	}
	handle, err := deps.open()
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeErr := deps.close(handle); err == nil && closeErr != nil {
			err = closeErr
		}
	}()

	var buffer []byte
	for attempts := 0; attempts < 3; attempts++ {
		needed, returned, enumerateErr := deps.enumerate(handle, buffer)
		if enumerateErr == nil {
			return decodeWindowsServicesForPID(buffer, returned, pid)
		}
		if !errors.Is(enumerateErr, syscall.ERROR_MORE_DATA) || needed <= uint32(len(buffer)) || needed > windowsSCMBufferLimit {
			if errors.Is(enumerateErr, syscall.ERROR_MORE_DATA) {
				return nil, ErrIdentityUnavailable
			}
			return nil, enumerateErr
		}
		buffer = make([]byte, needed)
	}
	return nil, ErrIdentityUnavailable
}

func decodeWindowsServicesForPID(buffer []byte, count uint32, pid int) ([]string, error) {
	wantedPID, err := windowsPID(pid)
	if err != nil {
		return nil, err
	}
	if count == 0 {
		return nil, nil
	}
	rowSize := uint64(unsafe.Sizeof(windows.ENUM_SERVICE_STATUS_PROCESS{}))
	if len(buffer) == 0 || uint64(count) > uint64(len(buffer))/rowSize {
		return nil, ErrIdentityUnavailable
	}
	rows := unsafe.Slice((*windows.ENUM_SERVICE_STATUS_PROCESS)(unsafe.Pointer(&buffer[0])), int(count))
	names := make([]string, 0)
	for _, row := range rows {
		if row.ServiceStatusProcess.ProcessId == 0 || row.ServiceStatusProcess.ProcessId != wantedPID {
			continue
		}
		name, ok := windowsUTF16StringInBuffer(buffer, row.ServiceName)
		if !ok || name == "" {
			return nil, ErrIdentityUnavailable
		}
		names = append(names, name)
	}
	return names, nil
}

func windowsUTF16StringInBuffer(buffer []byte, value *uint16) (string, bool) {
	if len(buffer) < 2 || value == nil {
		return "", false
	}
	base := uintptr(unsafe.Pointer(&buffer[0]))
	address := uintptr(unsafe.Pointer(value))
	if address < base || address-base >= uintptr(len(buffer)) || (address-base)%2 != 0 {
		return "", false
	}
	offset := int(address - base)
	encoded := make([]uint16, 0)
	for offset+2 <= len(buffer) {
		value := binary.LittleEndian.Uint16(buffer[offset : offset+2])
		if value == 0 {
			return string(utf16.Decode(encoded)), true
		}
		encoded = append(encoded, value)
		offset += 2
	}
	return "", false
}

func inspectPIDOwnerNative(ctx context.Context, listenAddr string) (int, error) {
	if ctx.Err() != nil {
		return 0, ErrIdentityUnavailable
	}
	ip, port, err := validateAddress(listenAddr)
	if err != nil {
		return 0, err
	}
	var size uint32
	result, _, _ := getExtendedTCPTable.Call(0, uintptr(unsafe.Pointer(&size)), 1, windows.AF_INET, 3, 0)
	if ctx.Err() != nil {
		return 0, ErrIdentityUnavailable
	}
	if syscall.Errno(result) != windows.ERROR_INSUFFICIENT_BUFFER || size < 4 {
		return 0, ErrIdentityUnavailable
	}
	buffer := make([]byte, size)
	result, _, _ = getExtendedTCPTable.Call(uintptr(unsafe.Pointer(&buffer[0])), uintptr(unsafe.Pointer(&size)), 1, windows.AF_INET, 3, 0)
	if ctx.Err() != nil {
		return 0, ErrIdentityUnavailable
	}
	if result != 0 || size > uint32(len(buffer)) {
		return 0, ErrIdentityUnavailable
	}
	pid, err := selectTCP4ListenerOwner(buffer[:size], ip, port)
	if err != nil {
		return 0, err
	}
	if ctx.Err() != nil {
		return 0, ErrIdentityUnavailable
	}
	return int(pid), nil
}

func selectTCP4ListenerOwner(buffer []byte, ip []byte, port int) (uint32, error) {
	if len(buffer) < 4 || len(ip) != 4 || port <= 0 || port > 65535 {
		return 0, ErrIdentityUnavailable
	}
	count := uint64(binary.LittleEndian.Uint32(buffer[:4]))
	const rowOffset = uint64(4)
	if count > (uint64(len(buffer))-rowOffset)/windowsTCPRowOwnerPIDSize {
		return 0, ErrIdentityUnavailable
	}
	rowEnd := rowOffset + count*windowsTCPRowOwnerPIDSize
	wantedAddress := binary.LittleEndian.Uint32(ip)
	var owner uint32
	for offset := int(rowOffset); offset < int(rowEnd); offset += windowsTCPRowOwnerPIDSize {
		row := buffer[offset : offset+windowsTCPRowOwnerPIDSize]
		if binary.LittleEndian.Uint32(row[0:4]) != 2 {
			continue
		}
		localPort := int(binary.BigEndian.Uint16(row[8:10]))
		if row[10] != 0 || row[11] != 0 || binary.LittleEndian.Uint32(row[12:16]) != 0 || binary.LittleEndian.Uint32(row[16:20]) != 0 || binary.LittleEndian.Uint32(row[20:24]) == 0 {
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

func supportsPIDOnlyNative() bool { return true }

func preflightTerminatePIDNative(pid int) error {
	windowsPID, err := windowsPID(pid)
	if err != nil {
		return err
	}
	handle, err := windows.OpenProcess(windows.PROCESS_TERMINATE, false, windowsPID)
	if err != nil {
		return mapWindowsProcessError(err)
	}
	return mapWindowsProcessError(windows.CloseHandle(handle))
}

func signalPIDNative(pid int) error {
	windowsPID, err := windowsPID(pid)
	if err != nil {
		return err
	}
	handle, err := windows.OpenProcess(windows.PROCESS_TERMINATE, false, windowsPID)
	if err != nil {
		return mapWindowsProcessError(err)
	}
	defer windows.CloseHandle(handle)
	return mapWindowsProcessError(windows.TerminateProcess(handle, 1))
}

func mapWindowsProcessError(err error) error {
	if errors.Is(err, windows.ERROR_ACCESS_DENIED) {
		return ErrPermissionDenied
	}
	if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
		return process.ErrNotFound
	}
	return err
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
