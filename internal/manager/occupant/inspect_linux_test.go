//go:build linux

package occupant

import (
	"bytes"
	"context"
	"errors"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
)

const procTCPHeader = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n"

func TestParseSystemdCgroup(t *testing.T) {
	tests := []struct {
		name           string
		input          string
		wantState      systemdCgroupState
		wantSupervisor *Supervisor
	}{
		{
			name:           "cgroup v2 user service",
			input:          "0::/user.slice/user-1000.slice/user@1000.service/app.slice/demo.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdUser, Scope: SupervisorScopeUser, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "cgroup v2 system service",
			input:          "0::/system.slice/mtls-router.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"mtls-router.service"}},
		},
		{
			name:           "cgroup v1 system service",
			input:          "1:name=systemd:/system.slice/legacy.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"legacy.service"}},
		},
		{
			name:           "cgroup v1 user service",
			input:          "2:name=systemd:/user.slice/user-1000.slice/user@1000.service/demo.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdUser, Scope: SupervisorScopeUser, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "consistent hybrid evidence",
			input:          "0::/system.slice/demo.service\n1:name=systemd:/system.slice/demo.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "delegated system service",
			input:          "0::/system.slice/demo.service/child\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "custom system slice",
			input:          "0::/codeasier.slice/router.slice/demo.service/child\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "255 byte slice ancestor",
			input:          "0::/" + strings.Repeat("x", 249) + ".slice/demo.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "escaped custom slice",
			input:          "0::/codeasier\\x2dcustom.slice/demo.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "delegated user service ignores manager",
			input:          "0::/user.slice/user-1000.slice/user@1000.service/app.slice/demo.service/child\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdUser, Scope: SupervisorScopeUser, Identifiers: []string{"demo.service"}},
		},
		{
			name:           "delegated user instance",
			input:          "0::/user.slice/user-1000.slice/user@1000.service/app.slice/worker@blue.service/control\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdUser, Scope: SupervisorScopeUser, Identifiers: []string{"worker@blue.service"}},
		},
		{
			name:           "escaped canonical unit",
			input:          "0::/system.slice/demo\\x2dworker:blue_1@main.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{`demo\x2dworker:blue_1@main.service`}},
		},
		{
			name:           "canonical punctuation and case",
			input:          "0::/system.slice/Router:blue_1@main-2.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"Router:blue_1@main-2.service"}},
		},
		{
			name:           "template unit",
			input:          "0::/system.slice/worker@.service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"worker@.service"}},
		},
		{
			name:           "255 byte unit",
			input:          "0::/system.slice/" + strings.Repeat("x", 247) + ".service\n",
			wantState:      systemdCgroupSupervised,
			wantSupervisor: &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{strings.Repeat("x", 247) + ".service"}},
		},
		{name: "scope is conclusive", input: "0::/system.slice/docker-abc.scope\n", wantState: systemdCgroupConclusive},
		{name: "user manager scope is conclusive", input: "0::/user.slice/user-1000.slice/user@1000.service/app.slice/session.scope\n", wantState: systemdCgroupConclusive},
		{name: "user manager itself is conclusive", input: "0::/user.slice/user-1000.slice/user@1000.service\n", wantState: systemdCgroupConclusive},
		{name: "container without service is conclusive", input: "0::/kubepods.slice/kubepods-burstable.slice/pod.scope\n", wantState: systemdCgroupConclusive},
		{name: "unsupported v1 hierarchy is conclusive", input: "1:cpu:/system.slice/demo.service\n", wantState: systemdCgroupConclusive},
		{name: "root is conclusive", input: "0::/\n", wantState: systemdCgroupConclusive},
		{name: "service below scope conflicts", input: "0::/system.slice/docker-abc.scope/evil.service/child\n"},
		{name: "traversal", input: "0::/system.slice/../evil.service\n"},
		{name: "empty component", input: "0::/system.slice//evil.service\n"},
		{name: "multiple system services", input: "0::/system.slice/outer.service/inner.service/child\n"},
		{name: "multiple user services", input: "0::/user.slice/user-1000.slice/user@1000.service/outer.service/inner.service/child\n"},
		{name: "user service without manager", input: "0::/user.slice/user-1000.slice/demo.service/child\n"},
		{name: "mismatched user manager", input: "0::/user.slice/user-1000.slice/user@1001.service/demo.service/child\n"},
		{name: "malformed slice space", input: "0::/bad slice.slice/demo.service\n"},
		{name: "malformed slice escape", input: "0::/bad\\qslice.slice/demo.service\n"},
		{name: "oversized slice ancestor", input: "0::/" + strings.Repeat("x", 250) + ".slice/demo.service\n"},
		{name: "malformed user app slice", input: "0::/user.slice/user-1000.slice/user@1000.service/bad app.slice/demo.service\n"},
		{name: "malformed service-free slice", input: "0::/bad slice.slice/pod.scope\n"},
		{name: "untrusted root service", input: "0::/pod.service/child\n"},
		{name: "untrusted container service", input: "0::/kubepods/pod.service/child\n"},
		{name: "untrusted mixed ancestry service", input: "0::/custom.slice/not-a-slice/demo.service\n"},
		{name: "relative path", input: "0::system.slice/demo.service\n"},
		{name: "malformed row", input: "0:/system.slice/demo.service\n"},
		{name: "malformed unified hierarchy", input: "0:cpu:/system.slice/demo.service\n1:name=systemd:/system.slice/demo.service\n"},
		{name: "malformed duplicate controller", input: "1:name=systemd,name=systemd:/system.slice/demo.service\n"},
		{name: "control in unit", input: "0::/system.slice/de\x01mo.service\n"},
		{name: "space in unit", input: "0::/system.slice/demo worker.service\n"},
		{name: "unknown escape", input: "0::/system.slice/demo\\qworker.service\n"},
		{name: "short escape", input: "0::/system.slice/demo\\x2.service\n"},
		{name: "non-hex escape", input: "0::/system.slice/demo\\xGG.service\n"},
		{name: "dangling escape", input: "0::/system.slice/demo\\.service\n"},
		{name: "non-ascii unit", input: "0::/system.slice/d\xc3\xa9m\xc3\xb8.service\n"},
		{name: "empty unit prefix", input: "0::/system.slice/.service\n"},
		{name: "empty at prefix", input: "0::/system.slice/@blue.service\n"},
		{name: "multiple literal at", input: "0::/system.slice/worker@blue@second.service\n"},
		{name: "256 byte unit", input: "0::/system.slice/" + strings.Repeat("x", 248) + ".service\n"},
		{name: "ambiguous units", input: "0::/system.slice/alpha.service\n1:name=systemd:/system.slice/beta.service\n"},
		{name: "ambiguous scopes", input: "0::/system.slice/demo.service\n1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/demo.service\n"},
		{name: "mixed service and scope", input: "0::/system.slice/demo.service\n1:name=systemd:/system.slice/session.scope\n"},
		{name: "duplicate unified hierarchy", input: "0::/system.slice/demo.service\n0::/system.slice/demo.service\n"},
		{name: "duplicate v1 hierarchy", input: "1:name=systemd:/system.slice/demo.service\n1:cpu:/system.slice/demo.service\n"},
		{name: "duplicate v1 controller across rows", input: "1:name=systemd:/system.slice/demo.service\n2:name=systemd:/system.slice/demo.service\n"},
		{name: "duplicate ordinary controller across rows", input: "1:cpu,cpuacct:/\n2:cpu:/\n"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			got := parseSystemdCgroup([]byte(test.input))
			if got.state != test.wantState || !reflect.DeepEqual(got.supervisor, test.wantSupervisor) {
				t.Fatalf("parseSystemdCgroup() = %+v, want state %d supervisor %+v", got, test.wantState, test.wantSupervisor)
			}
		})
	}
}

