package lifecycle

import "sync"

type boundedOutput struct {
	mu    sync.Mutex
	limit int
	data  []byte
}

func newBoundedOutput(limit int) *boundedOutput {
	if limit <= 0 {
		limit = 64 * 1024
	}
	return &boundedOutput{limit: limit}
}

func (b *boundedOutput) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.data = append(b.data, p...)
	if len(b.data) > b.limit {
		b.data = append([]byte(nil), b.data[len(b.data)-b.limit:]...)
	}
	return len(p), nil
}

func (b *boundedOutput) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return string(append([]byte(nil), b.data...))
}
