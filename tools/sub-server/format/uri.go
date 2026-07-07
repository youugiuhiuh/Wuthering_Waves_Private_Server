package format

import (
	"fmt"
	"net/url"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func BuildURI(cfg *pb.ProxyConfig) string {
	switch cfg.Protocol {
	case "vless":
		return buildVLESSUri(cfg)
	case "hysteria2", "hy2":
		return buildHysteria2Uri(cfg)
	case "tuic":
		return buildTUICUri(cfg)
	default:
		return ""
	}
}

func buildVLESSUri(cfg *pb.ProxyConfig) string {
	params := url.Values{}
	params.Set("encryption", cfg.GetEncryption())
	if params.Get("encryption") == "" {
		params.Set("encryption", "none")
	}
	params.Set("security", "reality")
	params.Set("sni", cfg.GetSni())
	params.Set("fp", cfg.GetFingerprint())
	params.Set("pbk", cfg.GetPublicKey())
	params.Set("sid", cfg.GetShortId())
	if spx := cfg.GetSpx(); spx != "" {
		params.Set("spx", spx)
	}
	params.Set("type", cfg.GetTransport())
	params.Set("flow", cfg.GetFlow())
	if mode := cfg.GetMode(); mode != "" {
		params.Set("mode", mode)
	}
	if httpHost := cfg.GetHttpHost(); httpHost != "" {
		params.Set("host", httpHost)
	}
	if headerType := cfg.GetHeaderType(); headerType != "" {
		params.Set("headerType", headerType)
	}
	if alpn := cfg.GetAlpn(); alpn != "" {
		params.Set("alpn", alpn)
	}
	query := params.Encode()
	return fmt.Sprintf("vless://%s@%s:%d?%s#%s",
		cfg.GetUuid(), cfg.GetHost(), cfg.GetPort(), query, cfg.GetTag())
}

func buildHysteria2Uri(cfg *pb.ProxyConfig) string {
	pw := cfg.GetPassword()
	host := cfg.GetHost()
	if host == "" {
		host = "0.0.0.0"
	}
	base := fmt.Sprintf("hysteria2://%s@%s", pw, host)
	if cfg.GetHopPortStart() > 0 && cfg.GetHopPortEnd() > cfg.GetHopPortStart() {
		base = fmt.Sprintf("%s:%d-%d", base, cfg.GetHopPortStart(), cfg.GetHopPortEnd())
	} else {
		base = fmt.Sprintf("%s:%d", base, cfg.GetPort())
	}
	params := url.Values{}
	if sni := cfg.GetSni(); sni != "" {
		params.Set("sni", sni)
	}
	if cert := cfg.GetCertSha256(); cert != "" {
		params.Set("pinSHA256", cert)
	} else {
		params.Set("insecure", "1")
	}
	if obfs := cfg.GetObfsType(); obfs != "" {
		params.Set("obfs", obfs)
		if obfsPw := cfg.GetObfsPassword(); obfsPw != "" {
			params.Set("obfs-password", obfsPw)
		}
	}
	if query := params.Encode(); query != "" {
		base += "?" + query
	}
	if tag := cfg.GetTag(); tag != "" {
		base += "#" + tag
	}
	return base
}

func buildTUICUri(cfg *pb.ProxyConfig) string {
	host := cfg.GetHost()
	if host == "" {
		host = "0.0.0.0"
	}
	base := fmt.Sprintf("tuic://%s@%s:%d", cfg.GetPassword(), host, cfg.GetPort())
	params := url.Values{}
	if cc := cfg.GetCongestionControl(); cc != "" {
		params.Set("congestion_control", cc)
	}
	if alpn := cfg.GetAlpn(); alpn != "" {
		params.Set("alpn", alpn)
	}
	if sni := cfg.GetSni(); sni != "" {
		params.Set("sni", sni)
	}
	params.Set("udp_relay_mode", "native")
	if query := params.Encode(); query != "" {
		base += "?" + query
	}
	if tag := cfg.GetTag(); tag != "" {
		base += "#" + tag
	}
	return base
}
