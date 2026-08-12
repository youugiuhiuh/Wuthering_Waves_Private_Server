package pkg

import (
	"errors"
	"net"
	"path/filepath"
	"syscall"
	"testing"
)

func TestGeoDownloadUsesProvidedNetworkDialer(t *testing.T) {
	called := false
	network := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error {
		called = true
		return errors.New("bound dialer used")
	}}}

	err := tryDownload(filepath.Join(t.TempDir(), "geo.mmdb"), "http://127.0.0.1:1", "", network)
	if err == nil || !called {
		t.Fatalf("expected provided dialer to be used, got %v", err)
	}
}
