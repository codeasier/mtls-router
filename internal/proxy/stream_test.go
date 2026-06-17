package proxy

import (
	"bytes"
	"context"
	"io"
	"strings"
	"testing"
)

type mutableBody struct{ io.ReadCloser }

func (m *mutableBody) Set(rc io.ReadCloser) { m.ReadCloser = rc }

func TestSniffStreamDetectsAndPreservesBody(t *testing.T) {
	body := &mutableBody{ReadCloser: io.NopCloser(strings.NewReader(`{"messages":[],"stream":true}`))}
	ctx, ok := SniffStream(context.Background(), body)
	if !ok || !IsStreamRequest(ctx) {
		t.Fatal("expected stream request")
	}
	b, _ := io.ReadAll(body)
	if string(b) != `{"messages":[],"stream":true}` {
		t.Fatalf("body corrupted: %q", b)
	}
}

func TestSniffStreamPreservesStandardRequestBody(t *testing.T) {
	body := io.NopCloser(strings.NewReader(`{"stream":true}`))
	_, ok := SniffStream(context.Background(), body)
	if !ok {
		t.Fatal("expected stream request")
	}
	b, err := io.ReadAll(body)
	if err != nil {
		t.Fatal(err)
	}
	if string(b) != `{"stream":true}` {
		t.Fatalf("body corrupted: %q", b)
	}
}

func TestSniffStreamDetectsMarkerInsideFirst4KBOfLargeBody(t *testing.T) {
	payload := bytes.Repeat([]byte(" "), 4000)
	payload = append(payload, []byte(`,"stream":true`)...)
	body := &mutableBody{ReadCloser: io.NopCloser(bytes.NewReader(payload))}
	ctx, ok := SniffStream(context.Background(), body)
	if !ok || !IsStreamRequest(ctx) {
		t.Fatal("expected stream request")
	}
	b, _ := io.ReadAll(body)
	if !bytes.Equal(b, payload) {
		t.Fatal("body not preserved")
	}
}

func TestSniffStreamFalseCases(t *testing.T) {
	cases := []string{"", `{"stream":false}`, `{bad`, strings.Repeat("x", 4096) + `{"stream":true}`}
	for _, tc := range cases {
		body := &mutableBody{ReadCloser: io.NopCloser(strings.NewReader(tc))}
		ctx, ok := SniffStream(context.Background(), body)
		if ok || IsStreamRequest(ctx) {
			t.Fatalf("unexpected stream for %q", tc)
		}
		b, _ := io.ReadAll(body)
		if string(b) != tc {
			t.Fatal("body not preserved")
		}
	}
}
