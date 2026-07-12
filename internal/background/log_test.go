package background

import (
	"os"
	"path/filepath"
	"testing"
)

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
