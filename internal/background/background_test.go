package background

import (
	"os"
	"path/filepath"
	"testing"
)

func TestOpenLogFileCreatesAppendOnlyLogFile(t *testing.T) {
	logPath := filepath.Join(t.TempDir(), "mtls-router.log")

	f, err := openLogFile(logPath)
	if err != nil {
		t.Fatalf("openLogFile() error = %v", err)
	}
	if _, err := f.WriteString("first line\n"); err != nil {
		_ = f.Close()
		t.Fatalf("first WriteString() error = %v", err)
	}
	if err := f.Close(); err != nil {
		t.Fatalf("first Close() error = %v", err)
	}

	f, err = openLogFile(logPath)
	if err != nil {
		t.Fatalf("second openLogFile() error = %v", err)
	}
	if _, err := f.WriteString("second line\n"); err != nil {
		_ = f.Close()
		t.Fatalf("second WriteString() error = %v", err)
	}
	if err := f.Close(); err != nil {
		t.Fatalf("second Close() error = %v", err)
	}

	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}

	want := "first line\nsecond line\n"
	if string(data) != want {
		t.Fatalf("log contents = %q, want %q", string(data), want)
	}
}
