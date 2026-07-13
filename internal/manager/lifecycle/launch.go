package lifecycle

import (
	"io"
	"os/exec"
)

type foregroundProcess interface {
	PID() int
	Wait() error
}

type commandProcess struct {
	cmd *exec.Cmd
}

func (p commandProcess) PID() int    { return p.cmd.Process.Pid }
func (p commandProcess) Wait() error { return p.cmd.Wait() }

func launchForeground(executable string, args, env []string, output io.Writer) (foregroundProcess, error) {
	cmd := exec.Command(executable, args...)
	cmd.Env = env
	cmd.Stdin = nil
	cmd.Stdout = output
	cmd.Stderr = output
	configureDesktopCommand(cmd)
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	return commandProcess{cmd: cmd}, nil
}
