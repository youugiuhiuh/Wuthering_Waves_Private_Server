package pkg

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"syscall"
	"testing"
	"time"
)

func TestResolveWithDNSPassesNetworkToDoHDoTAndUDP(t *testing.T) {
	bound := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error {
		return errors.New("bound")
	}}}

	_, err := resolveWithDNS(context.Background(), "example.com", bound)
	if err == nil {
		t.Fatal("expected dial failure from supplied network")
	}
}

func TestDoHClientUsesNetworkDialer(t *testing.T) {
	bound := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error {
		return errors.New("bound")
	}}}

	_, err := lookupHostDoHWire(bound.HTTPClient(time.Second, nil), "http://127.0.0.1:1", "example.com")
	if err == nil {
		t.Fatal("expected supplied dialer error")
	}
}

func TestDoHRejectsMissingNetworkClient(t *testing.T) {
	called := false
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		called = true
	}))
	defer server.Close()

	_, err := lookupHostDoHWire(nil, server.URL, "example.com")
	if err == nil {
		t.Fatal("expected missing network client error")
	}
	if called {
		t.Fatal("DoH request used a default client")
	}
}
