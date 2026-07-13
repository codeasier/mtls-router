//go:build windows

package state

import (
	"golang.org/x/sys/windows"
)

func replaceFile(from, to string) error {
	fromPath, err := windows.UTF16PtrFromString(from)
	if err != nil {
		return err
	}
	toPath, err := windows.UTF16PtrFromString(to)
	if err != nil {
		return err
	}
	return windows.MoveFileEx(fromPath, toPath, windows.MOVEFILE_REPLACE_EXISTING|windows.MOVEFILE_WRITE_THROUGH)
}

func syncDir(string) error {
	return nil
}
