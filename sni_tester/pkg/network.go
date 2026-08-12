package pkg

import (
	"fmt"
	"net"
	"net/http"
	"syscall"
	"time"
)

type wifiInterface struct {
	name  string
	index uint32
}

// Network creates clients that either use the system route or the active WiFi interface.
type Network struct {
	dialer *net.Dialer
}

func newNetwork(wifi bool, discover func() (wifiInterface, error)) (*Network, error) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	if !wifi {
		return &Network{dialer: dialer}, nil
	}

	iface, err := discover()
	if err != nil {
		return nil, fmt.Errorf("find active WiFi interface: %w", err)
	}
	dialer.Control = func(_, _ string, raw syscall.RawConn) error {
		var bindErr error
		if err := raw.Control(func(fd uintptr) {
			bindErr = bindInterface(fd, iface.index, iface.name)
		}); err != nil {
			return err
		}
		return bindErr
	}
	return &Network{dialer: dialer}, nil
}

func NewNetwork(wifi bool) (*Network, error) {
	return newNetwork(wifi, discoverWiFi)
}

func (n *Network) Dialer() *net.Dialer {
	return n.dialer
}

func (n *Network) HTTPClient(timeout time.Duration, configure func(*http.Transport)) *http.Client {
	transport := &http.Transport{DialContext: n.dialer.DialContext}
	if configure != nil {
		configure(transport)
	}
	return &http.Client{Transport: transport, Timeout: timeout}
}
