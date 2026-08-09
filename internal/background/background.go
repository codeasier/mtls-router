package background

import (
	"errors"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

const DefaultMaxLogBytes int64 = 4 * 1024 * 1024

func OpenLogFile(logPath string) (*os.File, error) {
	return os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
}

// PrepareSessionLogPath groups each launch by local date and start time.
func PrepareSessionLogPath(basePath string, startedAt time.Time) (string, error) {
	directory, extension := sessionLogParts(basePath)
	dayDir := filepath.Join(directory, startedAt.Format("2006-01-02"))
	if err := os.MkdirAll(dayDir, 0o700); err != nil {
		return "", err
	}
	if err := os.Chmod(dayDir, 0o700); err != nil {
		return "", err
	}

	startName := startedAt.Format("15-04-05")
	for sequence := 1; ; sequence++ {
		fileName := startName + extension
		if sequence > 1 {
			fileName = startName + "-" + strconv.Itoa(sequence) + extension
		}
		path := filepath.Join(dayDir, fileName)
		file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
		if errors.Is(err, os.ErrExist) {
			continue
		}
		if err != nil {
			return "", err
		}
		if err := file.Close(); err != nil {
			return "", err
		}
		return path, nil
	}
}

// LatestSessionLogPath returns the most recently named launch log.
func LatestSessionLogPath(basePath string) (string, error) {
	directory, extension := sessionLogParts(basePath)
	days, err := os.ReadDir(directory)
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", err
	}

	var latestPath string
	var latestStart time.Time
	latestSequence := 0
	for _, day := range days {
		if !day.IsDir() {
			continue
		}
		entries, err := os.ReadDir(filepath.Join(directory, day.Name()))
		if err != nil {
			return "", err
		}
		for _, entry := range entries {
			startedAt, sequence, ok := parseSessionLogName(day.Name(), entry, extension)
			if !ok || startedAt.Before(latestStart) || (startedAt.Equal(latestStart) && sequence <= latestSequence) {
				continue
			}
			latestPath = filepath.Join(directory, day.Name(), entry.Name())
			latestStart = startedAt
			latestSequence = sequence
		}
	}
	return latestPath, nil
}

func sessionLogParts(basePath string) (string, string) {
	extension := filepath.Ext(basePath)
	name := strings.TrimSuffix(filepath.Base(basePath), extension)
	if extension == "" {
		extension = ".log"
	}
	if name == "" || name == "." {
		name = "mtls-router"
	}
	return filepath.Join(filepath.Dir(basePath), name+"-logs"), extension
}

func parseSessionLogName(day string, entry os.DirEntry, extension string) (time.Time, int, bool) {
	if !entry.Type().IsRegular() || filepath.Ext(entry.Name()) != extension {
		return time.Time{}, 0, false
	}
	parts := strings.Split(strings.TrimSuffix(entry.Name(), extension), "-")
	if len(parts) != 3 && len(parts) != 4 {
		return time.Time{}, 0, false
	}
	startedAt, err := time.Parse("2006-01-02 15-04-05", day+" "+strings.Join(parts[:3], "-"))
	if err != nil {
		return time.Time{}, 0, false
	}
	sequence := 1
	if len(parts) == 4 {
		sequence, err = strconv.Atoi(parts[3])
		if err != nil || sequence < 2 {
			return time.Time{}, 0, false
		}
	}
	return startedAt, sequence, true
}

// OpenBoundedLogWriter opens an append-only log that resets before a write
// would exceed maxBytes. This bounds disk use without renaming an open file.
func OpenBoundedLogWriter(logPath string, maxBytes int64) (io.Writer, func(), error) {
	file, err := OpenLogFile(logPath)
	if err != nil {
		return nil, nil, err
	}
	if maxBytes <= 0 {
		maxBytes = DefaultMaxLogBytes
	}
	info, err := file.Stat()
	if err != nil {
		_ = file.Close()
		return nil, nil, err
	}
	size := info.Size()
	if size > maxBytes {
		if err := file.Truncate(0); err != nil {
			_ = file.Close()
			return nil, nil, err
		}
		size = 0
	}
	writer := &boundedLogWriter{file: file, maxBytes: maxBytes, size: size}
	return writer, func() { _ = file.Close() }, nil
}

type boundedLogWriter struct {
	mu       sync.Mutex
	file     *os.File
	maxBytes int64
	size     int64
}

func (w *boundedLogWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	originalLength := len(p)
	if int64(len(p)) >= w.maxBytes {
		p = p[len(p)-int(w.maxBytes):]
		if err := w.file.Truncate(0); err != nil {
			return 0, err
		}
		w.size = 0
	} else if w.size+int64(len(p)) > w.maxBytes {
		if err := w.file.Truncate(0); err != nil {
			return 0, err
		}
		w.size = 0
	}
	written, err := w.file.Write(p)
	w.size += int64(written)
	if err != nil {
		return written, err
	}
	return originalLength, nil
}
