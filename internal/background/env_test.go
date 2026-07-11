package background

import (
	"reflect"
	"testing"
)

func TestChildEnvFiltersBackendCaseInsensitivelyAndPreservesOtherEntries(t *testing.T) {
	env := []string{
		"PATH=/usr/bin",
		"MTLS_BACKEND=true",
		"HOME=/tmp/home",
		"mtls_backend=false",
		"MtLs_BaCkEnD=1",
		"MTLS_BACKEND_EXTRA=true",
		"INVALID_ENTRY",
	}

	got := ChildEnv(env)
	want := []string{
		"PATH=/usr/bin",
		"HOME=/tmp/home",
		"MTLS_BACKEND_EXTRA=true",
		"INVALID_ENTRY",
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildEnv() = %#v, want %#v", got, want)
	}
}

func TestChildEnvDoesNotModifyInput(t *testing.T) {
	env := []string{"PATH=/usr/bin", "MTLS_BACKEND=true", "HOME=/tmp/home"}
	want := append([]string(nil), env...)

	_ = ChildEnv(env)

	if !reflect.DeepEqual(env, want) {
		t.Fatalf("input after ChildEnv() = %#v, want %#v", env, want)
	}
}

func TestChildArgsAndEnvRemoveBackendConfiguration(t *testing.T) {
	gotArgs := ChildArgs([]string{"--backend=true", "-debug"}, "/tmp/default.log")
	wantArgs := []string{"-debug", "-log", "/tmp/default.log"}
	if !reflect.DeepEqual(gotArgs, wantArgs) {
		t.Fatalf("ChildArgs() = %#v, want %#v", gotArgs, wantArgs)
	}

	gotEnv := ChildEnv([]string{"MTLS_BACKEND=true", "MTLS_DEBUG=true"})
	wantEnv := []string{"MTLS_DEBUG=true"}
	if !reflect.DeepEqual(gotEnv, wantEnv) {
		t.Fatalf("ChildEnv() = %#v, want %#v", gotEnv, wantEnv)
	}
}
