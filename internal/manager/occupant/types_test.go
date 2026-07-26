package occupant

import (
	"fmt"
	"strings"
	"testing"
)

func TestValidSupervisorIdentifierGrammar(t *testing.T) {
	tests := []struct {
		name       string
		kind       SupervisorKind
		scope      SupervisorScope
		identifier string
		want       bool
	}{
		{name: "windows service", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: "Router Service", want: true},
		{name: "windows angle ampersand", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: "Svc<>&", want: true},
		{name: "windows whitespace", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: " ", want: false},
		{name: "windows slash", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: "Svc/name", want: false},
		{name: "windows backslash", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: `Svc\name`, want: false},
		{name: "windows comma", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: "Svc,name", want: false},
		{name: "windows quote", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: `Svc"name`, want: false},
		{name: "windows control", kind: SupervisorWindowsService, scope: SupervisorScopeSystem, identifier: "Svc\nname", want: false},
		{name: "systemd escaped template", kind: SupervisorSystemdUser, scope: SupervisorScopeUser, identifier: `demo\x2dworker@blue.service`, want: true},
		{name: "systemd maximum length", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: strings.Repeat("x", 247) + ".service", want: true},
		{name: "systemd empty instance", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: "worker@.service", want: true},
		{name: "systemd leading at", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: "@demo.service", want: false},
		{name: "systemd repeated at", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: "a@b@c.service", want: false},
		{name: "systemd invalid escape", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: `demo\q20.service`, want: false},
		{name: "systemd short escape", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: `demo\x2.service`, want: false},
		{name: "systemd whitespace", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: "demo scope.service", want: false},
		{name: "systemd angle ampersand", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: "demo<>&.service", want: false},
		{name: "systemd over maximum length", kind: SupervisorSystemdSystem, scope: SupervisorScopeSystem, identifier: strings.Repeat("x", 248) + ".service", want: false},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			supervisor := &Supervisor{Kind: test.kind, Scope: test.scope, Identifiers: []string{test.identifier}}
			if got := validSupervisor(supervisor); got != test.want {
				t.Fatalf("validSupervisor(%q) = %t, want %t", test.identifier, got, test.want)
			}
		})
	}
}

func TestValidSupervisorSizeDoesNotHTMLEscapeWindowsIdentifiers(t *testing.T) {
	identifiers := make([]string, 16)
	for index := range identifiers {
		identifiers[index] = fmt.Sprintf("%02d%s", index, strings.Repeat("<>&", 60))
	}
	supervisor := &Supervisor{Kind: SupervisorWindowsService, Scope: SupervisorScopeSystem, Identifiers: identifiers}
	if !validSupervisor(supervisor) {
		t.Fatal("validSupervisor rejected metadata within the shared encoded-size bound")
	}
}
