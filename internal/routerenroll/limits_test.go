//go:build unix

package routerenroll

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestControlConcurrencyDoesNotQueueUnboundedOrBlockReady(t *testing.T) {
	s, _, _ := nativeFixture(t)
	s.slots = make(chan struct{}, 2)
	entered := make(chan struct{}, 2)
	release := make(chan struct{})
	var once sync.Once
	defer once.Do(func() { close(release) })
	handler := s.boundedControl(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/enroll/ready" {
			w.WriteHeader(204)
			return
		}
		entered <- struct{}{}
		<-release
		w.WriteHeader(204)
	}))
	var group sync.WaitGroup
	for range 2 {
		group.Go(func() {
			handler.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest("POST", "/enroll/native/poll", nil))
		})
	}
	for range 2 {
		select {
		case <-entered:
		case <-time.After(time.Second):
			t.Fatal("control request did not enter")
		}
	}
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, httptest.NewRequest("POST", "/enroll/native/poll", nil))
	if w.Code != 429 {
		t.Fatal("overload was queued", w.Code)
	}
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, httptest.NewRequest("GET", "/enroll/ready", nil))
	if w.Code != 204 {
		t.Fatal("readiness blocked by provider work")
	}
	once.Do(func() { close(release) })
	group.Wait()
}

func TestNativeCreationRateBeforeCSRWorkAndRefill(t *testing.T) {
	s, request, _ := nativeFixture(t)
	data, _ := json.Marshal(request)
	handler := s.Handler()
	for i := 0; i < StartBurst+1; i++ {
		w := httptest.NewRecorder()
		r := httptest.NewRequest("POST", "/enroll/native/start", bytes.NewReader(data))
		r.Header.Set("Content-Type", "application/json")
		handler.ServeHTTP(w, r)
		want := 201
		if i == StartBurst {
			want = 429
		}
		if w.Code != want {
			t.Fatalf("start %d: %d", i, w.Code)
		}
	}
	now := s.now()
	s.now = func() time.Time { return now.Add(time.Second) }
	w := httptest.NewRecorder()
	r := httptest.NewRequest("POST", "/enroll/native/start", bytes.NewReader(data))
	r.Header.Set("Content-Type", "application/json")
	handler.ServeHTTP(w, r)
	if w.Code != 201 {
		t.Fatal("bounded rate did not refill", w.Code)
	}
}

func TestDuplicateAndTrailingNativeFieldsAreRejected(t *testing.T) {
	for _, body := range []string{`{"transaction":"first","transaction":"second"}`, `{"verifier":"secret-canary","verifier":"other"}`, `{} {}`, `[]`, `null`} {
		w := httptest.NewRecorder()
		r := httptest.NewRequest("POST", "/enroll/native/poll", strings.NewReader(body))
		r.Header.Set("Content-Type", "application/json")
		var proof NativeProof
		if decodeJSON(w, r, &proof) || w.Code != 400 || strings.Contains(w.Body.String(), "secret-canary") {
			t.Fatal("ambiguous/unsafe decode", body)
		}
	}
}
