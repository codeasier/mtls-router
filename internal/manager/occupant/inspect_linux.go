//go:build linux

package occupant

import (
	"bufio"
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

const (
	maxProcCgroupSize = 64 * 1024
)

type systemdCgroupState uint8

const (
	systemdCgroupUncertain systemdCgroupState = iota
	systemdCgroupConclusive
	systemdCgroupSupervised
)

type systemdCgroupResult struct {
	state      systemdCgroupState
	supervisor *Supervisor
}

type procListener struct {
	inode string
	ip    net.IP
	port  int
}

func inspectNative(ctx context.Context, listenAddr string) (Target, error) {
	return inspectLinux(ctx, listenAddr, "/proc", process.Inspect)
}

func inspectLinux(ctx context.Context, listenAddr, procRoot string, inspectProcess func(int) (process.Identity, error)) (Target, error) {
	ip, port, err := validateAddress(listenAddr)
	if err != nil {
		return Target{}, err
	}
	listeners, err := readProcListeners(procRoot, ip, port)
	if err != nil {
		return Target{}, ErrIdentityUnavailable
	}
	if len(listeners) == 0 {
		return Target{}, ErrNotFound
	}
	if len(listeners) != 1 {
		return Target{}, ErrIdentityUnavailable
	}
	entries, err := os.ReadDir(procRoot)
	if err != nil {
		return Target{}, ErrIdentityUnavailable
	}
	owners := map[int]bool{}
	want := "socket:[" + listeners[0].inode + "]"
	for _, entry := range entries {
		if ctx.Err() != nil {
			return Target{}, ErrIdentityUnavailable
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
		return Target{}, ErrIdentityUnavailable
	}
	var pid int
	for value := range owners {
		pid = value
	}
	userID, err := procEffectiveUID(procRoot, pid)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return Target{}, ErrNotFound
		}
		return Target{}, ErrIdentityUnavailable
	}
	identity, err := inspectProcess(pid)
	if err != nil {
		if errors.Is(err, process.ErrNotFound) || errors.Is(err, os.ErrNotExist) {
			return Target{}, ErrNotFound
		}
		return Target{}, ErrIdentityUnavailable
	}
	targetIdentity := Identity{ListenAddr: listenAddr, Network: "tcp4", SocketID: listeners[0].inode, Process: identity, UserID: userID}
	target := Target{
		Mode:       VerificationModeVerifiedIdentity,
		Identity:   targetIdentity,
		PID:        pid,
		ListenAddr: listenAddr,
	}
	cgroup, err := readSystemdCgroup(filepath.Join(procRoot, strconv.Itoa(pid), "cgroup"))
	if err != nil {
		return Target{}, err
	}
	switch cgroup.state {
	case systemdCgroupSupervised:
		target.Supervisor = cgroup.supervisor
	case systemdCgroupConclusive:
	case systemdCgroupUncertain:
		target.BlockReason = RecoveryReasonIdentityUnavailable
	default:
		target.BlockReason = RecoveryReasonIdentityUnavailable
	}
	return target, nil
}

func readSystemdCgroup(path string) (systemdCgroupResult, error) {
	file, err := os.Open(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			if _, statErr := os.Stat(filepath.Dir(path)); errors.Is(statErr, os.ErrNotExist) {
				return systemdCgroupResult{}, ErrNotFound
			}
		}
		return systemdCgroupResult{state: systemdCgroupUncertain}, nil
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, maxProcCgroupSize+1))
	if err != nil || len(data) > maxProcCgroupSize {
		return systemdCgroupResult{state: systemdCgroupUncertain}, nil
	}
	return parseSystemdCgroup(data), nil
}

