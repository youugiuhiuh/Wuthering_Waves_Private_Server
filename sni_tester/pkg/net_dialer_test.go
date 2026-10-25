package pkg

import (
	"net"
	"testing"
	"time"
)

func TestSetOutboundInterface(t *testing.T) {
	// 测试重置
	err := SetOutboundInterface("")
	if err != nil {
		t.Fatalf("SetOutboundInterface empty failed: %v", err)
	}
	if GetBoundInterface() != "" || GetBoundIP() != "" {
		t.Errorf("Expected empty bound info, got iface=%s ip=%s", GetBoundInterface(), GetBoundIP())
	}

	// 测试回环接口 lo
	err = SetOutboundInterface("lo")
	if err != nil {
		t.Fatalf("SetOutboundInterface lo failed: %v", err)
	}
	if GetBoundInterface() != "lo" {
		t.Errorf("Expected iface lo, got %s", GetBoundInterface())
	}
	if GetBoundIP() != "127.0.0.1" {
		t.Errorf("Expected IP 127.0.0.1, got %s", GetBoundIP())
	}

	// 测试生成的 Dialer
	dialer := NewDialer(2 * time.Second)
	if netDialer, ok := dialer.(*net.Dialer); ok {
		if netDialer.LocalAddr == nil {
			t.Fatal("Expected dialer.LocalAddr to be set")
		}
	} else {
		t.Fatal("Expected *net.Dialer type")
	}

	// 测试直接传 IP
	err = SetOutboundInterface("192.168.1.100")
	if err != nil {
		t.Fatalf("SetOutboundInterface IP failed: %v", err)
	}
	if GetBoundIP() != "192.168.1.100" {
		t.Errorf("Expected 192.168.1.100, got %s", GetBoundIP())
	}

	// 测试不存在的接口
	err = SetOutboundInterface("nonexistent_iface_xyz")
	if err == nil {
		t.Fatal("Expected error for non-existent interface")
	}

	// 清理
	_ = SetOutboundInterface("")
}
