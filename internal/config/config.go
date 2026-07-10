package config

import (
	"flag"
	"fmt"
	"net/url"
	"os"
	"strconv"
	"time"

	"github.com/codeasier/mtls-router/internal/tlspolicy"
)

type Config struct {
	ListenAddr  string
	UpstreamURL string
	TLSMin      string
	Timeout     time.Duration
	Debug       bool
	Backend     bool
	LogPath     string
}

type Defaults = Config

func Load(defaults Defaults, args []string) (Config, error) {
	cfg := Config(defaults)
	applyEnv(&cfg)

	fs := flag.NewFlagSet("mtls-router", flag.ContinueOnError)
	fs.StringVar(&cfg.ListenAddr, "listen", cfg.ListenAddr, "listen address")
	fs.StringVar(&cfg.UpstreamURL, "upstream", cfg.UpstreamURL, "upstream URL")
	fs.StringVar(&cfg.TLSMin, "tls-min", cfg.TLSMin, "minimum TLS version")
	fs.DurationVar(&cfg.Timeout, "timeout", cfg.Timeout, "upstream timeout")
	fs.BoolVar(&cfg.Debug, "debug", cfg.Debug, "enable debug logging")
	fs.BoolVar(&cfg.Backend, "backend", cfg.Backend, "run in background")
	fs.StringVar(&cfg.LogPath, "log", cfg.LogPath, "log file path")
	if err := fs.Parse(args); err != nil {
		return Config{}, err
	}
	if err := cfg.Validate(); err != nil {
		return Config{}, err
	}
	return cfg, nil
}

func (c Config) Validate() error {
	if c.UpstreamURL == "" {
		return fmt.Errorf("upstream URL is required")
	}
	u, err := url.Parse(c.UpstreamURL)
	if err != nil || u.Scheme != "https" || u.Host == "" {
		return fmt.Errorf("invalid upstream URL")
	}
	if _, err := tlspolicy.MinVersion(c.TLSMin); err != nil {
		return fmt.Errorf("invalid TLS minimum version")
	}
	return nil
}

func applyEnv(cfg *Config) {
	if v := os.Getenv("MTLS_LISTEN_ADDR"); v != "" {
		cfg.ListenAddr = v
	}
	if v := os.Getenv("MTLS_UPSTREAM_URL"); v != "" {
		cfg.UpstreamURL = v
	}
	if v := os.Getenv("MTLS_TLS_MIN"); v != "" {
		cfg.TLSMin = v
	}
	if v := os.Getenv("MTLS_TIMEOUT"); v != "" {
		if d, err := time.ParseDuration(v); err == nil {
			cfg.Timeout = d
		}
	}
	if v := os.Getenv("MTLS_DEBUG"); v != "" {
		if b, err := strconv.ParseBool(v); err == nil {
			cfg.Debug = b
		}
	}
	if v := os.Getenv("MTLS_BACKEND"); v != "" {
		if b, err := strconv.ParseBool(v); err == nil {
			cfg.Backend = b
		}
	}
	if v := os.Getenv("MTLS_LOG"); v != "" {
		cfg.LogPath = v
	}
}