func parseSystemdCgroup(data []byte) systemdCgroupResult {
	if len(data) == 0 || len(data) > maxProcCgroupSize || !utf8.Valid(data) {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	text := string(data)
	if strings.HasSuffix(text, "\n") {
		text = strings.TrimSuffix(text, "\n")
	}
	if text == "" {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	candidates := make(map[string]*Supervisor)
	seenHierarchies := make(map[uint64]struct{})
	seenControllers := make(map[string]struct{})
	unclassified := false
	for _, line := range strings.Split(text, "\n") {
		parts := strings.SplitN(line, ":", 3)
		if len(parts) != 3 || parts[0] == "" || hasControl(parts[1]) || hasControl(parts[2]) {
			return systemdCgroupResult{state: systemdCgroupUncertain}
		}
		hierarchy, err := strconv.ParseUint(parts[0], 10, 32)
		if err != nil {
			return systemdCgroupResult{state: systemdCgroupUncertain}
		}
		controllers, valid := parseCgroupControllers(parts[1])
		if !valid || !validCgroupPath(parts[2]) {
			return systemdCgroupResult{state: systemdCgroupUncertain}
		}
		if (hierarchy == 0) != (len(controllers) == 0) {
			return systemdCgroupResult{state: systemdCgroupUncertain}
		}
		if _, exists := seenHierarchies[hierarchy]; exists {
			return systemdCgroupResult{state: systemdCgroupUncertain}
		}
		seenHierarchies[hierarchy] = struct{}{}
		for controller := range controllers {
			if _, exists := seenControllers[controller]; exists {
				return systemdCgroupResult{state: systemdCgroupUncertain}
			}
			seenControllers[controller] = struct{}{}
		}
		selected := hierarchy == 0 && len(controllers) == 0
		if hierarchy > 0 {
			_, selected = controllers["name=systemd"]
		}
		if !selected {
			continue
		}
		pathResult := classifySystemdCgroupPath(parts[2])
		switch pathResult.state {
		case systemdCgroupSupervised:
			key := string(pathResult.supervisor.Kind) + "\x00" + pathResult.supervisor.Identifiers[0]
			candidates[key] = pathResult.supervisor
		case systemdCgroupConclusive:
			unclassified = true
		case systemdCgroupUncertain:
			return systemdCgroupResult{state: systemdCgroupUncertain}
		default:
			return systemdCgroupResult{state: systemdCgroupUncertain}
		}
	}
	if len(candidates) == 0 {
		return systemdCgroupResult{state: systemdCgroupConclusive}
	}
	if unclassified || len(candidates) != 1 {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	for _, supervisor := range candidates {
		if validSupervisor(supervisor) {
			return systemdCgroupResult{state: systemdCgroupSupervised, supervisor: supervisor}
		}
	}
	return systemdCgroupResult{state: systemdCgroupUncertain}
}

func validCgroupPath(path string) bool {
	if path == "/" {
		return true
	}
	if !strings.HasPrefix(path, "/") {
		return false
	}
	for _, component := range strings.Split(strings.TrimPrefix(path, "/"), "/") {
		if component == "" || component == "." || component == ".." || !utf8.ValidString(component) || hasControl(component) {
			return false
		}
	}
	return true
}

func parseCgroupControllers(value string) (map[string]struct{}, bool) {
	controllers := make(map[string]struct{})
	if value == "" {
		return controllers, true
	}
	for _, controller := range strings.Split(value, ",") {
		if controller == "" || !validCgroupController(controller) {
			return nil, false
		}
		if _, exists := controllers[controller]; exists {
			return nil, false
		}
		controllers[controller] = struct{}{}
	}
	return controllers, true
}

func validCgroupController(value string) bool {
	for _, character := range value {
		if !((character >= 'a' && character <= 'z') || (character >= '0' && character <= '9') || strings.ContainsRune("_.-=", character)) {
			return false
		}
	}
	return true
}

func classifySystemdCgroupPath(path string) systemdCgroupResult {
	if path == "/" {
		return systemdCgroupResult{state: systemdCgroupConclusive}
	}
	components := strings.Split(strings.TrimPrefix(path, "/"), "/")
	if components[0] == "user.slice" {
		return classifyUserServicePath(components)
	}
	return classifySystemServicePath(components)
}

func classifySystemServicePath(components []string) systemdCgroupResult {
	serviceIndexes, valid := systemdServiceIndexes(components, 0)
	if !valid || !validSystemdSliceComponents(components) {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	if len(serviceIndexes) == 0 {
		return systemdCgroupResult{state: systemdCgroupConclusive}
	}
	if len(serviceIndexes) != 1 {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	serviceIndex := serviceIndexes[0]
	if serviceIndex == 0 || !onlySliceAncestors(components[:serviceIndex]) {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	return supervisedSystemdCgroup(SupervisorSystemdSystem, SupervisorScopeSystem, components[serviceIndex])
}

func classifyUserServicePath(components []string) systemdCgroupResult {
	serviceIndexes, valid := systemdServiceIndexes(components, 2)
	if !valid || !validSystemdSliceComponents(components) {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	uid, valid := userSliceUID(components)
	if !valid {
		if len(serviceIndexes) == 0 {
			return systemdCgroupResult{state: systemdCgroupConclusive}
		}
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	infrastructure := "user@" + uid + ".service"
	hasInfrastructure := len(components) > 2 && components[2] == infrastructure
	actualServices := make([]int, 0, len(serviceIndexes))
	for _, index := range serviceIndexes {
		if index == 2 && components[index] == infrastructure {
			continue
		}
		actualServices = append(actualServices, index)
	}
	if len(actualServices) == 0 {
		if len(serviceIndexes) == 0 || hasInfrastructure {
			return systemdCgroupResult{state: systemdCgroupConclusive}
		}
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	if !hasInfrastructure || len(actualServices) != 1 {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	serviceIndex := actualServices[0]
	if !onlySliceAncestors(components[3:serviceIndex]) {
		return systemdCgroupResult{state: systemdCgroupUncertain}
	}
	return supervisedSystemdCgroup(SupervisorSystemdUser, SupervisorScopeUser, components[serviceIndex])
}

func systemdServiceIndexes(components []string, start int) ([]int, bool) {
	var indexes []int
	for index := start; index < len(components); index++ {
		if !strings.HasSuffix(components[index], ".service") {
			continue
		}
		if !validLinuxSystemdServiceUnit(components[index]) {
			return nil, false
		}
		indexes = append(indexes, index)
	}
	return indexes, true
}

func supervisedSystemdCgroup(kind SupervisorKind, scope SupervisorScope, unit string) systemdCgroupResult {
	return systemdCgroupResult{
		state:      systemdCgroupSupervised,
		supervisor: &Supervisor{Kind: kind, Scope: scope, Identifiers: []string{unit}},
	}
}

func validLinuxSystemdServiceUnit(unit string) bool {
	return validSystemdUnit(unit, ".service")
}

func validSystemdSliceUnit(unit string) bool {
	return validSystemdUnit(unit, ".slice")
}

func onlySliceAncestors(components []string) bool {
	for _, component := range components {
		if !validSystemdSliceUnit(component) {
			return false
		}
	}
	return true
}

func validSystemdSliceComponents(components []string) bool {
	for _, component := range components {
		if strings.HasSuffix(component, ".slice") && !validSystemdSliceUnit(component) {
			return false
		}
	}
	return true
}

func userSliceUID(components []string) (string, bool) {
	if len(components) < 2 || !strings.HasPrefix(components[1], "user-") || !strings.HasSuffix(components[1], ".slice") {
		return "", false
	}
	uid := strings.TrimSuffix(strings.TrimPrefix(components[1], "user-"), ".slice")
	if uid == "" {
		return "", false
	}
	if _, err := strconv.ParseUint(uid, 10, 32); err != nil {
		return "", false
	}
	return uid, true
}

func hasControl(value string) bool {
	for _, character := range value {
		if unicode.IsControl(character) {
			return true
		}
	}
	return false
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
