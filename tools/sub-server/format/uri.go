package format

import (
	"fmt"
	pb "github.com/NicholasDewar/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func BuildURI(cfg *pb.ProxyConfig) string {
	switch cfg.Protocol {
	case "vless":
		return fmt.Sprintf("vless://%s@%s:%d?encryption=none&security=reality&sni=%s&fp=chrome&pbk=%s&sid=%s&type=%s&flow=%s#%s",
			cfg.Uuid, cfg.Host, cfg.Port, cfg.Sni, cfg.PublicKey, cfg.ShortId, cfg.Transport, cfg.Flow, cfg.Tag)
	case "hysteria2", "hy2":
		pw := cfg.Password
		host := cfg.Host
		if host == "" {
			host = "0.0.0.0"
		}
		base := fmt.Sprintf("hysteria2://%s@%s", pw, host)
		if cfg.HopPortStart > 0 && cfg.HopPortEnd > cfg.HopPortStart {
			base = fmt.Sprintf("%s:%d-%d", base, cfg.HopPortStart, cfg.HopPortEnd)
		} else {
			base = fmt.Sprintf("%s:%d", base, cfg.Port)
		}
		params := ""
		if cfg.Sni != "" {
			params = addParam(params, "sni", cfg.Sni)
		}
		params = addParam(params, "insecure", "1")
		if cfg.ObfsType != "" {
			params = addParam(params, "obfs", cfg.ObfsType)
			if cfg.ObfsPassword != "" {
				params = addParam(params, "obfs-password", cfg.ObfsPassword)
			}
		}
		if params != "" {
			base = base + "?" + params
		}
		if cfg.Tag != "" {
			base = base + "#" + cfg.Tag
		}
		return base
	case "tuic":
		host := cfg.Host
		if host == "" {
			host = "0.0.0.0"
		}
		base := fmt.Sprintf("tuic://%s@%s:%d", cfg.Password, host, cfg.Port)
		params := ""
		if cfg.CongestionControl != "" {
			params = addParam(params, "congestion_control", cfg.CongestionControl)
		}
		if cfg.Alpn != "" {
			params = addParam(params, "alpn", cfg.Alpn)
		}
		if cfg.Sni != "" {
			params = addParam(params, "sni", cfg.Sni)
		}
		params = addParam(params, "udp_relay_mode", "native")
		if params != "" {
			base = base + "?" + params
		}
		if cfg.Tag != "" {
			base = base + "#" + cfg.Tag
		}
		return base
	default:
		return ""
	}
}

func addParam(params, key, value string) string {
	if params == "" {
		return fmt.Sprintf("%s=%s", key, value)
	}
	return fmt.Sprintf("%s&%s=%s", params, key, value)
}
