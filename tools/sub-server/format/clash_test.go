package format

import (
	"strings"
	"testing"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func TestToClashYAML_HasProxies(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:    "vless",
		Uuid:        "uuid-123",
		Host:        "example.com",
		Port:        443,
		Tag:         "my-vless",
		Sni:         "example.com",
		Fingerprint: "chrome",
		PublicKey:   "abc123",
		ShortId:     "def456",
		Transport:   "tcp",
		Flow:        "xtls-rprx-vision",
	}
	yaml, err := ToClashYAML([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(yaml, "my-vless") {
		t.Error("should contain proxy name")
	}
	if !strings.Contains(yaml, "client-fingerprint: chrome") {
		t.Error("REALITY fingerprint missing")
	}
	if !strings.Contains(yaml, "public-key: abc123") {
		t.Error("reality-opts public-key missing")
	}
}

func TestToClashYAML_Hysteria2(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:     "hysteria2",
		Host:         "example.com",
		Port:         443,
		Password:     "pass",
		Sni:          "example.com",
		HopPortStart: 10000,
		HopPortEnd:   20000,
		Tag:          "hy2-port-hop",
	}
	yaml, err := ToClashYAML([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(yaml, "ports: 10000-20000") {
		t.Errorf("hop ports should be rendered, got:\n%s", yaml)
	}
}
