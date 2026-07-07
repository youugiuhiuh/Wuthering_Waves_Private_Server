package config

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
)

const defaultConfigPath = "/etc/wwps/sub-server/config.json"

type Config struct {
	ListenAddr string `json:"listen_addr"`
	TLSCert    string `json:"tls_cert"`
	TLSKey     string `json:"tls_key"`
	AegisGrpc  string `json:"aegis_grpc"`
	RateLimit  int    `json:"rate_limit"`
	CacheTTL   int    `json:"cache_ttl"`
}

func defaultConfig() *Config {
	return &Config{
		ListenAddr: ":8443",
		AegisGrpc:  "unix:///var/run/aegis/sub.sock",
		RateLimit:  10,
		CacheTTL:   60,
	}
}

func loadConfigFile(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	cfg := &Config{}
	if err := json.Unmarshal(data, cfg); err != nil {
		return nil, fmt.Errorf("parse config file: %w", err)
	}
	return cfg, nil
}

func Parse() *Config {
	cfg := defaultConfig()

	// Try loading from config file first
	if fileCfg, err := loadConfigFile(defaultConfigPath); err == nil {
		if fileCfg.ListenAddr != "" {
			cfg.ListenAddr = fileCfg.ListenAddr
		}
		if fileCfg.TLSCert != "" {
			cfg.TLSCert = fileCfg.TLSCert
		}
		if fileCfg.TLSKey != "" {
			cfg.TLSKey = fileCfg.TLSKey
		}
		if fileCfg.AegisGrpc != "" {
			cfg.AegisGrpc = fileCfg.AegisGrpc
		}
		if fileCfg.RateLimit > 0 {
			cfg.RateLimit = fileCfg.RateLimit
		}
		if fileCfg.CacheTTL > 0 {
			cfg.CacheTTL = fileCfg.CacheTTL
		}
	}

	// CLI flags override config file
	flag.StringVar(&cfg.ListenAddr, "listen-addr", cfg.ListenAddr, "listen address")
	flag.StringVar(&cfg.TLSCert, "tls-cert", cfg.TLSCert, "TLS certificate path")
	flag.StringVar(&cfg.TLSKey, "tls-key", cfg.TLSKey, "TLS key path")
	flag.StringVar(&cfg.AegisGrpc, "aegis-grpc", cfg.AegisGrpc, "Aegis gRPC socket")
	flag.IntVar(&cfg.RateLimit, "rate-limit", cfg.RateLimit, "requests per minute per token")
	flag.IntVar(&cfg.CacheTTL, "cache-ttl", cfg.CacheTTL, "cache TTL in seconds")
	flag.Parse()

	return cfg
}
