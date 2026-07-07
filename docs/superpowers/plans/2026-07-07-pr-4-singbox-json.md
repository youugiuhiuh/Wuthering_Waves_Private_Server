# PR-4: Fix Sing-box JSON Output

**Goal:** Produced Sing-box JSON works in SFA/SFI / Hiddify / NekoBox with full transport, REALITY, TLS settings, and DNS/route sections.

**Architecture:** Rewrite `ToSingBox` to build structured JSON outbounds with transport settings, REALITY/TLS configs, and add DNS + route sections.

**Tech Stack:** Go (map building, encoding/json)

## Global Constraints
- All Go changes must pass `go fmt ./... && go build ./... && go vet ./... && go test ./...`
- Sing-box JSON must be valid JSON (json.MarshalIndent)
- DNS section must use `dns` object with at least one public DNS server
- Route section must route CN traffic through `direct` outbound
- New proto fields available: Fingerprint, Spx, HttpHost, Alpn, Mode, ServiceName, CertSha256, Insecure

---

### Task 1: Rewrite Sing-box JSON with full transport/REALITY/TLS + DNS/route

**Files:**
- Modify: `tools/sub-server/format/singbox.go`
- Test: `tools/sub-server/format/singbox_test.go`

#### Implementation

Replace the entire `ToSingBox` function to produce complete Sing-box configuration:

```go
package format

import (
	"encoding/json"
	"fmt"
	"strconv"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func buildSingboxTransport(cfg *pb.ProxyConfig) map[string]interface{} {
	t := make(map[string]interface{})
	switch cfg.GetTransport() {
	case "ws":
		t["type"] = "ws"
		path := cfg.GetPath()
		if path != "" {
			t["path"] = path
		}
		headers := make(map[string]string)
		host := cfg.GetHttpHost()
		if host == "" {
			host = cfg.GetSni()
		}
		if host != "" {
			headers["Host"] = host
		}
		if len(headers) > 0 {
			t["headers"] = headers
		}
	case "xhttp":
		t["type"] = "xhttp"
		if p := cfg.GetPath(); p != "" {
			t["path"] = p
		}
		if h := cfg.GetHttpHost(); h != "" {
			t["host"] = h
		}
		if m := cfg.GetMode(); m != "" {
			t["mode"] = m
		}
		if cfg.GetExtra() != "" {
			var extra interface{}
			if json.Unmarshal([]byte(cfg.GetExtra()), &extra) == nil {
				t["extra"] = extra
			}
		}
	case "grpc":
		t["type"] = "grpc"
		if s := cfg.GetServiceName(); s != "" {
			t["service_name"] = s
		}
		if a := cfg.GetAuthority(); a != "" {
			t["authority"] = a
		}
	default:
		// tcp: no transport object needed in sing-box
		return nil
	}
	return t
}

func buildSingboxTLS(cfg *pb.ProxyConfig) map[string]interface{} {
	sni := cfg.GetSni()
	fp := cfg.GetFingerprint()
	certSha := cfg.GetCertSha256()
	if sni == "" && fp == "" && certSha == "" {
		return nil
	}
	tls := make(map[string]interface{})
	tls["enabled"] = true
	if sni != "" {
		tls["server_name"] = sni
	}
	if fp != "" {
		tls["utls"] = map[string]interface{}{
			"enabled":     true,
			"fingerprint": fp,
		}
	}
	if cfg.GetInsecure() {
		tls["insecure"] = true
	}
	if certSha != "" {
		tls["certificate"] = fmt.Sprintf("sha256:%s", certSha)
	}
	if cfg.GetAlpn() != "" {
		tls["alpn"] = []string{cfg.GetAlpn()}
	}
	return tls
}

func buildSingboxReality(cfg *pb.ProxyConfig) map[string]interface{} {
	if cfg.GetPublicKey() == "" {
		return nil
	}
	r := make(map[string]interface{})
	r["enabled"] = true
	r["public_key"] = cfg.GetPublicKey()
	r["short_id"] = cfg.GetShortId()
	if cfg.GetSni() != "" {
		r["server_name"] = cfg.GetSni()
	}
	if spx := cfg.GetSpx(); spx != "" {
		r["short_path"] = spx
	}
	return r
}

func ToSingBox(configs []*pb.ProxyConfig) (string, error) {
	outbounds := make([]map[string]interface{}, 0)
	for _, cfg := range configs {
		outbound := map[string]interface{}{
			"type":        cfg.GetProtocol(),
			"tag":         cfg.GetTag(),
			"server":      cfg.GetHost(),
			"server_port": cfg.GetPort(),
		}
		switch cfg.GetProtocol() {
		case "vless":
			if u := cfg.GetUuid(); u != "" {
				outbound["uuid"] = u
			}
			if f := cfg.GetFlow(); f != "" {
				outbound["flow"] = f
			}
			if enc := cfg.GetEncryption(); enc != "" {
				outbound["encryption"] = enc
			}
			if t := buildSingboxTransport(cfg); t != nil {
				outbound["transport"] = t
			}
			if r := buildSingboxReality(cfg); r != nil {
				outbound["reality"] = r
			}
			// For non-reality TLS
			if cfg.GetPublicKey() == "" {
				if tls := buildSingboxTLS(cfg); tls != nil {
					outbound["tls"] = tls
				}
			}
		case "hysteria2", "hy2":
			outbound["password"] = cfg.GetPassword()
			if cfg.GetHopPortStart() > 0 && cfg.GetHopPortEnd() > cfg.GetHopPortStart() {
				outbound["hop_port"] = strconv.Itoa(int(cfg.GetHopPortStart())) + "-" + strconv.Itoa(int(cfg.GetHopPortEnd()))
			}
			if cfg.GetObfsType() != "" {
				outbound["obfs"] = map[string]string{
					"type":     cfg.GetObfsType(),
					"password": cfg.GetObfsPassword(),
				}
			}
			if tls := buildSingboxTLS(cfg); tls != nil {
				outbound["tls"] = tls
			}
		case "tuic":
			outbound["password"] = cfg.GetPassword()
			if cc := cfg.GetCongestionControl(); cc != "" {
				outbound["congestion_control"] = cc
			}
			outbound["udp_relay_mode"] = "native"
			outbound["heartbeat"] = "10s"
			if tls := buildSingboxTLS(cfg); tls != nil {
				outbound["tls"] = tls
			}
		}
		outbounds = append(outbounds, outbound)
	}

	result := map[string]interface{}{
		"outbounds": outbounds,
		"dns": map[string]interface{}{
			"servers": []map[string]interface{}{
				{"address": "https://1.1.1.1/dns-query", "address_resolver": "local"},
				{"address": "local", "detour": "direct"},
			},
			"rules": []map[string]interface{}{
				{"outbound": "any", "server": "local"},
				{"geosite": "cn", "server": "local"},
			},
		},
		"route": map[string]interface{}{
			"rules": []map[string]interface{}{
				{"geoip": "cn", "outbound": "direct"},
				{"geosite": "cn", "outbound": "direct"},
			},
		},
		"inbounds": []map[string]interface{}{
			{
				"type": "tun",
				"tag":  "tun-in",
				"inet4_address": "172.19.0.1/30",
				"auto_route":    true,
				"strict_route":  false,
			},
		},
	}

	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return "", err
	}
	return string(data), nil
}
```

