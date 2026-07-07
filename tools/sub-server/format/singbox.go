package format

import (
	"encoding/json"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func ToSingBox(configs []*pb.ProxyConfig) (string, error) {
	outbounds := make([]map[string]interface{}, 0)
	for _, cfg := range configs {
		outbound := map[string]interface{}{
			"type":        cfg.Protocol,
			"tag":         cfg.Tag,
			"server":      cfg.Host,
			"server_port": cfg.Port,
		}
		switch cfg.Protocol {
		case "hysteria2", "hy2":
			outbound["password"] = cfg.Password
			if cfg.ObfsType != "" {
				outbound["obfs"] = map[string]string{
					"type":     cfg.ObfsType,
					"password": cfg.ObfsPassword,
				}
			}
		case "tuic":
			outbound["password"] = cfg.Password
			if cfg.CongestionControl != "" {
				outbound["congestion_control"] = cfg.CongestionControl
			}
		case "vless":
			outbound["uuid"] = cfg.Uuid
			if cfg.Flow != "" {
				outbound["flow"] = cfg.Flow
			}
		}
		if cfg.Sni != "" {
			outbound["tls"] = map[string]interface{}{
				"server_name": cfg.Sni,
				"enabled":     true,
				"utls":        map[string]bool{"enabled": true},
				"insecure":    true,
			}
		}
		outbounds = append(outbounds, outbound)
	}
	result := map[string]interface{}{
		"outbounds": outbounds,
	}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return "", err
	}
	return string(data), nil
}
