package pkg

import (
	"context"
	"crypto/tls"
	"net"
	"net/http"
	"strings"
	"time"
)

// EvaluateHTTPResponse 纯逻辑判定 HTTP 响应标头是否属于与 REALITY 排斥的 CDN（如 Cloudflare、hcdn 等）
func EvaluateHTTPResponse(domain string, statusCode int, header http.Header) (bool, string) {
	server := strings.ToLower(header.Get("Server"))
	if strings.Contains(server, "cloudflare") {
		return false, "Incompatible CDN (cloudflare)"
	}
	if strings.Contains(server, "hcdn") {
		return false, "Incompatible CDN (hcdn)"
	}
	if header.Get("Cf-Ray") != "" {
		return false, "Incompatible CDN (cf-ray)"
	}
	if header.Get("Cf-Cache-Status") != "" {
		return false, "Incompatible CDN (cloudflare proxy)"
	}

	return true, ""
}

// ValidateHTTP 发起真实的轻量 HTTP 探测，验证目标是否为合格的 REALITY 伪装目标（排除穿马甲的 Cloudflare 与 hcdn）
func ValidateHTTP(domain, targetIP string, timeout time.Duration) (bool, string) {
	if timeout <= 0 {
		timeout = 2 * time.Second
	}

	transport := &http.Transport{
		TLSClientConfig: &tls.Config{
			ServerName:         domain,
			InsecureSkipVerify: true,
			NextProtos:         []string{"h2", "http/1.1"},
		},
		DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
			connectAddr := addr
			if targetIP != "" {
				_, port, err := net.SplitHostPort(addr)
				if err != nil {
					port = "443"
				}
				connectAddr = net.JoinHostPort(targetIP, port)
			}
			return NewDialer(timeout).DialContext(ctx, "tcp", connectAddr)
		},
		ForceAttemptHTTP2: true,
		// 探测为单次短连接验证，禁用连接复用（Keep-Alive），握手完成后立即彻底关闭 TCP 连接，
		// 彻底根除部分迟钝/畸形服务器在连接归还空闲池后延迟发包导致的 "Unsolicited response received on idle HTTP channel" 告警。
		DisableKeepAlives: true,
	}

	client := &http.Client{
		Transport: transport,
		Timeout:   timeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			// 不自动重定向，直接获取第一跳的响应头进行审查
			return http.ErrUseLastResponse
		},
	}

	reqURL := "https://" + domain + "/"
	req, err := http.NewRequest("HEAD", reqURL, nil)
	if err != nil {
		return false, "Failed to create HTTP request: " + err.Error()
	}
	req.Header.Set("User-Agent", PickUserAgent())

	resp, err := client.Do(req)
	if err != nil {
		// 某些极端 Web 服务器拒绝 HEAD，回退尝试轻量 GET
		getReq, getErr := http.NewRequest("GET", reqURL, nil)
		if getErr != nil {
			return false, "HTTP connection failed: " + err.Error()
		}
		getReq.Header.Set("User-Agent", PickUserAgent())
		getResp, getDoErr := client.Do(getReq)
		if getDoErr != nil {
			return false, "HTTP connection failed: " + err.Error()
		}
		resp = getResp
	}
	defer resp.Body.Close()

	valid, reason := EvaluateHTTPResponse(domain, resp.StatusCode, resp.Header)
	if !valid {
		return false, reason
	}

	return true, "HTTP validated"
}
