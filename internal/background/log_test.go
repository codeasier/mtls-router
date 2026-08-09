package background

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestPrepareSessionLogPathGroupsByDateAndStartTime(t *testing.T) {
	dir := t.TempDir()
	basePath := filepath.Join(dir, "mtls-router.log")
	startedAt := time.Date(2026, 8, 9, 14, 5, 7, 0, time.Local)

	first, err := PrepareSessionLogPath(basePath, startedAt)
	if err != nil {
		t.Fatal(err)
	}
	wantFirst := filepath.Join(dir, "mtls-router-logs", "2026-08-09", "14-05-07.log")
	if first != wantFirst {
		t.Fatalf("first path = %q, want %q", first, wantFirst)
	}
	if err := os.WriteFile(first, []byte("first launch"), 0o600); err != nil {
		t.Fatal(err)
	}

	second, err := PrepareSessionLogPath(basePath, startedAt)
	if err != nil {
		t.Fatal(err)
	}
	wantSecond := filepath.Join(dir, "mtls-router-logs", "2026-08-09", "14-05-07-2.log")
	if second != wantSecond {
		t.Fatalf("second path = %q, want %q", second, wantSecond)
	}

	later, err := PrepareSessionLogPath(basePath, startedAt.Add(time.Second))
	if err != nil {
		t.Fatal(err)
	}
	if err := RecordLatestSessionLogPath(basePath, later); err != nil {
		t.Fatal(err)
	}
	latest, err := LatestSessionLogPath(basePath)
	if err != nil {
		t.Fatal(err)
	}
	if latest != later {
		t.Fatalf("latest path = %q, want %q", latest, later)
	}

	clockRollback, err := PrepareSessionLogPath(basePath, startedAt.Add(-time.Hour))
	if err != nil {
		t.Fatal(err)
	}
	if err := RecordLatestSessionLogPath(basePath, clockRollback); err != nil {
		t.Fatal(err)
	}
	latest, err = LatestSessionLogPath(basePath)
	if err != nil {
		t.Fatal(err)
	}
	if latest != clockRollback {
		t.Fatalf("latest path after clock rollback = %q, want %q", latest, clockRollback)
	}
}

func TestBoundedLogWriterLimitsExistingAndNewOutput(t *testing.T) {
	path := filepath.Join(t.TempDir(), "router.log")
	if err := os.WriteFile(path, []byte("existing output over limit"), 0o600); err != nil {
		t.Fatal(err)
	}
	writer, closeLog, err := OpenBoundedLogWriter(path, 8)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := writer.Write([]byte("0123456789")); err != nil {
		t.Fatal(err)
	}
	closeLog()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "23456789" {
		t.Fatalf("bounded log = %q", data)
	}
}