func TestReadSystemdCgroupRejectsOversizedCgroup(t *testing.T) {
	path := filepath.Join(t.TempDir(), "cgroup")
	data := "0::/system.slice/demo.service\n" + strings.Repeat("x", 64*1024)
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatal(err)
	}
	got, err := readSystemdCgroup(path)
	if err != nil || got.state != systemdCgroupUncertain || got.supervisor != nil {
		t.Fatalf("readSystemdCgroup() = %+v, %v", got, err)
	}
}

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
	if err := os.WriteFile(filepath.Join(pidDir, "cgroup"), []byte("0::/codeasier.slice/router.slice/demo.service/delegated/child\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	target, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, func(pid int) (process.Identity, error) {
		if pid != 4242 {
			t.Fatalf("pid = %d", pid)
		}
		return wantProcess, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	identity := target.Identity
	if identity.ListenAddr != "127.0.0.1:19099" || identity.Network != "tcp4" || identity.SocketID != "601" || identity.Process != wantProcess || identity.UserID != "1001" {
		t.Fatalf("identity = %+v", identity)
	}
	wantSupervisor := &Supervisor{Kind: SupervisorSystemdSystem, Scope: SupervisorScopeSystem, Identifiers: []string{"demo.service"}}
	if target.Mode != VerificationModeVerifiedIdentity || target.PID != 4242 || target.ListenAddr != identity.ListenAddr || !reflect.DeepEqual(target.Supervisor, wantSupervisor) {
		t.Fatalf("target = %+v, want supervisor %+v", target, wantSupervisor)
	}
	inspection := inspectLinuxTargetRecovery(t, target)
	if inspection.Recovery.Action != RecoveryActionManualStopRequired || inspection.Recovery.Reason != RecoveryReasonServiceManaged || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
		t.Fatalf("inspection = %+v", inspection)
	}
}

