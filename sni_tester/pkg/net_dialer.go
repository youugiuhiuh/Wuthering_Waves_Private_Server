package pkg

import (
	"fmt"
	"net"
	"sync"
	"syscall"
	"time"
)

var (
	bindMu         sync.RWMutex
	boundInterface string
	boundIP        net.IP
)

// SetOutboundInterface sets the egress network interface (e.g. "wlp6s0") or local IP.
// When set, all TLS connections and DNS queries are bound to this interface/IP,
// fully isolating test traffic from the default broadband interface.
func SetOutboundInterface(ifaceOrIP string) error {
	bindMu.Lock()
	defer bindMu.Unlock()

	if ifaceOrIP == "" {
		boundInterface = ""
		boundIP = nil
		return nil
	}

	// 1. 直接指定 IP
	if ip := net.ParseIP(ifaceOrIP); ip != nil {
		boundIP = ip
		boundInterface = ""
		return nil
	}

	// 2. 指定网卡名称 (如 wlp6s0)
	iface, err := net.InterfaceByName(ifaceOrIP)
	if err != nil {
		return fmt.Errorf("network interface %s not found: %w", ifaceOrIP, err)
	}

	addrs, err := iface.Addrs()
	if err != nil {
		return fmt.Errorf("failed to get addresses for interface %s: %w", ifaceOrIP, err)
	}

	var foundIP net.IP
	for _, a := range addrs {
		var ip net.IP
		switch v := a.(type) {
		case *net.IPNet:
			ip = v.IP
		case *net.IPAddr:
			ip = v.IP
		}
		if ip != nil && ip.To4() != nil {
			foundIP = ip
			break
		}
	}

	if foundIP == nil {
		return fmt.Errorf("interface %s has no active IPv4 address", ifaceOrIP)
	}

	boundInterface = iface.Name
	boundIP = foundIP
	return nil
}

func GetBoundInterface() string {
	bindMu.RLock()
	defer bindMu.RUnlock()
	return boundInterface
}

func GetBoundIP() string {
	bindMu.RLock()
	defer bindMu.RUnlock()
	if boundIP == nil {
		return ""
	}
	return boundIP.String()
}

// NewDialer creates a ContextDialer respecting globalProxy, interface, and local IP.
func NewDialer(timeout time.Duration) ContextDialer {
	proxyMu.RLock()
	pd := proxyDialer
	proxyMu.RUnlock()
	if pd != nil {
		return pd
	}
	bindMu.RLock()
	iface := boundInterface
	ip := boundIP
	bindMu.RUnlock()

	dialer := &net.Dialer{
		Timeout: timeout,
	}

	if ip != nil {
		dialer.LocalAddr = &net.TCPAddr{IP: ip}
	}

	if iface != "" {
		dialer.Control = func(network, address string, c syscall.RawConn) error {
			var operr error
			err := c.Control(func(fd uintptr) {
				operr = setSocketBindToDevice(int(fd), iface)
			})
			if err != nil {
				return err
			}
			return operr
		}
	}

	return dialer
}

// NewUDPDialer creates a net.Dialer configured for UDP with LocalAddr bound.
func NewUDPDialer(timeout time.Duration) *net.Dialer {
	bindMu.RLock()
	iface := boundInterface
	ip := boundIP
	bindMu.RUnlock()

	dialer := &net.Dialer{
		Timeout: timeout,
	}

	if ip != nil {
		dialer.LocalAddr = &net.UDPAddr{IP: ip}
	}

	if iface != "" {
		dialer.Control = func(network, address string, c syscall.RawConn) error {
			var operr error
			err := c.Control(func(fd uintptr) {
				operr = setSocketBindToDevice(int(fd), iface)
			})
			if err != nil {
				return err
			}
			return operr
		}
	}

	return dialer
}
