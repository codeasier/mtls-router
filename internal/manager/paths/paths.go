// Package paths resolves manager-owned per-user files without depending on a
// desktop runtime.
package paths

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
)

const desktopAppID = "com.codeasier.mtls-router"

// Paths contains the CLI-compatible and desktop-specific manager paths.
type Paths struct {
	CLIStateDir      string
	CLIStateFile     string
	CLILogFile       string
	DesktopDataDir   string
	DesktopStateFile string
	DesktopLogFile   string
	DesktopLockFile  string
}

// Resolve returns paths for the current user and operating system.
func Resolve() (Paths, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return Paths{}, fmt.Errorf("resolve user home: %w", err)
	}
	return resolve(runtime.GOOS, home, os.Getenv), nil
}

func resolve(goos, home string, getenv func(string) string) Paths {
	cliDir := getenv("MTLS_ROUTER_STATE_DIR")
	if cliDir == "" {
		cliDir = filepath.Join(home, ".mtls-router")
	}
	cliLog := getenv("MTLS_ROUTER_LOG_PATH")
	if cliLog == "" {
		cliLog = filepath.Join(cliDir, "mtls-router.log")
	}

	desktopDir := getenv("MTLS_ROUTER_DESKTOP_DATA_DIR")
	if desktopDir == "" {
		switch goos {
		case "windows":
			base := getenv("APPDATA")
			if base == "" {
				base = filepath.Join(home, "AppData", "Roaming")
			}
			desktopDir = filepath.Join(base, desktopAppID)
		case "darwin":
			desktopDir = filepath.Join(home, "Library", "Application Support", desktopAppID)
		default:
			base := getenv("XDG_DATA_HOME")
			if base == "" {
				base = filepath.Join(home, ".local", "share")
			}
			desktopDir = filepath.Join(base, desktopAppID)
		}
	}

	return Paths{
		CLIStateDir:      cliDir,
		CLIStateFile:     filepath.Join(cliDir, "setup-state.json"),
		CLILogFile:       cliLog,
		DesktopDataDir:   desktopDir,
		DesktopStateFile: filepath.Join(desktopDir, "desktop-state.json"),
		DesktopLogFile:   filepath.Join(desktopDir, "mtls-router.log"),
		DesktopLockFile:  filepath.Join(desktopDir, "desktop-owner.lock"),
	}
}
