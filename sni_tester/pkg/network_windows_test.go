//go:build windows

package pkg

import (
	"net"
	"testing"
)

func TestConfiguredIPv4RejectsUnspecifiedAddress(t *testing.T) {
	if configuredIPv4(net.IPv4zero) {
		t.Fatal("unspecified IPv4 address must not select a WiFi adapter")
	}
}
