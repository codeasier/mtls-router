package agent

import (
	"errors"
	"os"
	"path/filepath"
)

func inspectRecoveryTarget(role, path string, format Format) RecoveryFileState {
	file := RecoveryFileState{Role: role, Path: path, Format: format}
	info, err := os.Lstat(path)
	if err == nil {
		file.Exists = true
		if isFinalComponentLink(path, info) {
			file.Reasons = appendReason(file.Reasons, RecoveryLinked)
		} else if !info.Mode().IsRegular() {
			file.Reasons = appendReason(file.Reasons, RecoveryNonRegular)
		} else if !pathWritable(path) {
			file.Reasons = appendReason(file.Reasons, RecoveryNotWritable)
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		file.Reasons = appendReason(file.Reasons, RecoveryUnreadable)
	}

	parent := filepath.Dir(path)
	parentInfo, parentErr := os.Stat(parent)
	if parentErr != nil || !parentInfo.IsDir() || parentInfo.Mode().Perm()&0200 == 0 {
		file.Reasons = appendReason(file.Reasons, RecoveryParentUnavailable)
	}
	return file
}

func finalizeRecovery(state *State) {
	var reasons []RecoveryReason
	hasSyntaxInvalid := false
	blocked := false
	for i := range state.Recovery.Files {
		for _, reason := range state.Recovery.Files[i].Reasons {
			reasons = appendReason(reasons, reason)
			if reason == RecoverySyntaxInvalid {
				hasSyntaxInvalid = true
			} else {
				blocked = true
			}
		}
	}
	state.Recovery.Reasons = reasons
	state.Recovery.Eligible = hasSyntaxInvalid && !blocked
}

func hasBlockingFileReason(reasons []RecoveryReason) bool {
	for _, reason := range reasons {
		if reason != RecoverySyntaxInvalid {
			return true
		}
	}
	return false
}

func hasInvalidFileReason(reasons []RecoveryReason) bool {
	for _, reason := range reasons {
		switch reason {
		case RecoverySyntaxInvalid, RecoveryUnsupportedStructure, RecoveryUnreadable, RecoveryOversized, RecoveryNonRegular, RecoveryLinked:
			return true
		}
	}
	return false
}

func appendReason(reasons []RecoveryReason, reason RecoveryReason) []RecoveryReason {
	for _, existing := range reasons {
		if existing == reason {
			return reasons
		}
	}
	return append(reasons, reason)
}
