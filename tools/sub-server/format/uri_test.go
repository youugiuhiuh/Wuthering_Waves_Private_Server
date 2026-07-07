package format

import (
	"strings"
	"testing"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func TestBuildURI_VLESS_URLEncoding(t *testing.T) {
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
		Tag:         "my-node",
	}
	uri := BuildURI(cfg)
	if strings.Contains(uri, "spx=a+b/c") {
		t.Errorf("spx should be URL-encoded, got: %s", uri)
	}
	if !strings.Contains(uri, "spx=a%2Bb%2Fc") {
		t.Errorf("spx not correctly URL-encoded, got: %s", uri)
	}
	if !strings.Contains(uri, "fp=chrome") {
		t.Errorf("fingerprint missing, got: %s", uri)
	}
}

func TestBuildURI_Hysteria2_CertSHA256(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:   "hysteria2",
		Host:       "example.com",
		Port:       443,
		Password:   "pass",
		CertSha256: "abc123...",
	}
	uri := BuildURI(cfg)
	if !strings.Contains(uri, "pinSHA256=abc123") {
		t.Errorf("should use pinSHA256 when cert_sha256 available, got: %s", uri)
	}
	if strings.Contains(uri, "insecure=1") {
		t.Errorf("should not use insecure=1 when cert_sha256 set, got: %s", uri)
	}
}

func TestBuildURI_TUIC(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:          "tuic",
		Password:          "pass",
		Host:              "example.com",
		Port:              443,
		CongestionControl: "bbr",
		Alpn:              "h3",
		Sni:               "example.com",
	}
	uri := BuildURI(cfg)
	if !strings.Contains(uri, "alpn=h3") {
		t.Errorf("alpn missing, got: %s", uri)
	}
}

func TestBuildURI_Hysteria2_NoCert_Insecure(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol: "hysteria2",
		Host:     "example.com",
		Port:     443,
		Password: "pass",
	}
	uri := BuildURI(cfg)
	if !strings.Contains(uri, "insecure=1") {
		t.Errorf("should use insecure=1 when no cert_sha256, got: %s", uri)
	}
}

func TestBuildURI_VLESS_AllParams(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:    "vless",
		Uuid:        "uuid-123",
		Host:        "example.com",
		Port:        443,
		Sni:         "sni.example.com",
		Fingerprint: "chrome",
		PublicKey:   "abc123",
		ShortId:     "def456",
		Spx:         "spxval",
		Transport:   "ws",
		Flow:        "xtls-rprx-vision",
		Mode:        "auto",
		HttpHost:    "host.example.com",
		HeaderType:  "none",
		Alpn:        "h2,h3",
		Tag:         "my-node",
	}
	uri := BuildURI(cfg)
	for _, param := range []string{"mode=auto", "host=host.example.com", "headerType=none", "alpn=h2%2Ch3"} {
		if !strings.Contains(uri, param) {
			t.Errorf("missing param %q in uri: %s", param, uri)
		}
	}
}
