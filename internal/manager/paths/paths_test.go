package paths

import (
	"path/filepath"
	"testing"
)

func TestResolvePlatformDefaults(t *testing.T) {
	tests := []struct {
		goos    string
		env     map[string]string
		desktop string
	}{
		{"linux", nil, filepath.Join("home", ".local", "share", desktopAppID)},
		{"linux", map[string]string{"XDG_DATA_HOME": "data"}, filepath.Join("data", desktopAppID)},
		{"darwin", nil, filepath.Join("home", "Library", "Application Support", desktopAppID)},
		{"windows", map[string]string{"APPDATA": "roaming"}, filepath.Join("roaming", desktopAppID)},
	}
	for _, tt := range tests {
		t.Run(tt.goos, func(t *testing.T) {
			got := resolve(tt.goos, "home", func(key string) string { return tt.env[key] })
			if got.CLIStateFile != filepath.Join("home", ".mtls-router", "setup-state.json") {
				t.Fatalf("CLIStateFile = %q", got.CLIStateFile)
			}
			if got.DesktopDataDir != tt.desktop {
				t.Fatalf("DesktopDataDir = %q, want %q", got.DesktopDataDir, tt.desktop)
			}
			if filepath.Dir(got.DesktopStateFile) != got.DesktopDataDir || filepath.Dir(got.DesktopLockFile) != got.DesktopDataDir {
				t.Fatal("desktop files are not in the desktop data directory")
			}
		})
	}
}

func TestResolveOverrides(t *testing.T) {
	env := map[string]string{
		"MTLS_ROUTER_STATE_DIR":        "cli-state",
		"MTLS_ROUTER_LOG_PATH":         "cli.log",
		"MTLS_ROUTER_DESKTOP_DATA_DIR": "desktop-data",
	}
	got := resolve("linux", "home", func(key string) string { return env[key] })
	if got.CLIStateDir != "cli-state" || got.CLILogFile != "cli.log" || got.DesktopDataDir != "desktop-data" {
		t.Fatalf("overrides not preserved: %+v", got)
	}
}
