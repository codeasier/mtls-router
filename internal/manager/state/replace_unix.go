//go:build !windows

package state

import "os"

func replaceFile(from, to string) error {
	return os.Rename(from, to)
}

func syncDir(path string) error {
	dir, err := os.Open(path)
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
