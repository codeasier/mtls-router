package background

import (
	"reflect"
	"testing"
)

func TestChildArgsRemovesLongBackendAndAppendsDefaultLog(t *testing.T) {
	got := ChildArgs([]string{"--backend", "-listen", "127.0.0.1:19099"}, "/tmp/default.log")
	want := []string{"-listen", "127.0.0.1:19099", "-log", "/tmp/default.log"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestChildArgsRemovesShortBackendAndAppendsDefaultLog(t *testing.T) {
	got := ChildArgs([]string{"-backend", "-debug"}, "/tmp/default.log")
	want := []string{"-debug", "-log", "/tmp/default.log"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestChildArgsRemovesLongBackendEqualsAndAppendsDefaultLog(t *testing.T) {
	got := ChildArgs([]string{"--backend=true", "-listen", "127.0.0.1:19099"}, "/tmp/default.log")
	want := []string{"-listen", "127.0.0.1:19099", "-log", "/tmp/default.log"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestChildArgsRemovesShortBackendEqualsAndAppendsDefaultLog(t *testing.T) {
	got := ChildArgs([]string{"-backend=true", "-debug"}, "/tmp/default.log")
	want := []string{"-debug", "-log", "/tmp/default.log"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestChildArgsPreservesLongLogPair(t *testing.T) {
	got := ChildArgs([]string{"--backend", "--log", "/tmp/custom.log", "-debug"}, "/tmp/default.log")
	want := []string{"--log", "/tmp/custom.log", "-debug"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestChildArgsPreservesShortLogPair(t *testing.T) {
	got := ChildArgs([]string{"-backend", "-log", "/tmp/custom.log"}, "/tmp/default.log")
	want := []string{"-log", "/tmp/custom.log"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestChildArgsPreservesEqualsLog(t *testing.T) {
	got := ChildArgs([]string{"--backend", "--log=/tmp/custom.log"}, "/tmp/default.log")
	want := []string{"--log=/tmp/custom.log"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("ChildArgs() = %#v, want %#v", got, want)
	}
}

func TestDefaultLogPathUsesBinaryDirectory(t *testing.T) {
	got := DefaultLogPath("/opt/mtls-router/mtls-router")
	want := "/opt/mtls-router/mtls-router.log"
	if got != want {
		t.Fatalf("DefaultLogPath() = %q, want %q", got, want)
	}
}
