//go:build unix

package routerenroll

import (
	"context"
	"errors"
	"reflect"
	"syscall"
	"testing"
	"time"
)

func exerciseNginxReloadRetirement(t *testing.T, runtime NginxRuntime) {
	t.Helper()
	workers, err := runtime.reloadWorkers(context.Background())
	if err != nil {
		t.Fatal("snapshot isolated nginx workers", err)
	}
	func() {
		var stopped []int
		defer func() {
			for _, pid := range stopped {
				_ = syscall.Kill(pid, syscall.SIGCONT)
			}
		}()
		for _, pid := range workers {
			if syscall.Kill(pid, syscall.SIGSTOP) != nil {
				t.Fatal("stop isolated old nginx worker")
			}
			stopped = append(stopped, pid)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 750*time.Millisecond)
		defer cancel()
		if runtime.Reload(ctx) == nil {
			t.Fatal("reload acknowledged while a stopped old worker still owned its listener")
		}
	}()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if waitNginxWorkersRetired(ctx, workers, nginxListeningWorkers) != nil {
		t.Fatal("resumed old workers did not retire listeners")
	}
	t.Log("old-worker listener retirement required; a new worker/marker alone cannot acknowledge reload")
}

func TestNginxWorkerSnapshotCannotHideOtherUsersOrMissingMaster(t *testing.T) {
	rows := "100 1 0\n101 100 33\n102 100 33\n200 1 1000\n201 1 -2\n"
	workers, err := nginxWorkerPIDs(rows, 100, 0)
	if err != nil || !reflect.DeepEqual(workers, []int{101, 102}) {
		t.Fatal("worker snapshot", workers, err)
	}
	if _, err := nginxWorkerPIDs(rows, 100, 1000); err == nil {
		t.Fatal("unreadable root-owned descriptors could be mistaken for retirement")
	}
	for _, bad := range []string{"101 100 1000\n", "100 1 1000\n", "100 1 bad\n101 100 1000\n"} {
		if _, err := nginxWorkerPIDs(bad, 100, 1000); err == nil {
			t.Fatal("incomplete snapshot accepted")
		}
	}
}

func TestNginxRetirementWaitsForEveryOldListenerNotLiveStreams(t *testing.T) {
	workers := []int{101, 102}
	calls := 0
	err := waitNginxWorkersRetired(context.Background(), workers, func(_ context.Context, got []int) ([]int, error) {
		if !reflect.DeepEqual(got, workers) {
			t.Fatal("retirement scope changed")
		}
		calls++
		switch calls {
		case 1:
			return []int{101, 102}, nil
		case 2:
			return []int{102}, nil
		default:
			// Both processes may still hold established SSE sockets. They no
			// longer listen, so waiting for process exit would be incorrect.
			return nil, nil
		}
	})
	if err != nil || calls != 3 {
		t.Fatal("ack before every listener retired", calls, err)
	}
}

func TestNginxRetirementTimeoutAndInspectionFailureDoNotAck(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	if err := waitNginxWorkersRetired(ctx, []int{101}, func(context.Context, []int) ([]int, error) {
		return []int{101}, nil
	}); err == nil {
		t.Fatal("live old listener acknowledged")
	}
	if err := waitNginxWorkersRetired(context.Background(), []int{101}, func(context.Context, []int) ([]int, error) {
		return nil, errors.New("inspection-failed")
	}); err == nil {
		t.Fatal("unknown listener state acknowledged")
	}
}
