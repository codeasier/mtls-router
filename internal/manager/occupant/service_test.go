package occupant

import (
	"bytes"
	"context"
	"errors"
	"net"
	"os"
	"sync"
	"testing"
	"time"

	"github.com/codeasier/mtls-router/internal/manager/discovery"
	"github.com/codeasier/mtls-router/internal/manager/process"
)

func TestServiceInspectionTokenAndTermination(t *testing.T) {
	now := time.Date(2026, 7, 18, 1, 2, 3, 0, time.UTC)
	identity := testIdentity()
	status := process.StatusGenuine
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Identity, error) { return identity, nil },
		CurrentUser: func() (string, error) { return identity.UserID, nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return status, nil },
		Signal: func(got process.Identity, signal os.Signal) error {
			if signal != os.Kill || got != identity.Process {
				t.Fatalf("signal = %v identity = %+v", signal, got)
			}
			signals++
			status = process.StatusAbsent
			return nil
		},
		Dial:   func(context.Context, string, string) (net.Conn, error) { return nil, errors.New("refused") },
		Random: bytes.NewReader(bytes.Repeat([]byte{7}, 64)), Now: func() time.Time { return now },
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(inspection.ConfirmationToken) != 43 || !inspection.ExpiresAt.Equal(now.Add(30*time.Second)) || inspection.ProcessName != "listener" {
		t.Fatalf("inspection = %+v", inspection)
	}
	result, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken)
	if err != nil || result.State != "absent" || signals != 1 {
		t.Fatalf("result=%+v err=%v signals=%d", result, err, signals)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) || signals != 1 {
		t.Fatalf("replay err=%v signals=%d", err, signals)
	}
}

func TestServiceExpirySupersessionAndRestart(t *testing.T) {
	now := time.Now()
	newService := func() *Service {
		return New(Config{ListenAddr: testIdentity().ListenAddr}, Dependencies{
			Discover: func(context.Context) discovery.Result {
				return discovery.Result{Classification: discovery.UnknownOccupant}
			},
			Inspect:     func(context.Context, string) (Identity, error) { return testIdentity(), nil },
			CurrentUser: func() (string, error) { return "user", nil },
			SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
			Validate:    func(process.Identity, string) (process.Status, error) { return process.StatusGenuine, nil },
			Random:      bytes.NewReader(append(bytes.Repeat([]byte{9}, 32), bytes.Repeat([]byte{10}, 96)...)),
			Now:         func() time.Time { return now },
		})
	}
	service := newService()
	first, _ := service.Inspect(context.Background())
	second, _ := service.Inspect(context.Background())
	if _, err := service.ForceTerminate(context.Background(), first.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) {
		t.Fatalf("superseded token error = %v", err)
	}
	if _, err := newService().ForceTerminate(context.Background(), second.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) {
		t.Fatalf("restart token error = %v", err)
	}
	third, _ := service.Inspect(context.Background())
	now = now.Add(30 * time.Second)
	if _, err := service.ForceTerminate(context.Background(), third.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) {
		t.Fatalf("expired token error = %v", err)
	}
}

func TestServiceRejectsChangedOwnedAndProtectedWithoutSignal(t *testing.T) {
	for _, test := range []struct {
		name      string
		mutate    func(*Identity, *Config)
		wantError error
	}{
		{name: "other user", mutate: func(identity *Identity, _ *Config) { identity.UserID = "other" }, wantError: ErrNotOwned},
		{name: "desktop", mutate: func(identity *Identity, config *Config) { config.DesktopPID = identity.Process.PID }, wantError: ErrProtected},
		{name: "manager", mutate: func(identity *Identity, config *Config) { config.ManagerIdentity = identity.Process }, wantError: ErrProtected},
		{name: "managed router", mutate: func(_ *Identity, config *Config) { config.IsProtected = func(Identity) bool { return true } }, wantError: ErrProtected},
	} {
		t.Run(test.name, func(t *testing.T) {
			identity := testIdentity()
			config := Config{ListenAddr: identity.ListenAddr}
			test.mutate(&identity, &config)
			signals := 0
			service := New(config, Dependencies{
				Discover: func(context.Context) discovery.Result {
					return discovery.Result{Classification: discovery.UnknownOccupant}
				},
				Inspect: func(context.Context, string) (Identity, error) { return identity, nil }, CurrentUser: func() (string, error) { return "user", nil },
				Signal: func(process.Identity, os.Signal) error { signals++; return nil }, Random: bytes.NewReader(make([]byte, 32)),
			})
			if _, err := service.Inspect(context.Background()); !errors.Is(err, test.wantError) || signals != 0 {
				t.Fatalf("error=%v signals=%d", err, signals)
			}
		})
	}
}

