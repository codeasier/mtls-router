package background

import (
	"io"
	"os"
	"sync"
)

const DefaultMaxLogBytes int64 = 4 * 1024 * 1024

func OpenLogFile(logPath string) (*os.File, error) {
	return os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
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
