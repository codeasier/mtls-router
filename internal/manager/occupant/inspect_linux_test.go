//go:build linux

package occupant

import (
	"context"
	"errors"
	"net"
	"os"
	"path/filepath"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/process"
)

const procTCPHeader = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n"

func TestReadProcListenersExactLoopbackTCPAndTCP6(t *testing.T) {
	tests := []struct {
		name    string
		file    string
		address string
		row     string
		inode   string
	}{
		{
			name:    "tcp4",
			file:    "tcp",
			address: "127.0.0.1",
			row:     procTCPRow("0100007F:4A9B", "101") + procTCPRow("0200007F:4A9B", "102"),
			inode:   "101",
		},
		{
			name:    "tcp6",
			file:    "tcp6",
			address: "::1",
			row:     procTCPRow("00000000000000000000000001000000:4A9B", "201") + procTCPRow("00000000000000000000000002000000:4A9B", "202"),
			inode:   "201",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			root := writeProcNet(t, map[string]string{test.file: procTCPHeader + test.row})
			listeners, err := readProcListeners(root, net.ParseIP(test.address), 19099)
			if err != nil {
				t.Fatal(err)
			}
			if len(listeners) != 1 || listeners[0].inode != test.inode || !listeners[0].ip.Equal(net.ParseIP(test.address)) || listeners[0].port != 19099 {
				t.Fatalf("listeners = %+v", listeners)
			}
		})
	}
}

func TestReadProcListenersRejectsMalformedInput(t *testing.T) {
	tests := []struct {
		name string
		file string
		row  string
	}{
		{name: "tcp short row", file: "tcp", row: "0: malformed\n"},
		{name: "tcp bad address", file: "tcp", row: procTCPRow("not-an-address:4A9B", "301")},
		{name: "tcp bad port", file: "tcp", row: procTCPRow("0100007F:XXXX", "302")},
		{name: "tcp6 bad address", file: "tcp6", row: procTCPRow("not-an-address:4A9B", "303")},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			root := writeProcNet(t, map[string]string{test.file: procTCPHeader + test.row})
			if _, err := readProcListeners(root, net.ParseIP("127.0.0.1"), 19099); err == nil {
				t.Fatal("readProcListeners() error = nil")
			}
		})
	}
}

func TestReadProcListenersRejectsWildcardAmbiguity(t *testing.T) {
	tests := []struct {
		name string
		file string
		row  string
	}{
		{name: "tcp4", file: "tcp", row: procTCPRow("00000000:4A9B", "401")},
		{name: "tcp6", file: "tcp6", row: procTCPRow("00000000000000000000000000000000:4A9B", "402")},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			root := writeProcNet(t, map[string]string{test.file: procTCPHeader + test.row})
			_, err := readProcListeners(root, net.ParseIP("127.0.0.1"), 19099)
			if err == nil || err.Error() != "wildcard listener is ambiguous" {
				t.Fatalf("error = %v", err)
			}
		})
	}
}

func TestReadProcListenersPreservesDuplicateMatches(t *testing.T) {
	row := procTCPRow("0100007F:4A9B", "501") + procTCPRow("0100007F:4A9B", "502")
	root := writeProcNet(t, map[string]string{"tcp": procTCPHeader + row})
	listeners, err := readProcListeners(root, net.ParseIP("127.0.0.1"), 19099)
	if err != nil {
		t.Fatal(err)
	}
	if len(listeners) != 2 || listeners[0].inode != "501" || listeners[1].inode != "502" {
		t.Fatalf("listeners = %+v", listeners)
	}
}

func TestInspectLinuxCorrelatesSocketOwner(t *testing.T) {
	root := writeProcNet(t, map[string]string{"tcp": procTCPHeader + procTCPRow("0100007F:4A9B", "601")})
	pidDir := filepath.Join(root, "4242")
	if err := os.MkdirAll(filepath.Join(pidDir, "fd"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pidDir, "status"), []byte("Name:\ttest\nUid:\t1000\t1001\t1002\t1003\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("socket:[601]", filepath.Join(pidDir, "fd", "7")); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("socket:[601]", filepath.Join(pidDir, "fd", "8")); err != nil {
		t.Fatal(err)
	}

	wantProcess := process.Identity{PID: 4242, StartedAt: "123", Executable: "/tmp/listener"}
	identity, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, func(pid int) (process.Identity, error) {
		if pid != 4242 {
			t.Fatalf("pid = %d", pid)
		}
		return wantProcess, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if identity.ListenAddr != "127.0.0.1:19099" || identity.Network != "tcp4" || identity.SocketID != "601" || identity.Process != wantProcess || identity.UserID != "1001" {
		t.Fatalf("identity = %+v", identity)
	}
}

func TestInspectLinuxRejectsDuplicateListenersAndOwners(t *testing.T) {
	t.Run("listeners", func(t *testing.T) {
		row := procTCPRow("0100007F:4A9B", "701") + procTCPRow("0100007F:4A9B", "702")
		root := writeProcNet(t, map[string]string{"tcp": procTCPHeader + row})
		_, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, process.Inspect)
		if !errors.Is(err, ErrIdentityUnavailable) {
			t.Fatalf("error = %v", err)
		}
	})

	t.Run("owners", func(t *testing.T) {
		root := writeProcNet(t, map[string]string{"tcp": procTCPHeader + procTCPRow("0100007F:4A9B", "703")})
		for _, pid := range []string{"8001", "8002"} {
			fdDir := filepath.Join(root, pid, "fd")
			if err := os.MkdirAll(fdDir, 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.Symlink("socket:[703]", filepath.Join(fdDir, "1")); err != nil {
				t.Fatal(err)
			}
		}
		_, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, process.Inspect)
		if !errors.Is(err, ErrIdentityUnavailable) {
			t.Fatalf("error = %v", err)
		}
	})
}

func procTCPRow(localAddress, inode string) string {
	return "   0: " + localAddress + " 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 " + inode + " 1 0000000000000000 100 0 0 10 0\n"
}

func writeProcNet(t *testing.T, files map[string]string) string {
	t.Helper()
	root := t.TempDir()
	netDir := filepath.Join(root, "net")
	if err := os.Mkdir(netDir, 0o755); err != nil {
		t.Fatal(err)
	}
	for name, content := range files {
		if err := os.WriteFile(filepath.Join(netDir, name), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return root
}
