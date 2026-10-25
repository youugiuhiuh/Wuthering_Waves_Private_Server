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

// PerformCombinedProbe 在单个 TCP + TLS 连接上复合完成 TLS1.3/PQ 判断与 HTTP 响应头审查，杜绝 2 次重复握手开销
func PerformCombinedProbe(ctx context.Context, domain string, targetIP string, timeout time.Duration) (*TLSResult, error) {
	if timeout <= 0 {
		timeout = 3 * time.Second
	}

	result := &TLSResult{
		Domain: domain,
		IP:     targetIP,
	}

	var capturedState *utls.ConnectionState
	var capturedGroup utls.CurveID

	transport := &http.Transport{
		DialTLSContext: func(dialCtx context.Context, network, addr string) (net.Conn, error) {
			connectAddr := addr
			if targetIP != "" {
				_, port, err := net.SplitHostPort(addr)
				if err != nil {
					port = "443"
				}
				connectAddr = net.JoinHostPort(targetIP, port)
			}

			dialer := NewDialer(timeout)
			rawConn, err := dialer.DialContext(dialCtx, "tcp", connectAddr)
			if err != nil {
				return nil, err
			}

			config := &utls.Config{
				ServerName:         domain,
				InsecureSkipVerify: true,
				MinVersion:         utls.VersionTLS13,
				MaxVersion:         utls.VersionTLS13,
				NextProtos:         PickALPNProfile(),
			}

			helloID := PickClientHelloID()
			uConn := utls.UClient(rawConn, config, helloID)
			uConn.SetDeadline(time.Now().Add(timeout))

			if err := uConn.Handshake(); err != nil {
				uConn.Close()
				return nil, err
			}

			state := uConn.ConnectionState()
			capturedState = &state
			if uConn.HandshakeState.ServerHello != nil {
				capturedGroup = uConn.HandshakeState.ServerHello.ServerShare.Group
			}

			remoteAddr := uConn.RemoteAddr().String()
			ip, _, _ := net.SplitHostPort(remoteAddr)
			if ip != "" {
				result.IP = ip
			}

			return uConn, nil
		},
		DisableKeepAlives: true,
		ForceAttemptHTTP2: true,
	}

	client := &http.Client{
		Transport: transport,
		Timeout:   timeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}

	reqURL := "https://" + domain + "/"
	req, err := http.NewRequestWithContext(ctx, "HEAD", reqURL, nil)
	if err != nil {
		result.Error = err.Error()
		return result, err
	}
	req.Header.Set("User-Agent", PickUserAgent())

	resp, err := client.Do(req)
	if err != nil {
		// 某些极端服务器拒绝 HEAD，回退尝试 GET
		getReq, getErr := http.NewRequestWithContext(ctx, "GET", reqURL, nil)
		if getErr == nil {
			getReq.Header.Set("User-Agent", PickUserAgent())
			if getResp, getDoErr := client.Do(getReq); getDoErr == nil {
				resp = getResp
				err = nil
			}
		}
	}

	// 记录握手解析结果
	if capturedState != nil {
		result.TLSVersion = capturedState.Version
		result.ALPN = capturedState.NegotiatedProtocol
		result.CertChainLen = len(capturedState.PeerCertificates)
		totalBytes := 0
		for _, cert := range capturedState.PeerCertificates {
			totalBytes += len(cert.Raw)
		}
		result.TotalCertBytes = totalBytes
		result.KeyGroup = capturedGroup
		result.IsPQ = IsPQGroup(capturedGroup)
		result.HandshakeOK = true
	}

	if err != nil {
		result.Error = err.Error()
		return result, err
	}
	defer resp.Body.Close()

	valid, reason := EvaluateHTTPResponse(domain, resp.StatusCode, resp.Header)
	if !valid {
		result.Error = reason
		return result, fmt.Errorf("%s", reason)
	}

	return result, nil
}

// PerformTLSHandshake 发起标准的单次 TLS 1.3 握手
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

// ----------------------------------------------------------------------------
// 暂停中（Hold）：等 2028 之后 PQ 普及再启用
// ----------------------------------------------------------------------------
// 本分支 feature/pq-min-cert 于 2026-10-25 从 main 切出后按计划暂停。
// 当时决定先不启用：PQ 域名只占公网的 15.1%（已剔除 Cloudflare），而这个下限过滤器
// 会再砍掉 US.pb 的 4.9%、SG.pb 的 11.6%，代价大于收益。
//
// 触发重审的条件（任一成立就该回来看一眼）：
//  1. 公网 PQ 占比显著上升（原判断是 OpenSSL 3.4+ / Ubuntu 24.04+ / Debian 13 铺开，
//     预计 2027~2028）。用 `sni_tester` 重扫 Tranco Top 10 万跑一次就能量化。
//  2. 短链 PQ 域名（证书链 < 3500 字节）在 PQ 池里占比超过 20%，说明可用池要被砍太多，
//     那时该换思路（比如 PQ 池只对长链域名下发 mldsa65Seed，而不是直接踢出池子）。
//  3. XTLS/REALITY 上游改掉 3499 字节固定假凭证的做法（届时 MinCertBytes 直接失效）。
//
// 启用时要连带做的事：
//  - 重新生成全部 .pb，让短链域名真的落到 classic_domains。
//  - 核对 rust/aegis/src/core/xray/config.rs 里下发 mldsa65Seed 的判据：
//    目前是 `enable_pq && !REALITY_PQ_SEED.is_empty()`，没有链长信息，
//    所以本分支的过滤是唯一的防线，别只 merge 一边。
//  - 别忘了 main 上 archive/pre-2aa473b-reset 里的其余 PQ 实现（双池 selector、
//    proto 精简、两阶段管道等），它们也随这次暂停一起被归档了。
// ----------------------------------------------------------------------------

const (
	// MaxCertChainDepth REALITY 凭证链最大深度：终端证书 + 最多2张中间CA，严禁4层及以上的超长链
	MaxCertChainDepth = 3
	// MaxCertRecordBytes REALITY 凭证总大小安全阈值：Xray 核心内部硬编码 8192 字节缓冲区，限制在 5500 预留充足裕量
	MaxCertRecordBytes = 5500
	// MinCertBytes 开启 ML-DSA-65 的 REALITY 目标下限：REALITY 的假凭证固定 3499 字节，
	// 服务端会把自身 Certificate record padding 到与目标站等长，目标链更短则 padding < 0 直接握手失败。
	// 已知实测反例：annsummers.com 2363B、accio.ai 2287B、acronis.com 3030B 均为短链 PQ 站。
	MinCertBytes = 3500
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

// SupportsRealityPQ 该目标能否进后量子池：链太短时 REALITY 装不下 3499 字节的 ML-DSA-65 假凭证，
// 只能降级为普通（不开启 mldsa65Seed）的经典目标。
//
// shortcut: 暂未启用（见上方「暂停中」说明），等 PQ 普及后重新评估；
// 若将来改为「PQ 池内按需开关 mldsa65Seed」，这个布尔判断要拆成两个维度。
func (r *TLSResult) SupportsRealityPQ() bool {
	return r.IsPQ && r.TotalCertBytes >= MinCertBytes
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
		// 单次探测禁用连接复用，避免连接池出现 idle channel 延迟脏数据
		DisableKeepAlives: true,
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
