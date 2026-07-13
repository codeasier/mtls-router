//go:build !windows

package state

import "os"

func restrictPath(path string, directory bool) error {
	if directory {
		return os.Chmod(path, 0o700)
	}
	return os.Chmod(path, 0o600)
}
