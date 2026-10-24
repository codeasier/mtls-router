//go:build unix

package routerenroll

import (
	"bufio"
	"context"
	"crypto/tls"
	"net/http"
	"net/http/httptrace"
	"os"
	"os/exec"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"
)

// Explicit opt-in; this probes only the isolated test gateway, never production.
func benchmarkIsolatedGateway(t *testing.T, client *http.Client, pid int, reload func() error) {
	t.Helper()
	raw := os.Getenv("MTLS_TEST_GATEWAY_SAMPLES")
	if raw == "" {
		return
	}
	streaming := *client
	streaming.Timeout = 0
	client = &streaming
	count, err := strconv.Atoi(raw)
	if err != nil || count < 10 || count > 1000 {
		t.Fatal("MTLS_TEST_GATEWAY_SAMPLES must be 10..1000")
	}
	holds := make([]context.CancelFunc, 0, 4)
	defer func() {
		for _, cancel := range holds {
			cancel()
		}
	}()
	measure := func(label string, withReload bool) {
		t.Helper()
		var lock sync.Mutex
		var handshake, firstToken []float64
		var queued []float64
		failures := 0
		start := time.Now()
		var workers sync.WaitGroup
		queue := make(chan struct{}, 4)
		var reloadDone chan error
		if withReload {
			reloadDone = make(chan error, 1)
			go func() { time.Sleep(5 * time.Millisecond); reloadDone <- reload() }()
		}
		for i := 0; i < count; i++ {
			entered := time.Now()
			queue <- struct{}{}
			workers.Go(func() {
				queueMS := float64(time.Since(entered).Microseconds()) / 1000
				defer func() { <-queue }()
				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				defer cancel()
				var tlsStart time.Time
				tlsMS := 0.0
				trace := &httptrace.ClientTrace{TLSHandshakeStart: func() { tlsStart = time.Now() }, TLSHandshakeDone: func(_ tls.ConnectionState, _ error) { tlsMS = float64(time.Since(tlsStart).Microseconds()) / 1000 }}
				request, _ := http.NewRequestWithContext(httptrace.WithClientTrace(ctx, trace), "GET", RelayOrigin+"/v1/bench-stream", nil)
				request.Header.Set("Authorization", "Bearer business-canary")
				began := time.Now()
				response, err := client.Do(request)
				ok := false
				if err == nil {
					_, readErr := bufio.NewReader(response.Body).ReadString('\n')
					ok = response.StatusCode == 200 && readErr == nil
					_ = response.Body.Close()
				}
				elapsed := float64(time.Since(began).Microseconds()) / 1000
				lock.Lock()
				if ok {
					handshake = append(handshake, tlsMS)
					firstToken = append(firstToken, elapsed)
					queued = append(queued, queueMS)
				} else {
					failures++
				}
				lock.Unlock()
			})
		}
		workers.Wait()
		if reloadDone != nil {
			if err := <-reloadDone; err != nil {
				t.Fatal(err)
			}
		}
		sort.Float64s(handshake)
		sort.Float64s(firstToken)
		sort.Float64s(queued)
		percentile := func(values []float64, p float64) float64 {
			if len(values) == 0 {
				return -1
			}
			return values[int(float64(len(values)-1)*p)]
		}
		stats, _ := exec.Command("ps", "-axo", "pid=,ppid=,rss=,pcpu=").Output()
		rss := int64(0)
		cpu := float64(0)
		for _, line := range strings.Split(string(stats), "\n") {
			fields := strings.Fields(line)
			if len(fields) != 4 {
				continue
			}
			process, _ := strconv.Atoi(fields[0])
			parent, _ := strconv.Atoi(fields[1])
			if process == pid || parent == pid {
				r, _ := strconv.ParseInt(fields[2], 10, 64)
				c, _ := strconv.ParseFloat(fields[3], 64)
				rss += r
				cpu += c
			}
		}
		var memory runtime.MemStats
		runtime.ReadMemStats(&memory)
		t.Logf("isolated_gateway phase=%s samples=%d failed=%d qps=%.2f handshake_p95_ms=%.3f handshake_p99_ms=%.3f first_event_p95_ms=%.3f first_event_p99_ms=%.3f driver_queue_p95_ms=%.3f nginx_total_rss_kib=%d nginx_cpu_snapshot_pct=%.2f driver_heap_bytes=%d", label, count, failures, float64(count)/time.Since(start).Seconds(), percentile(handshake, .95), percentile(handshake, .99), percentile(firstToken, .95), percentile(firstToken, .99), percentile(queued, .95), rss, cpu, memory.HeapAlloc)
		if failures > 0 {
			t.Fatal("isolated gateway load had failed requests")
		}
	}
	measure("baseline", false)
	for range 4 {
		ctx, cancel := context.WithCancel(context.Background())
		holds = append(holds, cancel)
		request, _ := http.NewRequestWithContext(ctx, "GET", RelayOrigin+"/v1/bench-stream", nil)
		request.Header.Set("Authorization", "Bearer business-canary")
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := bufio.NewReader(response.Body).ReadString('\n'); err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
	}
	measure("resident_sse", false)
	measure("resident_sse_and_reload", true)
}
