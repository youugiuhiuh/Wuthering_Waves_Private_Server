package pkg

import (
	"errors"
	"net/http"
	"strings"
	"testing"
	"time"
)

func TestNewNetworkDisablesBindingWhenWiFiIsOff(t *testing.T) {
	network, err := newNetwork(false, func() (wifiInterface, error) {
		t.Fatal("discovery must not run")
		return wifiInterface{}, nil
	})
	if err != nil || network.Dialer().Control != nil {
		t.Fatalf("expected unbound network, got %v", err)
	}
}

func TestNewNetworkFailsWhenWiFiDiscoveryFails(t *testing.T) {
	_, err := newNetwork(true, func() (wifiInterface, error) {
		return wifiInterface{}, errors.New("no WiFi")
	})
	if err == nil || !strings.Contains(err.Error(), "no WiFi") {
		t.Fatalf("expected discovery error, got %v", err)
	}
}

func TestNetworkHTTPClientUsesDialerAndConfiguration(t *testing.T) {
	network, err := newNetwork(false, nil)
	if err != nil {
		t.Fatal(err)
	}

	client := network.HTTPClient(time.Second, func(transport *http.Transport) {
		transport.DisableKeepAlives = true
	})
	transport, ok := client.Transport.(*http.Transport)
	if !ok || client.Timeout != time.Second || !transport.DisableKeepAlives {
		t.Fatalf("unexpected client configuration: %#v", client)
	}
}
