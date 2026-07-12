//go:build !windows

package lifecycle

import "os/exec"

func configureDesktopCommand(*exec.Cmd) {}