func TestInspectLinuxClassifiesDelegatedUserService(t *testing.T) {
	root := writeLinuxProcessFixture(t, "0::/user.slice/user-1000.slice/user@1000.service/app.slice/demo@blue.service/delegated/child\n")
	target, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, testLinuxProcessIdentity)
	if err != nil {
		t.Fatal(err)
	}
	wantSupervisor := &Supervisor{Kind: SupervisorSystemdUser, Scope: SupervisorScopeUser, Identifiers: []string{"demo@blue.service"}}
	if !reflect.DeepEqual(target.Supervisor, wantSupervisor) || target.BlockReason != "" {
		t.Fatalf("target = %+v, want supervisor %+v", target, wantSupervisor)
	}
	inspection := inspectLinuxTargetRecovery(t, target)
	if inspection.Recovery.Action != RecoveryActionManualStopRequired || inspection.Recovery.Reason != RecoveryReasonServiceManaged || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
		t.Fatalf("inspection = %+v", inspection)
	}
}

func TestInspectLinuxLeavesOrdinaryTargetForceable(t *testing.T) {
	root := writeLinuxProcessFixture(t, "0::/user.slice/user-1000.slice/user@1000.service/app.slice/session.scope\n")
	target, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, func(int) (process.Identity, error) {
		return process.Identity{PID: 4242, StartedAt: "123", Executable: "/tmp/listener"}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if target.Mode != VerificationModeVerifiedIdentity || target.Supervisor != nil || target.BlockReason != "" {
		t.Fatalf("target = %+v", target)
	}
	inspection := inspectLinuxTargetRecovery(t, target)
	if inspection.Recovery.Action != RecoveryActionForceTerminate || inspection.ConfirmationToken == "" || inspection.ExpiresAt == nil {
		t.Fatalf("inspection = %+v", inspection)
	}
}

func TestInspectLinuxCgroupUncertaintyCannotMintToken(t *testing.T) {
	tests := []struct {
		name    string
		cgroup  string
		prepare func(*testing.T, string)
	}{
		{
			name:   "unreadable",
			cgroup: "0::/system.slice/demo.service\n",
			prepare: func(t *testing.T, path string) {
				if err := os.Remove(path); err != nil {
					t.Fatal(err)
				}
				if err := os.Mkdir(path, 0o755); err != nil {
					t.Fatal(err)
				}
			},
		},
		{
			name:   "missing while pid exists",
			cgroup: "0::/system.slice/demo.service\n",
			prepare: func(t *testing.T, path string) {
				if err := os.Remove(path); err != nil {
					t.Fatal(err)
				}
			},
		},
		{name: "oversized", cgroup: "0::/system.slice/demo.service\n" + strings.Repeat("x", 64*1024)},
		{name: "malformed", cgroup: "malformed\n"},
		{name: "malformed unit", cgroup: "0::/system.slice/demo worker.service\n"},
		{name: "conflicting", cgroup: "0::/system.slice/alpha.service\n1:name=systemd:/system.slice/beta.service\n"},
		{name: "ambiguous scope", cgroup: "0::/system.slice/demo.service\n1:name=systemd:/system.slice/session.scope\n"},
		{name: "multiple delegated services", cgroup: "0::/system.slice/outer.service/inner.service/child\n"},
		{name: "duplicate hierarchy", cgroup: "0::/system.slice/demo.service\n0::/system.slice/demo.service\n"},
		{name: "duplicate controller", cgroup: "1:name=systemd:/system.slice/demo.service\n2:name=systemd:/system.slice/demo.service\n"},
		{name: "malformed slice", cgroup: "0::/bad slice.slice/demo.service\n"},
		{name: "oversized slice", cgroup: "0::/" + strings.Repeat("x", 250) + ".slice/demo.service\n"},
		{name: "malformed service-free slice", cgroup: "0::/bad slice.slice/pod.scope\n"},
		{name: "malformed user slice", cgroup: "0::/user.slice/user-1000.slice/user@1000.service/bad app.slice/demo.service\n"},
		{name: "untrusted service path", cgroup: "0::/kubepods/pod.service/child\n"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			root := writeLinuxProcessFixture(t, test.cgroup)
			if test.prepare != nil {
				test.prepare(t, filepath.Join(root, "4242", "cgroup"))
			}
			target, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, testLinuxProcessIdentity)
			if err != nil {
				t.Fatal(err)
			}
			if target.Mode != VerificationModeVerifiedIdentity || target.Supervisor != nil || target.BlockReason != RecoveryReasonIdentityUnavailable {
				t.Fatalf("target = %+v", target)
			}
			inspection := inspectLinuxTargetRecovery(t, target)
			if inspection.Recovery.Action != RecoveryActionUnavailable || inspection.Recovery.Reason != RecoveryReasonIdentityUnavailable || inspection.ConfirmationToken != "" || inspection.ExpiresAt != nil {
				t.Fatalf("inspection = %+v", inspection)
			}
		})
	}
}

