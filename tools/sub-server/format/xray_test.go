package format

import (
	"strings"
	"testing"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func TestToXrayJSON_VLESS_Reality(t *testing.T) {
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
	json, err := ToXrayJSON([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(json, "uuid-123") {
		t.Error("uuid missing")
	}
	if !strings.Contains(json, "\"security\": \"reality\"") {
		t.Error("reality security missing")
	}
	if !strings.Contains(json, "abc123") {
		t.Error("publicKey missing")
	}
	if !strings.Contains(json, "xtls-rprx-vision") {
		t.Error("flow missing")
	}
	if !strings.Contains(json, "\"loglevel\": \"warning\"") {
		t.Error("log section missing")
	}
}

func TestToXrayJSON_VLESS_Ws(t *testing.T) {
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
	json, err := ToXrayJSON([]*pb.ProxyConfig{cfg})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(json, "\"network\": \"ws\"") {
		t.Error("ws network missing")
	}
	if !strings.Contains(json, "/ws") {
		t.Error("ws path missing")
	}
	if !strings.Contains(json, "\"security\": \"tls\"") {
		t.Error("tls security missing")
	}
}
