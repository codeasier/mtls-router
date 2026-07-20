//go:build !windows

package agent

import "os"

func isFinalComponentLink(_ string, info os.FileInfo) bool {
	return info.Mode()&os.ModeSymlink != 0
}
