package background

import (
	"path/filepath"
	"strings"
)

func DefaultLogPath(exePath string) string {
	return filepath.Join(filepath.Dir(exePath), "mtls-router.log")
}

func ChildArgs(args []string, logPath string) []string {
	child := make([]string, 0, len(args)+2)
	hasLog := false
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "-backend" || arg == "--backend" || strings.HasPrefix(arg, "-backend=") || strings.HasPrefix(arg, "--backend="):
			continue
		case arg == "-log" || arg == "--log":
			hasLog = true
			child = append(child, arg)
			if i+1 < len(args) {
				i++
				child = append(child, args[i])
			}
		case strings.HasPrefix(arg, "-log=") || strings.HasPrefix(arg, "--log="):
			hasLog = true
			child = append(child, arg)
		default:
			child = append(child, arg)
		}
	}
	if !hasLog {
		child = append(child, "-log", logPath)
	}
	return child
}
