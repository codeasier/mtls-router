package routerenroll

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func (n NginxRuntime) reloadWorkers(ctx context.Context) ([]int, error) {
	if !filepath.IsAbs(n.PidFile) {
		return nil, ErrGateway
	}
	data, err := readPublicBounded(n.PidFile, 64)
	if err != nil {
		return nil, ErrGateway
	}
	master, err := strconv.Atoi(strings.TrimSpace(string(data)))
	if err != nil || master <= 1 {
		return nil, ErrGateway
	}
	output, err := exec.CommandContext(ctx, "ps", "-A", "-o", "pid=,ppid=,uid=").Output()
	if err != nil {
		return nil, ErrGateway
	}
	return nginxWorkerPIDs(string(output), master, os.Geteuid())
}

func nginxWorkerPIDs(output string, master, callerUID int) ([]int, error) {
	var workers []int
	found := false
	for _, line := range strings.Split(output, "\n") {
		fields := strings.Fields(line)
		if len(fields) == 0 {
			continue
		}
		if len(fields) != 3 {
			return nil, ErrGateway
		}
		pid, e1 := strconv.Atoi(fields[0])
		parent, e2 := strconv.Atoi(fields[1])
		uid, e3 := strconv.Atoi(fields[2])
		if e1 != nil || e2 != nil || e3 != nil || pid <= 0 || parent < 0 {
			return nil, ErrGateway
		}
		if pid != master && parent != master {
			continue
		}
		if uid < 0 {
			return nil, ErrGateway
		}
		// lsof can silently hide another user's descriptors. Such an empty
		// result is not proof of listener retirement; the loader must own the
		// processes or run as root, as required for the nginx reload itself.
		if callerUID != 0 && uid != callerUID {
			return nil, ErrGateway
		}
		if pid == master {
			found = true
		} else {
			workers = append(workers, pid)
		}
	}
	if !found || len(workers) == 0 {
		return nil, ErrGateway
	}
	return workers, nil
}

func nginxListeningWorkers(ctx context.Context, workers []int) ([]int, error) {
	if len(workers) == 0 {
		return nil, ErrGateway
	}
	ids := make([]string, len(workers))
	allowed := make(map[int]bool, len(workers))
	for i, pid := range workers {
		if pid <= 1 {
			return nil, ErrGateway
		}
		ids[i], allowed[pid] = strconv.Itoa(pid), true
	}
	cmd := exec.CommandContext(ctx, "lsof", "-nP", "-a", "-p", strings.Join(ids, ","), "-iTCP", "-sTCP:LISTEN", "-t")
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	data, err := cmd.Output()
	if ctx.Err() != nil || stderr.Len() != 0 {
		return nil, ErrGateway
	}
	if err != nil {
		if exit, ok := err.(*exec.ExitError); !ok || exit.ExitCode() != 1 || len(data) != 0 {
			return nil, ErrGateway
		}
		return nil, nil // lsof exit 1 with no output: no matching listeners.
	}
	var listening []int
	for _, field := range strings.Fields(string(data)) {
		pid, err := strconv.Atoi(field)
		if err != nil || !allowed[pid] {
			return nil, ErrGateway
		}
		listening = append(listening, pid)
	}
	return listening, nil
}

func waitNginxWorkersRetired(ctx context.Context, workers []int, scan func(context.Context, []int) ([]int, error)) error {
	tick := time.NewTicker(25 * time.Millisecond)
	defer tick.Stop()
	for {
		listening, err := scan(ctx, workers)
		if err != nil || ctx.Err() != nil {
			return ErrGateway
		}
		if len(listening) == 0 {
			return nil
		}
		select {
		case <-ctx.Done():
			return ErrGateway
		case <-tick.C:
		}
	}
}
