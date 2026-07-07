package config

import "flag"

type Config struct {
	ListenAddr string
	TLSCert    string
	TLSKey     string
	AegisGrpc  string
	RateLimit  int
	CacheTTL   int
}

func Parse() *Config {
	cfg := &Config{}
	flag.StringVar(&cfg.ListenAddr, "listen-addr", ":8443", "listen address")
	flag.StringVar(&cfg.TLSCert, "tls-cert", "", "TLS certificate path")
	flag.StringVar(&cfg.TLSKey, "tls-key", "", "TLS key path")
	flag.StringVar(&cfg.AegisGrpc, "aegis-grpc", "unix:///var/run/aegis/sub.sock", "Aegis gRPC socket")
	flag.IntVar(&cfg.RateLimit, "rate-limit", 10, "requests per minute per token")
	flag.IntVar(&cfg.CacheTTL, "cache-ttl", 60, "cache TTL in seconds")
	flag.Parse()
	return cfg
}
