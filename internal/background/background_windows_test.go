//go:build windows

package background

import (
	"testing"

	"golang.org/x/sys/windows"
)

func TestWindowsSysProcAttrDetachesHiddenProcess(t *testing.T) {
	attr := windowsSysProcAttr()

	flags := attr.CreationFlags
	if flags&windows.DETACHED_PROCESS == 0 {
		t.Fatalf("CreationFlags %#x missing DETACHED_PROCESS %#x", flags, windows.DETACHED_PROCESS)
	}
	if flags&windows.CREATE_NEW_PROCESS_GROUP == 0 {
		t.Fatalf("CreationFlags %#x missing CREATE_NEW_PROCESS_GROUP %#x", flags, windows.CREATE_NEW_PROCESS_GROUP)
	}
	if !attr.HideWindow {
		t.Fatal("HideWindow = false, want true")
	}
}
