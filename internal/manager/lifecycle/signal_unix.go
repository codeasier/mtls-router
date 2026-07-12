//go:build !windows

package lifecycle

import "os"

func gracefulSignal() os.Signal { return os.Interrupt }
