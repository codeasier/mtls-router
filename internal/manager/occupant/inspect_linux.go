//go:build linux

package occupant

import (
	"bufio"
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

type procListener struct {
	inode string
	ip    net.IP
	port  int
}

func inspectNative(ctx context.Context, listenAddr string) (Target, error) {
	identity, err := inspectLinux(ctx, listenAddr, "/proc", process.Inspect)
	if err != nil {
		return Target{}, err
	}
	return Target{Mode: VerificationModeVerifiedIdentity, Identity: identity, PID: identity.Process.PID, ListenAddr: identity.ListenAddr}, nil
}

func inspectLinux(ctx context.Context, listenAddr, procRoot string, inspectProcess func(int) (process.Identity, error)) (Identity, error) {
	ip, port, err := validateAddress(listenAddr)
	if err != nil {
		return Identity{}, err
	}
	listeners, err := readProcListeners(procRoot, ip, port)
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	if len(listeners) == 0 {
		return Identity{}, ErrNotFound
	}
	if len(listeners) != 1 {
		return Identity{}, ErrIdentityUnavailable
	}
	entries, err := os.ReadDir(procRoot)
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	owners := map[int]bool{}
	want := "socket:[" + listeners[0].inode + "]"
	for _, entry := range entries {
		if ctx.Err() != nil {
			return Identity{}, ErrIdentityUnavailable
		}
		pid, err := strconv.Atoi(entry.Name())
		if err != nil || pid <= 0 || !entry.IsDir() {
			continue
		}
		fds, err := os.ReadDir(filepath.Join(procRoot, entry.Name(), "fd"))
		if err != nil {
			continue
		}
		for _, fd := range fds {
			target, err := os.Readlink(filepath.Join(procRoot, entry.Name(), "fd", fd.Name()))
			if err == nil && target == want {
				owners[pid] = true
			}
		}
	}
	if len(owners) != 1 {
		return Identity{}, ErrIdentityUnavailable
	}
	var pid int
	for value := range owners {
		pid = value
	}
	userID, err := procEffectiveUID(procRoot, pid)
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	identity, err := inspectProcess(pid)
	if err != nil {
		return Identity{}, ErrIdentityUnavailable
	}
	return Identity{ListenAddr: listenAddr, Network: "tcp4", SocketID: listeners[0].inode, Process: identity, UserID: userID}, nil
}

func readProcListeners(root string, ip net.IP, port int) ([]procListener, error) {
	var matches []procListener
	for _, name := range []string{"tcp", "tcp6"} {
		file, err := os.Open(filepath.Join(root, "net", name))
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return nil, err
		}
		scanner := bufio.NewScanner(file)
		first := true
		for scanner.Scan() {
			if first {
				first = false
				continue
			}
			fields := strings.Fields(scanner.Text())
			if len(fields) < 10 {
				file.Close()
				return nil, errors.New("malformed proc TCP row")
			}
			if fields[3] != "0A" {
				continue
			}
			parts := strings.Split(fields[1], ":")
			if len(parts) != 2 {
				file.Close()
				return nil, errors.New("malformed proc TCP address")
			}
			decodedPort, err := strconv.ParseUint(parts[1], 16, 16)
			if err != nil {
				file.Close()
				return nil, err
			}
			decodedIP, err := decodeProcIP(parts[0])
			if err != nil {
				file.Close()
				return nil, err
			}
			if int(decodedPort) == port && decodedIP.IsUnspecified() {
				file.Close()
				return nil, errors.New("wildcard listener is ambiguous")
			}
			if int(decodedPort) == port && decodedIP.Equal(ip) {
				matches = append(matches, procListener{inode: fields[9], ip: decodedIP, port: port})
			}
		}
		err = scanner.Err()
		file.Close()
		if err != nil {
			return nil, err
		}
	}
	return matches, nil
}

func decodeProcIP(value string) (net.IP, error) {
	data, err := hex.DecodeString(value)
	if err != nil || (len(data) != 4 && len(data) != 16) {
		return nil, errors.New("invalid proc TCP address")
	}
	for offset := 0; offset < len(data); offset += 4 {
		data[offset], data[offset+3] = data[offset+3], data[offset]
		data[offset+1], data[offset+2] = data[offset+2], data[offset+1]
	}
	return net.IP(data), nil
}

func procEffectiveUID(root string, pid int) (string, error) {
	data, err := os.ReadFile(filepath.Join(root, fmt.Sprintf("%d", pid), "status"))
	if err != nil {
		return "", err
	}
	for _, line := range strings.Split(string(data), "\n") {
		if strings.HasPrefix(line, "Uid:") {
			fields := strings.Fields(line)
			if len(fields) >= 3 {
				if _, err := strconv.ParseUint(fields[2], 10, 32); err == nil {
					return fields[2], nil
				}
			}
		}
	}
	return "", errors.New("effective UID unavailable")
}

func currentUserNative() (string, error) { return strconv.Itoa(os.Geteuid()), nil }
