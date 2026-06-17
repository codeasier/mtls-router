package proxy

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"reflect"
	"unsafe"
)

const sniffLimit = 4096

type streamKey struct{}

type replayReadCloser struct {
	*bytes.Reader
	close func() error
}

func (r replayReadCloser) Close() error {
	if r.close != nil {
		return r.close()
	}
	return nil
}

func SniffStream(ctx context.Context, body io.ReadCloser) (context.Context, bool) {
	if body == nil {
		return ctx, false
	}
	data, _ := io.ReadAll(body)
	_ = body.Close()
	restoreBody(body, data)
	marker := containsStreamTrue(data[:min(len(data), sniffLimit)])
	if marker {
		ctx = context.WithValue(ctx, streamKey{}, true)
	}
	return ctx, marker
}

func IsStreamRequest(ctx context.Context) bool {
	v, _ := ctx.Value(streamKey{}).(bool)
	return v
}

func restoreBody(body io.ReadCloser, data []byte) {
	replay := replayReadCloser{Reader: bytes.NewReader(data)}
	if setter, ok := body.(interface{ Set(io.ReadCloser) }); ok {
		setter.Set(replay)
		return
	}
	v := reflect.ValueOf(body)
	if v.Kind() != reflect.Struct {
		return
	}
	field, ok := v.Type().FieldByName("Reader")
	if !ok || !field.Type.AssignableTo(reflect.TypeOf((*io.Reader)(nil)).Elem()) {
		return
	}
	type iface struct {
		typ  unsafe.Pointer
		data unsafe.Pointer
	}
	ptr := (*iface)(unsafe.Pointer(&body)).data
	fieldPtr := unsafe.Pointer(uintptr(ptr) + field.Offset)
	reflect.NewAt(field.Type, fieldPtr).Elem().Set(reflect.ValueOf(bytes.NewReader(data)))
}

func containsStreamTrue(data []byte) bool {
	if bytes.Contains(data, []byte(`"stream":true`)) || bytes.Contains(data, []byte(`"stream": true`)) {
		return true
	}
	var obj map[string]any
	if err := json.Unmarshal(data, &obj); err != nil {
		return false
	}
	v, _ := obj["stream"].(bool)
	return v
}
