package pkg

import (
	"context"
	crand "crypto/rand"
	"crypto/tls"
	"math/big"
	"net"
	"net/http"
	"strings"
	"time"

	utls "github.com/refraction-networking/utls"
)

var tlsCache = NewLRU[string, *TLSResult](50000)

func randIndex(n int) int {
	if n <= 1 {
		return 0
	}
	max := big.NewInt(int64(n))
	v, err := crand.Int(crand.Reader, max)
	if err != nil {
		return 0
	}
	return int(v.Int64())
}

func PickClientHelloID() utls.ClientHelloID {
	return ClientHelloProfiles[randIndex(len(ClientHelloProfiles))]
}

func PickALPNProfile() []string {
	return ALPNProfiles[randIndex(len(ALPNProfiles))]
}

func PickUserAgent() string {
	return UserAgentPool[randIndex(len(UserAgentPool))]
}

func IsValidKeyGroup(group utls.CurveID) bool {
	return group == utls.X25519 ||
		group == utls.X25519MLKEM768 ||
		group == utls.X25519Kyber768Draft00
}

func PerformTLSHandshake(domain string, targetIP string, tlsTimeout time.Duration, needTLS13 bool) (*TLSResult, error) {
	result := &TLSResult{
		Domain: domain,
		IP:     targetIP,
	}

	dialer := &net.Dialer{Timeout: 5 * time.Second}
	addr := net.JoinHostPort(targetIP, "443")
	rawConn, err := dialer.DialContext(context.Background(), "tcp", addr)
	if err != nil {
		result.Error = err.Error()
		return result, err
	}

	alpn := PickALPNProfile()
	config := &utls.Config{
		ServerName: domain,
		MinVersion: utls.VersionTLS12,
		MaxVersion: utls.VersionTLS13,
		NextProtos: alpn,
	}
	if needTLS13 {
		config.MinVersion = utls.VersionTLS13
	}

	helloID := PickClientHelloID()
	uConn := utls.UClient(rawConn, config, helloID)
	defer uConn.Close()
	uConn.SetDeadline(time.Now().Add(tlsTimeout))

	if err := uConn.Handshake(); err != nil {
		result.Error = err.Error()
		return result, err
	}

	state := uConn.ConnectionState()
	hs := uConn.HandshakeState

	result.TLSVersion = state.Version
	result.ALPN = state.NegotiatedProtocol
	if hs.ServerHello != nil {
		result.KeyGroup = hs.ServerHello.ServerShare.Group
	}
	result.HandshakeOK = true

	remoteAddr := uConn.RemoteAddr().String()
	ip, _, _ := net.SplitHostPort(remoteAddr)
	if ip != "" {
		result.IP = ip
	}

	return result, nil
}

func ValidateDomain(result *TLSResult) (bool, string) {
	if result.TLSVersion != utls.VersionTLS13 {
		return false, "TLS 1.3 required"
	}

	if !IsValidKeyGroup(result.KeyGroup) {
		return false, "X25519-based key exchange required"
	}

	if result.ALPN == "h2" {
		return true, "Validated (H2)"
	}

	h3Supported := CheckH3Support(result.Domain, result.IP)
	if h3Supported {
		return true, "Validated (H3)"
	}

	return false, "Neither H2 nor H3 support detected"
}

func CheckH3Support(domain string, targetIP string) bool {
	transport := &http.Transport{
		TLSClientConfig: &tls.Config{
			ServerName: domain,
			NextProtos: PickALPNProfile(),
		},
		DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
			connectAddr := addr
			if targetIP != "" {
				_, port, _ := net.SplitHostPort(addr)
				connectAddr = net.JoinHostPort(targetIP, port)
			}
			return (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, "tcp", connectAddr)
		},
		ForceAttemptHTTP2: true,
	}
	client := &http.Client{Transport: transport, Timeout: 8 * time.Second}

	req, err := http.NewRequest("HEAD", "https://"+domain, nil)
	if err != nil {
		return false
	}
	req.Header.Set("User-Agent", PickUserAgent())
	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()

	altSvc := resp.Header.Get("Alt-Svc")
	return strings.Contains(altSvc, "h3")
}

func GetCachedTLS(domain, ip string, tlsTimeout time.Duration, needTLS13 bool) *TLSResult {
	cacheKey := domain + ":" + ip
	if cached, ok := tlsCache.Get(cacheKey); ok {
		return cached
	}

	result, _ := PerformTLSHandshake(domain, ip, tlsTimeout, needTLS13)
	tlsCache.Set(cacheKey, result)
	return result
}
