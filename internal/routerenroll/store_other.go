//go:build !unix

package routerenroll

import (
	"context"
	"os"
)

// The single signing master is deployed on Unix; never claim chmod alone is a
// private Windows DACL. Windows client identity storage belongs to Rust.
func privateOwner(os.FileInfo) bool              { return false }
func acquireWriterLock(string) (*os.File, error) { return nil, ErrStorage }
func openCommitLock(string) (*os.File, error)    { return nil, ErrContinuity }
func lockCommit(context.Context, *os.File) error { return ErrContinuity }
func unlockCommit(*os.File)                      {}
