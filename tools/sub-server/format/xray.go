package format

import (
	"encoding/json"
	"fmt"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func buildXrayOutbound(cfg *pb.ProxyConfig) map[string]interface{} {
	outbound := map[string]interface{}{
		"protocol": cfg.GetProtocol(),
		"tag":      cfg.GetTag(),
		"settings": map[string]interface{}{},
	}

	switch cfg.GetProtocol() {
	case "vless":
		users := map[string]interface{}{
			"id":         cfg.GetUuid(),
			"encryption": "none",
		}
		if f := cfg.GetFlow(); f != "" {
			users["flow"] = f
		}
		settings := map[string]interface{}{
			"vnext": []map[string]interface{}{
				{
					"address": cfg.GetHost(),
					"port":    cfg.GetPort(),
					"users":   []map[string]interface{}{users},
				},
			},
		}
		outbound["settings"] = settings

		streamSettings := buildXrayStreamSettings(cfg)
		if len(streamSettings) > 0 {
			outbound["streamSettings"] = streamSettings
		}
	}

	return outbound
}

func buildXrayStreamSettings(cfg *pb.ProxyConfig) map[string]interface{} {
	s := make(map[string]interface{})

	transport := cfg.GetTransport()
	if transport == "" {
		transport = "tcp"
	}
	s["network"] = transport

	if cfg.GetPublicKey() != "" {
		s["security"] = "reality"
		reality := map[string]interface{}{
			"serverName":  cfg.GetSni(),
			"fingerprint": cfg.GetFingerprint(),
			"publicKey":   cfg.GetPublicKey(),
			"shortId":     cfg.GetShortId(),
		}
		if spx := cfg.GetSpx(); spx != "" {
			reality["shortPath"] = spx
		}
		s["realitySettings"] = reality
	} else if cfg.GetSni() != "" {
		s["security"] = "tls"
		tls := map[string]interface{}{
			"serverName":    cfg.GetSni(),
			"allowInsecure": cfg.GetInsecure(),
			"fingerprint":   cfg.GetFingerprint(),
		}
		if alpn := cfg.GetAlpn(); alpn != "" {
			tls["alpn"] = []string{alpn}
		}
		s["tlsSettings"] = tls
	}

	switch transport {
	case "ws":
		ws := make(map[string]interface{})
		if p := cfg.GetPath(); p != "" {
			ws["path"] = p
		}
		host := cfg.GetHttpHost()
		if host == "" {
			host = cfg.GetSni()
		}
		if host != "" {
			ws["headers"] = map[string]string{"Host": host}
		}
		if len(ws) > 0 {
			s["wsSettings"] = ws
		}
	case "xhttp":
		xhttp := make(map[string]interface{})
		if p := cfg.GetPath(); p != "" {
			xhttp["path"] = p
		}
		if h := cfg.GetHttpHost(); h != "" {
			xhttp["host"] = h
		}
		if m := cfg.GetMode(); m != "" {
			xhttp["mode"] = m
		}
		if len(xhttp) > 0 {
			s["xhttpSettings"] = xhttp
		}
	case "grpc":
		grpc := make(map[string]interface{})
		if svc := cfg.GetServiceName(); svc != "" {
			grpc["serviceName"] = svc
		}
		if len(grpc) > 0 {
			s["grpcSettings"] = grpc
		}
	}

	return s
}

func ToXrayJSON(configs []*pb.ProxyConfig) (string, error) {
	outbounds := make([]map[string]interface{}, 0)
	for _, cfg := range configs {
		if ob := buildXrayOutbound(cfg); ob != nil {
			outbounds = append(outbounds, ob)
		}
	}
	result := map[string]interface{}{
		"outbounds": outbounds,
		"log": map[string]interface{}{
			"loglevel": "warning",
		},
	}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return "", fmt.Errorf("xray json marshal: %w", err)
	}
	return string(data), nil
}
