//go:build !windows

package lifecycle

import (
	"errors"
	"io"
	"os/exec"
	"testing"
	"time"
)

func TestForegroundWaitDelayBoundsInheritedOutputPipe(t *testing.T) {
	child, err := launchForegroundCommand(
		"/bin/sh",
		[]string{"-c", "sleep 1 & exit 0"},
		nil,
		io.Discard,
	)
	if err != nil {
		t.Fatal(err)
	}
	started := time.Now()
	waitErr := child.Wait()
	if !errors.Is(waitErr, exec.ErrWaitDelay) {
		t.Fatalf("Wait error = %v, want exec.ErrWaitDelay", waitErr)
	}
	if elapsed := time.Since(started); elapsed >= 500*time.Millisecond {
		t.Fatalf("Wait remained blocked on inherited output pipe for %s", elapsed)
	}
}
