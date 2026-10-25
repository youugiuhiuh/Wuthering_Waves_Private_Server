package pkg

import (
	"context"
	"fmt"
	"net"
	"net/url"
	"sync"
	"time"

	"golang.org/x/net/proxy"
)

var (
	proxyMu     sync.RWMutex
	globalProxy string
	proxyDialer proxy.ContextDialer
)

// SetOutboundProxy sets the outbound proxy (e.g. "socks5://127.0.0.1:1080").
func SetOutboundProxy(proxyURLStr string) error {
	proxyMu.Lock()
	defer proxyMu.Unlock()

	if proxyURLStr == "" {
		globalProxy = ""
		proxyDialer = nil
		return nil
	}

	u, err := url.Parse(proxyURLStr)
	if err != nil {
		return fmt.Errorf("invalid proxy URL %q: %w", proxyURLStr, err)
	}

	switch u.Scheme {
	case "socks5", "socks5h":
		var auth *proxy.Auth
		if u.User != nil {
			auth = &proxy.Auth{
				User: u.User.Username(),
			}
			auth.Password, _ = u.User.Password()
		}

		baseDialer := &net.Dialer{
			Timeout: 5 * time.Second,
		}

		d, err := proxy.SOCKS5("tcp", u.Host, auth, baseDialer)
		if err != nil {
			return fmt.Errorf("failed to create socks5 dialer: %w", err)
		}

		ctxDialer, ok := d.(proxy.ContextDialer)
		if !ok {
			return fmt.Errorf("socks5 dialer does not implement ContextDialer")
		}

		globalProxy = proxyURLStr
		proxyDialer = ctxDialer
		return nil

	default:
		return fmt.Errorf("unsupported proxy protocol %q: currently socks5 and socks5h are supported", u.Scheme)
	}
}

// GetOutboundProxy returns the current global outbound proxy URL.
func GetOutboundProxy() string {
	proxyMu.RLock()
	defer proxyMu.RUnlock()
	return globalProxy
}

// ContextDialer matches the interface needed to establish connections.
type ContextDialer interface {
	DialContext(ctx context.Context, network, addr string) (net.Conn, error)
}