func TestInspectLinuxCgroupDisappearanceReturnsNotFound(t *testing.T) {
	root := writeLinuxProcessFixture(t, "0::/system.slice/demo.service\n")
	inspectProcess := func(int) (process.Identity, error) {
		if err := os.RemoveAll(filepath.Join(root, "4242")); err != nil {
			t.Fatal(err)
		}
		return testLinuxProcessIdentity(4242)
	}
	if _, err := inspectLinux(context.Background(), "127.0.0.1:19099", root, inspectProcess); !errors.Is(err, ErrNotFound) {
		t.Fatalf("error = %v, want %v", err, ErrNotFound)
	}
}

func TestServiceForceTerminateMapsLinuxCgroupDisappearanceToChanged(t *testing.T) {
	root := writeLinuxProcessFixture(t, "0::/user.slice/user-1000.slice/user@1000.service/app.slice/session.scope\n")
	inspectCalls := 0
	service := New(Config{ListenAddr: "127.0.0.1:19099"}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect: func(ctx context.Context, listenAddr string) (Target, error) {
			inspectCalls++
			if inspectCalls == 1 {
				return inspectLinux(ctx, listenAddr, root, testLinuxProcessIdentity)
			}
			return inspectLinux(ctx, listenAddr, root, func(int) (process.Identity, error) {
				if err := os.RemoveAll(filepath.Join(root, "4242")); err != nil {
					t.Fatal(err)
				}
				return testLinuxProcessIdentity(4242)
			})
		},
		CurrentUser: func() (string, error) { return "1001", nil },
		Random:      bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil || inspection.ConfirmationToken == "" {
		t.Fatalf("inspection = %+v, error = %v", inspection, err)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrChanged) {
		t.Fatalf("error = %v, want %v", err, ErrChanged)
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

func writeLinuxProcessFixture(t *testing.T, cgroup string) string {
	t.Helper()
	root := writeProcNet(t, map[string]string{"tcp": procTCPHeader + procTCPRow("0100007F:4A9B", "601")})
	pidDir := filepath.Join(root, "4242")
	if err := os.MkdirAll(filepath.Join(pidDir, "fd"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pidDir, "status"), []byte("Uid:\t1000\t1001\t1002\t1003\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(pidDir, "cgroup"), []byte(cgroup), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("socket:[601]", filepath.Join(pidDir, "fd", "7")); err != nil {
		t.Fatal(err)
	}
	return root
}

func testLinuxProcessIdentity(int) (process.Identity, error) {
	return process.Identity{PID: 4242, StartedAt: "123", Executable: "/tmp/listener"}, nil
}

func inspectLinuxTargetRecovery(t *testing.T, target Target) Inspection {
	t.Helper()
	service := New(Config{ListenAddr: target.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Target, error) { return target, nil },
		CurrentUser: func() (string, error) { return target.Identity.UserID, nil },
		Random:      bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	return inspection
}
