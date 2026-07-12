package agent

import (
	"errors"
	"testing"
)

func TestPostReplaceErrorReportsReplacement(t *testing.T) {
	inner := errors.New("sync failed")
	err := &postReplaceError{err: inner}
	if !replacementOccurred(err) || !errors.Is(err, inner) {
		t.Fatalf("post-replace error classification failed: %v", err)
	}
	if replacementOccurred(inner) {
		t.Fatal("ordinary error was classified as a completed replacement")
	}
}
