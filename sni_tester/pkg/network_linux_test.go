//go:build linux

package pkg

import (
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDiscoverLinuxWiFiSelectsUpIPv4Interface(t *testing.T) {
	root := t.TempDir()
	interfaces, err := net.Interfaces()
	if err != nil || len(interfaces) == 0 {
		t.Fatalf("list interfaces: %v", err)
	}
	name := interfaces[0].Name
	if err := os.MkdirAll(filepath.Join(root, name, "wireless"), 0o755); err != nil {
		t.Fatal(err)
	}

	iface, err := discoverLinuxWiFi(root, func(name string) ([]net.Addr, error) {
		return []net.Addr{&net.IPNet{IP: net.ParseIP("192.0.2.2")}}, nil
	})
	if err != nil || iface.name != name {
		t.Fatalf("got %#v, %v", iface, err)
	}
}

func TestDiscoverLinuxWiFiRejectsInterfaceWithoutIPv4(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "wlan0", "wireless"), 0o755); err != nil {
		t.Fatal(err)
	}
	_, err := discoverLinuxWiFi(root, func(string) ([]net.Addr, error) { return nil, nil })
	if err == nil {
		t.Fatal("expected no usable WiFi error")
	}
}

func TestBindInterfaceReportsCAPNetRawRequirement(t *testing.T) {
	err := bindInterface(^uintptr(0), 0, "wlan0")
	if err == nil || !strings.Contains(err.Error(), "CAP_NET_RAW") {
		t.Fatalf("expected CAP_NET_RAW error, got %v", err)
	}
}
