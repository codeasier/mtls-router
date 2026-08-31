package apikeyusage

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

const usageKeyCanary = "usage-key-canary-4419"

func TestClientFetchesExactUsageEndpoint(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/usage" || r.URL.RawQuery != "period=7d" || r.URL.Fragment != "" {
			t.Errorf("request = %s", r.URL.RequestURI())
		}
		if r.Header.Get("Authorization") != "Bearer "+usageKeyCanary {
			t.Errorf("Authorization = %q", r.Header.Get("Authorization"))
		}
		_, _ = io.WriteString(w, `{"period":"7d","summary":{"requests":3,"prompt_tokens":9,"completion_tokens":1,"cost":0.25},"by_model":[]}`)
	}))
	defer server.Close()

	snapshot, err := New(nil).Fetch(context.Background(), Request{
		URL: server.URL + "/v1/usage", Period: Period7d, APIKey: usageKeyCanary,
	})
	if err != nil {
		t.Fatalf("Fetch() = %v", err)
	}
	if snapshot.Summary.Requests != 3 || snapshot.Summary.Cost != 0.25 {
		t.Fatalf("snapshot = %+v", snapshot)
	}
}

func TestClientMapsStatusAndRejectsUnsafeURLs(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Query().Get("period") {
		case "today":
			w.WriteHeader(http.StatusNotFound)
		case "30d":
			w.WriteHeader(http.StatusUnauthorized)
		default:
			w.WriteHeader(http.StatusBadGateway)
		}
	}))
	defer server.Close()

	for _, test := range []struct {
		period Period
		code   protocol.ErrorCode
	}{
		{period: PeriodToday, code: protocol.CodeUsageUnavailable},
		{period: Period30d, code: protocol.CodeUsageAuthFailed},
		{period: Period7d, code: protocol.CodeUsageRequestFailed},
	} {
		_, err := New(nil).Fetch(context.Background(), Request{
			URL: server.URL + "/v1/usage", Period: test.period, APIKey: usageKeyCanary,
		})
		if CodeOf(err) != test.code {
			t.Fatalf("period %s code = %v, want %s", test.period, err, test.code)
		}
		if err != nil && strings.Contains(err.Error(), usageKeyCanary) {
			t.Fatalf("error leaked key: %v", err)
		}
	}

	_, err := New(nil).Fetch(context.Background(), Request{
		URL: server.URL + "/v1/usage?period=7d", Period: Period7d, APIKey: usageKeyCanary,
	})
	if CodeOf(err) != protocol.CodeUsageRequestFailed {
		t.Fatalf("pre-set query = %v", err)
	}
}

func TestClientRejectsOversizedBody(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = io.WriteString(w, `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"pad":"`)
		_, _ = io.WriteString(w, strings.Repeat("x", maxBodyBytes))
		_, _ = io.WriteString(w, `"}`)
	}))
	defer server.Close()

	_, err := New(nil).Fetch(context.Background(), Request{
		URL: server.URL + "/v1/usage", Period: Period7d, APIKey: usageKeyCanary,
	})
	if CodeOf(err) != protocol.CodeUsageResponseInvalid {
		t.Fatalf("oversized body = %v", err)
	}
}