#### Tests

```go
package format

import (
	"strings"
	"testing"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func TestToSingBox_VLESS_Reality(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:    "vless",
		Uuid:        "uuid-123",
		Host:        "example.com",
		Port:        443,
		Sni:         "example.com",
		Fingerprint: "chrome",
		PublicKey:   "abc123",
		ShortId:     "def456",
		Spx:         "a+b/c",
		Transport:   "tcp",
		Flow:        "xtls-rprx-vision",
		Tag:         "my-vless",
	}
	json, err := ToSingBox([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(json, "uuid-123") { t.Error("uuid missing") }
	if !strings.Contains(json, "\"reality\"") { t.Error("reality section missing") }
	if !strings.Contains(json, "chrome") { t.Error("fingerprint missing") }
	if !strings.Contains(json, "\"dns\"") { t.Error("dns section missing") }
	if !strings.Contains(json, "\"route\"") { t.Error("route section missing") }
}

func TestToSingBox_Hysteria2(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:    "hysteria2",
		Host:        "example.com",
		Port:        443,
		Password:    "pass",
		Sni:         "example.com",
		HopPortStart: 10000,
		HopPortEnd:   20000,
		ObfsType:    "salamander",
		ObfsPassword: "obf-pass",
		Tag:         "hy2",
	}
	json, err := ToSingBox([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(json, "hop_port") { t.Error("hop_port missing") }
	if !strings.Contains(json, "salamander") { t.Error("obfs missing") }
}

func TestToSingBox_VLESS_Ws(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol: "vless",
		Uuid:     "uuid",
		Host:     "example.com",
		Port:     443,
		Sni:      "example.com",
		Fingerprint: "chrome",
		Transport:   "ws",
		Path:        "/ws",
		HttpHost:    "example.com",
		Tag:         "ws-vless",
	}
	json, err := ToSingBox([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(json, "\"type\": \"ws\"") { t.Error("ws transport missing") }
	if !strings.Contains(json, "/ws") { t.Error("ws path missing") }
}
```

#### Quality Gates
```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-4-singbox-json/tools/sub-server
go fmt ./... && go build ./... && go vet ./... && go test ./... -v 2>&1
```

#### Commit
```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-4-singbox-json
git add tools/sub-server/format/singbox.go tools/sub-server/format/singbox_test.go
git commit -m "fix(singbox): full transport/REALITY/TLS support with DNS and route sections"
```
