package config

import (
	"testing"
	"time"
)

func TestLoadPrecedence(t *testing.T) {
	t.Setenv("MTLS_LISTEN_ADDR", "127.0.0.1:2")
	t.Setenv("MTLS_UPSTREAM_URL", "https://env.example")
	t.Setenv("MTLS_TLS_MIN", "tls1.3")
	t.Setenv("MTLS_TIMEOUT", "3s")
	t.Setenv("MTLS_DEBUG", "true")

	cfg, err := Load(Defaults{ListenAddr: "127.0.0.1:1", UpstreamURL: "https://default.example", TLSMin: "tls1.2", Timeout: time.Second}, []string{
		"-listen", "127.0.0.1:3", "-upstream", "https://flag.example", "-tls-min", "tls1.2", "-timeout", "4s", "-debug=false",
	})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.ListenAddr != "127.0.0.1:3" || cfg.UpstreamURL != "https://flag.example" || cfg.TLSMin != "tls1.2" || cfg.Timeout != 4*time.Second || cfg.Debug {
		t.Fatalf("unexpected config: %+v", cfg)
	}
}

func TestLoadRejectsMissingOrInvalidUpstream(t *testing.T) {
	if _, err := Load(Defaults{}, nil); err == nil {
		t.Fatal("expected missing upstream error")
	}
	if _, err := Load(Defaults{UpstreamURL: "://bad"}, nil); err == nil {
		t.Fatal("expected invalid upstream error")
	}
}

func TestLoadTLSMinValidation(t *testing.T) {
	for _, version := range []string{"tls1.2", "tls1.3"} {
		if _, err := Load(Defaults{UpstreamURL: "https://example.test", TLSMin: version}, nil); err != nil {
			t.Fatalf("%s should be valid: %v", version, err)
		}
	}
	if _, err := Load(Defaults{UpstreamURL: "https://example.test", TLSMin: "tls1.1"}, nil); err == nil {
		t.Fatal("expected invalid TLS version error")
	}
}
