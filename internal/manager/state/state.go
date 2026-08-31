// Package state provides durable manager state and desktop ownership locking.
package state

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

var ErrCorrupt = errors.New("manager state is corrupt")

// RouterState retains the setup scripts' field names and adds desktop manager
// ownership fields without requiring a legacy migration.
type RouterState struct {
	PID                       int    `json:"pid"`
	ListenAddr                string `json:"listen_addr,omitempty"`
	BinaryPath                string `json:"binary_path,omitempty"`
	LogPath                   string `json:"log_path,omitempty"`
	StartedAt                 string `json:"started_at,omitempty"`
	ProcessStartedAt          string `json:"process_started_at,omitempty"`
	ProcessExecutable         string `json:"process_executable,omitempty"`
	Owner                     string `json:"owner,omitempty"`
	DesktopSessionID          string `json:"desktop_session_id,omitempty"`
	InstallationID            string `json:"installation_id,omitempty"`
	PackageGeneration         int    `json:"package_generation,omitempty"`
	ManagerPID                int    `json:"manager_pid,omitempty"`
	ManagerProcessStartedAt   string `json:"manager_process_started_at,omitempty"`
	ManagerProcessExecutable  string `json:"manager_process_executable,omitempty"`
	ManagerVersion            string `json:"manager_version,omitempty"`
	RouterVersion             string `json:"router_version,omitempty"`
	DeploymentID              string `json:"deployment_id,omitempty"`
	ManagementProtocolVersion string `json:"management_protocol_version,omitempty"`
}

// Read decodes a router state file. Missing and permission errors remain
// discoverable with errors.Is; malformed or trailing JSON wraps ErrCorrupt.
func Read(path string) (RouterState, error) {
	var value RouterState
	err := ReadJSON(path, &value)
	return value, err
}

// Write atomically persists router state beside its destination.
func Write(path string, value RouterState) error {
	return WriteJSON(path, value)
}

// ReadJSON reads exactly one JSON value from path.
func ReadJSON(path string, dst any) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()

	reader := bufio.NewReader(f)
	if prefix, _ := reader.Peek(3); len(prefix) == 3 && prefix[0] == 0xef && prefix[1] == 0xbb && prefix[2] == 0xbf {
		_, _ = reader.Discard(3)
	}
	decoder := json.NewDecoder(reader)
	if err := decoder.Decode(dst); err != nil {
		return fmt.Errorf("%w: %v", ErrCorrupt, err)
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		if err == nil {
			err = errors.New("multiple JSON values")
		}
		return fmt.Errorf("%w: %v", ErrCorrupt, err)
	}
	return nil
}

// WriteJSON uses a same-directory temporary file, sync, and atomic replacement.
func WriteJSON(path string, value any) (err error) {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("create state directory: %w", err)
	}
	if err := restrictPath(dir, true); err != nil {
		return fmt.Errorf("restrict state directory: %w", err)
	}

	tmp, err := os.CreateTemp(dir, "."+filepath.Base(path)+"-*")
	if err != nil {
		return fmt.Errorf("create state temporary file: %w", err)
	}
	tmpPath := tmp.Name()
	defer func() {
		tmp.Close()
		os.Remove(tmpPath)
	}()

	if err := restrictPath(tmpPath, false); err != nil {
		return fmt.Errorf("restrict state temporary file: %w", err)
	}
	encoder := json.NewEncoder(tmp)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(value); err != nil {
		return fmt.Errorf("encode state: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		return fmt.Errorf("sync state temporary file: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close state temporary file: %w", err)
	}
	if err := replaceFile(tmpPath, path); err != nil {
		return fmt.Errorf("replace state file: %w", err)
	}
	if err := restrictPath(path, false); err != nil {
		return fmt.Errorf("restrict state file: %w", err)
	}
	return syncDir(dir)
}
