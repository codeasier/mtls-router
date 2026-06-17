package proxy

import (
	"bytes"
	"context"
	"errors"
	"io"
	"strings"
	"sync/atomic"
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

var errInjectedRead = errors.New("injected read error")

type errorAfterPrefixBody struct {
	prefix strings.Reader
}

func (b *errorAfterPrefixBody) Read(p []byte) (int, error) {
	if b.prefix.Len() > 0 {
		n, _ := b.prefix.Read(p)
		return n, errInjectedRead
	}
	return 0, errInjectedRead
}

func (b *errorAfterPrefixBody) Close() error { return nil }

type countingReadCloser struct {
	io.Reader
	closed atomic.Int32
}

func (c *countingReadCloser) Close() error {
	c.closed.Add(1)
	return nil
}

type closeCountingBody struct {
	io.ReadCloser
}

func (b *closeCountingBody) Set(rc io.ReadCloser) { b.ReadCloser = rc }

func TestSniffStreamRestoredSetterCloseDoesNotRecurse(t *testing.T) {
	original := &countingReadCloser{Reader: strings.NewReader(`{"stream":true}`)}
	body := &closeCountingBody{ReadCloser: original}

	_, ok := SniffStream(context.Background(), body)
	if !ok {
		t.Fatal("expected stream request")
	}
	if err := body.ReadCloser.Close(); err != nil {
		t.Fatal(err)
	}
	if original.closed.Load() != 1 {
		t.Fatalf("original Close called %d times, want 1", original.closed.Load())
	}
}

func TestSniffStreamRestoredEmbeddedSetterCloseDoesNotRecurse(t *testing.T) {
	body := &mutableBody{ReadCloser: io.NopCloser(strings.NewReader(`{"stream":true}`))}

	_, ok := SniffStream(context.Background(), body)
	if !ok {
		t.Fatal("expected stream request")
	}
	if err := body.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestSniffStreamPreservesPrefixOnReadError(t *testing.T) {
	body := &mutableBody{ReadCloser: &errorAfterPrefixBody{prefix: *strings.NewReader(`{"stream":`)}}

	_, ok := SniffStream(context.Background(), body)
	if ok {
		t.Fatal("unexpected stream request")
	}
	got, _ := io.ReadAll(body)
	if string(got) != `{"stream":` {
		t.Fatalf("body prefix lost after read error: %q", got)
	}
}

func TestSniffStreamDoesNotReadPastWindow(t *testing.T) {
	body := newBlockingBody(strings.Repeat("x", sniffLimit))

	_, _ = SniffStream(context.Background(), body)

	if body.blocked.Load() {
		t.Fatal("SniffStream read beyond the sniff window")
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
