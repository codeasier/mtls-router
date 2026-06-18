package background

import "os"

func openLogFile(logPath string) (*os.File, error) {
	return os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
}