func TestServiceConcurrentTokenConsumptionSignalsOnce(t *testing.T) {
	identity := testIdentity()
	status := process.StatusGenuine
	var signalMu sync.Mutex
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect: func(context.Context, string) (Identity, error) { return identity, nil }, CurrentUser: func() (string, error) { return "user", nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return status, nil },
		Signal: func(process.Identity, os.Signal) error {
			signalMu.Lock()
			signals++
			status = process.StatusAbsent
			signalMu.Unlock()
			return nil
		},
		Dial: func(context.Context, string, string) (net.Conn, error) { return nil, errors.New("refused") }, Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, _ := service.Inspect(context.Background())
	var wait sync.WaitGroup
	for range 2 {
		wait.Add(1)
		go func() {
			defer wait.Done()
			_, _ = service.ForceTerminate(context.Background(), inspection.ConfirmationToken)
		}()
	}
	wait.Wait()
	if signals != 1 {
		t.Fatalf("signals = %d", signals)
	}
}

func TestServiceChangedIdentityConsumesTokenWithoutSignal(t *testing.T) {
	identity := testIdentity()
	live := identity
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Identity, error) { return live, nil },
		CurrentUser: func() (string, error) { return "user", nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Signal:      func(process.Identity, os.Signal) error { signals++; return nil }, Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, err := service.Inspect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	live.SocketID = "replacement-socket"
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrChanged) || signals != 0 {
		t.Fatalf("error=%v signals=%d", err, signals)
	}
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrConfirmationExpired) || signals != 0 {
		t.Fatalf("replay error=%v signals=%d", err, signals)
	}
}

func TestServiceNeverSignalsReplacementDuringReleaseWait(t *testing.T) {
	identity := testIdentity()
	live := identity
	status := process.StatusGenuine
	signals := 0
	service := New(Config{ListenAddr: identity.ListenAddr}, Dependencies{
		Discover: func(context.Context) discovery.Result {
			return discovery.Result{Classification: discovery.UnknownOccupant}
		},
		Inspect:     func(context.Context, string) (Identity, error) { return live, nil },
		CurrentUser: func() (string, error) { return "user", nil },
		SameProcess: func(left, right process.Identity) (bool, error) { return left == right, nil },
		Validate:    func(process.Identity, string) (process.Status, error) { return status, nil },
		Signal: func(process.Identity, os.Signal) error {
			signals++
			status = process.StatusAbsent
			live.SocketID = "replacement"
			live.Process.PID++
			return nil
		},
		Dial:   func(context.Context, string, string) (net.Conn, error) { return &stubConn{}, nil },
		Random: bytes.NewReader(make([]byte, 32)),
	})
	inspection, _ := service.Inspect(context.Background())
	if _, err := service.ForceTerminate(context.Background(), inspection.ConfirmationToken); !errors.Is(err, ErrChanged) || signals != 1 {
		t.Fatalf("error=%v signals=%d", err, signals)
	}
}

type stubConn struct{ net.Conn }

func (stubConn) Close() error { return nil }

func testIdentity() Identity {
	return Identity{ListenAddr: "127.0.0.1:19099", Network: "tcp4", SocketID: "socket", Process: process.Identity{PID: 42, StartedAt: "start", Executable: "/tmp/listener"}, UserID: "user"}
}
