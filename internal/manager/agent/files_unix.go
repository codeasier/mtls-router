//go:build !windows

package agent

import "os"

func replaceAtomic(from, to string) error { return os.Rename(from, to) }

func syncDirectory(path string) error {
	dir, err := os.Open(path)
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}

func restrictPrivate(path string, directory bool) error {
	if directory {
		return os.Chmod(path, 0o700)
	}
	return os.Chmod(path, 0o600)
}

func applyPrivateMode(path string, source os.FileMode) error {
	mode := source.Perm() & 0o700
	if mode&0o400 == 0 {
		mode |= 0o400
	}
	return os.Chmod(path, mode)
}

func applyTargetPermissions(path, _ string, mode os.FileMode) error {
	return os.Chmod(path, mode.Perm())
}
