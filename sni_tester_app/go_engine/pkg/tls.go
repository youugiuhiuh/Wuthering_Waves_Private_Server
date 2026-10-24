package pkg

import (
	"context"
	crand "crypto/rand"
	"crypto/tls"
	"fmt"
	"math/big"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	utls "github.com/refraction-networking/utls"
)

var tlsCache sync.Map

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

const CurveSecP256r1MLKEM768 utls.CurveID = 4589

func IsPQGroup(group utls.CurveID) bool {
	return group == utls.X25519MLKEM768 ||
		group == utls.X25519Kyber768Draft00 ||
		group == CurveSecP256r1MLKEM768
}

func IsValidKeyGroup(group utls.CurveID) bool {
	return group == utls.X25519 ||
		group == utls.X25519MLKEM768 ||
		group == utls.X25519Kyber768Draft00 ||
		group == CurveSecP256r1MLKEM768
}

func PerformTLSHandshake(domain string, targetIP string, tlsTimeout time.Duration, needTLS13 bool) (*TLSResult, error) {
	result := &TLSResult{
		Domain: domain,
		IP:     targetIP,
	}

	dialer := NewDialer(5 * time.Second)
	addr := net.JoinHostPort(targetIP, "443")
	rawConn, err := dialer.DialContext(context.Background(), "tcp", addr)
	if err != nil {
		result.Error = err.Error()
		return result, err
	}

	alpn := PickALPNProfile()
	config := &utls.Config{
		ServerName:         domain,
		InsecureSkipVerify: true,
		MinVersion:         utls.VersionTLS12,
		MaxVersion:         utls.VersionTLS13,
		NextProtos:         alpn,
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
		result.IsPQ = IsPQGroup(result.KeyGroup)
	}
	result.CertChainLen = len(state.PeerCertificates)
	totalBytes := 0
	for _, cert := range state.PeerCertificates {
		totalBytes += len(cert.Raw)
	}
	result.TotalCertBytes = totalBytes
	result.HandshakeOK = true

	remoteAddr := uConn.RemoteAddr().String()
	ip, _, _ := net.SplitHostPort(remoteAddr)
	if ip != "" {
		result.IP = ip
	}

	return result, nil
}

const (
	// MaxCertChainDepth REALITY 凭证链最大深度：终端证书 + 最多2张中间CA，严禁4层及以上的超长链
	MaxCertChainDepth = 3
	// MaxCertRecordBytes REALITY 凭证总大小安全阈值：Xray 核心内部硬编码 8192 字节缓冲区，限制在 5500 预留充足裕量
	MaxCertRecordBytes = 5500
)

// ValidateCertLimits 校验目标凭证链深度与总长度，防止触发 Xray REALITY 8192 字节硬限制
func ValidateCertLimits(result *TLSResult) (bool, string) {
	if result.CertChainLen > MaxCertChainDepth {
		return false, fmt.Sprintf("Certificate chain too long (%d > %d)", result.CertChainLen, MaxCertChainDepth)
	}
	if result.TotalCertBytes > MaxCertRecordBytes {
		return false, fmt.Sprintf("Certificate total size too large (%d > %d bytes, exceeds REALITY safe limit)", result.TotalCertBytes, MaxCertRecordBytes)
	}
	return true, ""
}

func ValidateDomain(result *TLSResult) (bool, string) {
	if result.TLSVersion != utls.VersionTLS13 {
		return false, "TLS 1.3 required"
	}

	if !IsValidKeyGroup(result.KeyGroup) {
		return false, "X25519-based key exchange required"
	}

	// 凭证规范审查：排除畸形超长凭证链与大体积证书（防 Xray 8192 缓冲区溢出）
	if valid, reason := ValidateCertLimits(result); !valid {
		return false, reason
	}

	if result.ALPN != "h2" && !CheckH3Support(result.Domain, result.IP) {
		return false, "Neither H2 nor H3 support detected"
	}

	// 深入验证 HTTP 响应标头与重定向行为（排除套皮 CDN 与跨域 301 站点）
	httpValid, httpReason := ValidateHTTP(result.Domain, result.IP, 2*time.Second)
	if !httpValid {
		return false, httpReason
	}

	return true, "Validated (H2/HTTP)"
}

func CheckH3Support(domain string, targetIP string) bool {
	h3Timeout := 1500 * time.Millisecond
	transport := &http.Transport{
		TLSClientConfig: &tls.Config{
			ServerName:         domain,
			InsecureSkipVerify: true,
			NextProtos:         PickALPNProfile(),
		},
		DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
			connectAddr := addr
			if targetIP != "" {
				_, port, _ := net.SplitHostPort(addr)
				connectAddr = net.JoinHostPort(targetIP, port)
			}
			return NewDialer(h3Timeout).DialContext(ctx, "tcp", connectAddr)
		},
		ForceAttemptHTTP2: true,
	}
	client := &http.Client{Transport: transport, Timeout: h3Timeout}

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
	if cached, ok := tlsCache.Load(cacheKey); ok {
		return cached.(*TLSResult)
	}

	result, _ := PerformTLSHandshake(domain, ip, tlsTimeout, needTLS13)
	tlsCache.Store(cacheKey, result)
	return result
}
