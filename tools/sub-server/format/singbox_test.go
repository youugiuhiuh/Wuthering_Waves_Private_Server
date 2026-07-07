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
	if !strings.Contains(json, "uuid-123") {
		t.Error("uuid missing")
	}
	if !strings.Contains(json, "\"reality\"") {
		t.Error("reality section missing")
	}
	if !strings.Contains(json, "chrome") {
		t.Error("fingerprint missing")
	}
	if !strings.Contains(json, "\"dns\"") {
		t.Error("dns section missing")
	}
	if !strings.Contains(json, "\"route\"") {
		t.Error("route section missing")
	}
}

func TestToSingBox_Hysteria2(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:     "hysteria2",
		Host:         "example.com",
		Port:         443,
		Password:     "pass",
		Sni:          "example.com",
		HopPortStart: 10000,
		HopPortEnd:   20000,
		ObfsType:     "salamander",
		ObfsPassword: "obf-pass",
		Tag:          "hy2",
	}
	json, err := ToSingBox([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(json, "hop_port") {
		t.Error("hop_port missing")
	}
	if !strings.Contains(json, "salamander") {
		t.Error("obfs missing")
	}
}

func TestToSingBox_VLESS_Ws(t *testing.T) {
	cfg := &pb.ProxyConfig{
		Protocol:    "vless",
		Uuid:        "uuid",
		Host:        "example.com",
		Port:        443,
		Sni:         "example.com",
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
	if !strings.Contains(json, "\"type\": \"ws\"") {
		t.Error("ws transport missing")
	}
	if !strings.Contains(json, "/ws") {
		t.Error("ws path missing")
	}
}
