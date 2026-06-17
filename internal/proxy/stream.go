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
	io.Reader
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
	buf := make([]byte, sniffLimit)
	n, err := io.ReadFull(body, buf)
	if err != nil && err != io.ErrUnexpectedEOF && err != io.EOF {
		restoreBody(body, buf[:n])
		return ctx, false
	}
	data := buf[:n]
	restoreBody(body, data)
	marker := containsStreamTrue(data)
	if marker {
		ctx = context.WithValue(ctx, streamKey{}, true)
	}
	return ctx, marker
}

func IsStreamRequest(ctx context.Context) bool {
	v, _ := ctx.Value(streamKey{}).(bool)
	return v
}

func contextWithStream(ctx context.Context) context.Context {
	return context.WithValue(ctx, streamKey{}, true)
}

func restoreBody(body io.ReadCloser, data []byte) {
	if setter, ok := body.(interface{ Set(io.ReadCloser) }); ok {
		rest := body
		v := reflect.ValueOf(body)
		if v.Kind() == reflect.Pointer {
			field := v.Elem().FieldByName("ReadCloser")
			if field.IsValid() && field.CanInterface() {
				if rc, ok := field.Interface().(io.ReadCloser); ok {
					rest = rc
				}
			}
		}
		setter.Set(replayReadCloser{Reader: io.MultiReader(bytes.NewReader(data), rest), close: rest.Close})
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
	original := reflect.NewAt(field.Type, fieldPtr).Elem().Interface().(io.Reader)
	reflect.NewAt(field.Type, fieldPtr).Elem().Set(reflect.ValueOf(io.MultiReader(bytes.NewReader(data), original)))
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
