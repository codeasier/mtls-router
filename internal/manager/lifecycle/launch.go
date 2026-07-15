package lifecycle

import (
	"io"
)

type foregroundProcess interface {
	PID() int
	Wait() error
}

func launchForeground(executable string, args, env []string, output io.Writer) (foregroundProcess, error) {
	return launchForegroundCommand(executable, args, env, output)
}
