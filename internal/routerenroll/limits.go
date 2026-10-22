package routerenroll

import (
	"net/http"
	"sync"
	"time"
)

const MaxConcurrentControlRequests = 32
const StartBurst = 10

type startBudget struct {
	mu     sync.Mutex
	tokens float64
	last   time.Time
}

func (b *startBudget) take(now time.Time) bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.last.IsZero() {
		b.last = now
		b.tokens = StartBurst
	}
	if elapsed := now.Sub(b.last).Seconds(); elapsed > 0 {
		b.tokens += elapsed
		if b.tokens > StartBurst {
			b.tokens = StartBurst
		}
		b.last = now
	}
	if b.tokens < 1 {
		return false
	}
	b.tokens--
	return true
}
func (s *Service) boundedControl(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Minimal readiness does not consume a provider-work slot.
		if r.URL.Path == "/enroll/ready" && r.Method == "GET" {
			next.ServeHTTP(w, r)
			return
		}
		select {
		case s.slots <- struct{}{}:
			defer func() { <-s.slots }()
		default:
			WriteError(w, CapacityReached)
			return
		}
		if r.URL.Path == "/enroll/native/start" && r.Method == "POST" && !s.starts.take(s.now()) {
			w.Header().Set("Retry-After", "1")
			WriteError(w, CapacityReached)
			return
		}
		next.ServeHTTP(w, r)
	})
}
