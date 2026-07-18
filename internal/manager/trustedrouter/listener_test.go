package trustedrouter

import "testing"

func TestNormalizeListenerStrictNumericLoopback(t *testing.T) {
	tests := []struct {
		input, authority, router, api string
		valid                         bool
	}{
		{input: "127.0.0.1:19099", authority: "127.0.0.1:19099", router: "http://127.0.0.1:19099", api: "http://127.0.0.1:19099/v1", valid: true},
		{input: "127.42.1.9:1", authority: "127.42.1.9:1", router: "http://127.42.1.9:1", api: "http://127.42.1.9:1/v1", valid: true},
		{input: "[::1]:65535", authority: "[::1]:65535", router: "http://[::1]:65535", api: "http://[::1]:65535/v1", valid: true},
		{input: "localhost:19099"},
		{input: "0.0.0.0:19099"},
		{input: "192.168.1.2:19099"},
		{input: "[::2]:19099"},
		{input: "127.0.0.1:0"},
		{input: "127.0.0.1:65536"},
		{input: "127.0.0.1"},
	}
	for _, test := range tests {
		t.Run(test.input, func(t *testing.T) {
			got, err := NormalizeListener(test.input)
			if !test.valid {
				if err == nil {
					t.Fatalf("NormalizeListener() = %+v, want error", got)
				}
				return
			}
			if err != nil || got.Authority != test.authority || got.RouterBaseURL != test.router || got.APIBaseURL != test.api {
				t.Fatalf("NormalizeListener() = %+v, %v", got, err)
			}
		})
	}
}
