package pkg

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"testing"
	"time"
)

func startTestSOCKS5Server(t *testing.T) (string, func()) {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("failed to listen for mock socks5: %v", err)
	}

	closed := make(chan struct{})

	go func() {
		for {
			conn, err := ln.Accept()
			if err != nil {
				select {
				case <-closed:
					return
				default:
					continue
				}
			}
			go handleTestSOCKS5(conn)
		}
	}()

	cleanup := func() {
		close(closed)
		ln.Close()
	}

	return ln.Addr().String(), cleanup
}

func handleTestSOCKS5(conn net.Conn) {
	defer conn.Close()
	conn.SetDeadline(time.Now().Add(5 * time.Second))

	buf := make([]byte, 258)
	if _, err := io.ReadFull(conn, buf[:2]); err != nil {
		return
	}
	if buf[0] != 0x05 {
		return
	}
	nmethods := int(buf[1])
	if _, err := io.ReadFull(conn, buf[:nmethods]); err != nil {
		return
	}
	if _, err := conn.Write([]byte{0x05, 0x00}); err != nil {
		return
	}

	if _, err := io.ReadFull(conn, buf[:4]); err != nil {
		return
	}
	if buf[1] != 0x01 {
		conn.Write([]byte{0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		return
	}

	var destAddr string
	atyp := buf[3]
	switch atyp {
	case 0x01:
		if _, err := io.ReadFull(conn, buf[:4]); err != nil {
			return
		}
		ip := net.IP(buf[:4])
		var portBuf [2]byte
		if _, err := io.ReadFull(conn, portBuf[:]); err != nil {
			return
		}
		port := int(portBuf[0])<<8 | int(portBuf[1])
		destAddr = fmt.Sprintf("%s:%d", ip.String(), port)
	case 0x03:
		if _, err := io.ReadFull(conn, buf[:1]); err != nil {
			return
		}
		domainLen := int(buf[0])
		if _, err := io.ReadFull(conn, buf[:domainLen]); err != nil {
			return
		}
		domain := string(buf[:domainLen])
		var portBuf [2]byte
		if _, err := io.ReadFull(conn, portBuf[:]); err != nil {
			return
		}
		port := int(portBuf[0])<<8 | int(portBuf[1])
		destAddr = fmt.Sprintf("%s:%d", domain, port)
	default:
		conn.Write([]byte{0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		return
	}

	destConn, err := net.DialTimeout("tcp", destAddr, 3*time.Second)
	if err != nil {
		conn.Write([]byte{0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0})
		return
	}
	defer destConn.Close()

	conn.Write([]byte{0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0})

	done := make(chan struct{}, 2)
	go func() {
		io.Copy(destConn, conn)
		done <- struct{}{}
	}()
	go func() {
		io.Copy(conn, destConn)
		done <- struct{}{}
	}()
	<-done
}

func TestSetOutboundProxy_SOCKS5(t *testing.T) {
	targetSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("HELLO_FROM_TARGET"))
	}))
	defer targetSrv.Close()

	targetURL, _ := url.Parse(targetSrv.URL)

	socksAddr, cleanup := startTestSOCKS5Server(t)
	defer cleanup()

	proxyURL := "socks5://" + socksAddr
	if err := SetOutboundProxy(proxyURL); err != nil {
		t.Fatalf("SetOutboundProxy(%q) failed: %v", proxyURL, err)
	}
	defer SetOutboundProxy("")

	dialer := NewDialer(3 * time.Second)
	conn, err := dialer.DialContext(context.Background(), "tcp", targetURL.Host)
	if err != nil {
		t.Fatalf("DialContext via SOCKS5 failed: %v", err)
	}
	defer conn.Close()

	req := fmt.Sprintf("GET / HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n", targetURL.Host)
	if _, err := conn.Write([]byte(req)); err != nil {
		t.Fatalf("write request: %v", err)
	}

	respBuf := make([]byte, 1024)
	n, err := conn.Read(respBuf)
	if err != nil && err != io.EOF {
		t.Fatalf("read response: %v", err)
	}
	respStr := string(respBuf[:n])
	if !containsStr(respStr, "HELLO_FROM_TARGET") {
		t.Fatalf("expected response to contain HELLO_FROM_TARGET, got: %s", respStr)
	}
}

func TestSetOutboundProxy_Clear(t *testing.T) {
	if err := SetOutboundProxy(""); err != nil {
		t.Fatalf("SetOutboundProxy(\"\") failed: %v", err)
	}
	if GetOutboundProxy() != "" {
		t.Fatalf("expected empty proxy, got: %s", GetOutboundProxy())
	}
}

func TestSetOutboundProxy_Invalid(t *testing.T) {
	err := SetOutboundProxy("invalid-scheme://foo:bar")
	if err == nil {
		t.Fatalf("expected error on invalid scheme, got nil")
	}
}

func containsStr(s, sub string) bool {
	return len(s) >= len(sub) && (s == sub || (len(s) > len(sub) && (s[:len(sub)] == sub || containsStr(s[1:], sub))))
}
