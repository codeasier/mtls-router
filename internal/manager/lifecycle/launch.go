package lifecycle

import (
	"io"
	"time"
)

const foregroundWaitDelay = 250 * time.Millisecond

type foregroundProcess interface {
	PID() int
	Kill() error
	Wait() error
}

func launchForeground(executable string, args, env []string, output io.Writer) (foregroundProcess, error) {
	return launchForegroundCommand(executable, args, env, output)
}
