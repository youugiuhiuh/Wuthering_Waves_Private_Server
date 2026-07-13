package pkg

import (
	"bytes"
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"math"
	"math/rand"
	"net"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/miekg/dns"
	"golang.org/x/time/rate"
)

// DNS caching
var DnsCache sync.Map
var DnsPrefetchCache sync.Map
var DnsPrefetchQueue = make(chan string, 500)

// DNSRateLimiter controls DNS query rate
type DNSRateLimiter struct {
	globalLimiter    *rate.Limiter
	providerLimiters map[DNSProvider]*rate.Limiter
	semaphore        chan struct{}
	providerMapUDP   map[string]DNSProvider
	providerMapDoH   map[string]DNSProvider
}

func NewDNSRateLimiter() *DNSRateLimiter {
	return &DNSRateLimiter{
		globalLimiter: rate.NewLimiter(rate.Limit(dnsGlobalLimit), dnsBurstSize),
		providerLimiters: map[DNSProvider]*rate.Limiter{
			ProviderAliyunDoH: rate.NewLimiter(rate.Limit(dnsAliyunDoHLimit), dnsBurstSize),
			ProviderAliyunUDP: rate.NewLimiter(rate.Limit(dnsAliyunUDPLimit), dnsBurstSize),
			ProviderTencent:   rate.NewLimiter(rate.Limit(dnsTencentLimit), dnsBurstSize),
			ProviderDomestic:  rate.NewLimiter(rate.Limit(dnsDomesticLimit), dnsBurstSize),
			ProviderGlobal:    rate.NewLimiter(rate.Limit(dnsInternationalLimit), dnsBurstSize),
		},
		semaphore:      make(chan struct{}, dnsMaxConcurrent),
		providerMapUDP: DNSProviderMapUDP,
		providerMapDoH: DNSProviderMapDoH,
	}
}

func (r *DNSRateLimiter) Acquire(ctx context.Context, server string, isDoHOrDoT bool) (func(), error) {
	select {
	case r.semaphore <- struct{}{}:
	case <-ctx.Done():
		return nil, ctx.Err()
	}

	if err := r.globalLimiter.Wait(ctx); err != nil {
		<-r.semaphore
		return nil, err
	}

	provider := r.getProvider(server, isDoHOrDoT)
	if limiter, ok := r.providerLimiters[provider]; ok {
		if err := limiter.Wait(ctx); err != nil {
			<-r.semaphore
			return nil, err
		}
	}

	return func() { <-r.semaphore }, nil
}

func (r *DNSRateLimiter) getProvider(server string, isDoHOrDoT bool) DNSProvider {
	var providerMap map[string]DNSProvider
	if isDoHOrDoT {
		providerMap = r.providerMapDoH
	} else {
		providerMap = r.providerMapUDP
	}

	ip := server
	if strings.Contains(server, ":") {
		host, _, err := net.SplitHostPort(server)
		if err == nil {
			ip = host
		}
	}

	for prefix, provider := range providerMap {
		if strings.HasPrefix(ip, prefix) {
			return provider
		}
	}

	return ProviderGlobal
}

func (r *DNSRateLimiter) TryAcquire() bool {
	select {
	case r.semaphore <- struct{}{}:
		if r.globalLimiter.Allow() {
			return true
		}
		<-r.semaphore
		return false
	default:
		return false
	}
}

var DnsRateLimiterInstance = NewDNSRateLimiter()

var DnsHealthMap sync.Map

var dnsRng = rand.New(rand.NewSource(time.Now().UnixNano()))
var dnsRngMu sync.Mutex

func calcDnsWeight(h *DnsHealth) float64 {
	if h == nil {
		return 1.0 / dnsHealthEpsilon
	}
	total := float64(h.SuccessCount + h.FailCount)
	if total == 0 {
		return 1.0 / dnsHealthEpsilon
	}
	base := float64(h.SuccessCount) / (total + dnsHealthEpsilon)
	if h.ConsecutiveFail >= dnsMaxConsecutiveFail {
		decay := math.Pow(dnsWeightDecay, float64(h.ConsecutiveFail-dnsMaxConsecutiveFail+1))
		base *= decay
	}
	if base < dnsMinWeight {
		base = dnsMinWeight
	}
	return base
}

func updateDnsHealth(server string, success bool) {
	h, _ := DnsHealthMap.LoadOrStore(server, &DnsHealth{})
	hh := h.(*DnsHealth)
	if success {
		hh.ConsecutiveFail = 0
		hh.SuccessCount++
		newWeight := hh.Weight * dnsRecoveryBoost
		if newWeight > 1.0 {
			newWeight = 1.0
		}
		hh.Weight = newWeight
	} else {
		hh.ConsecutiveFail++
		hh.FailCount++
		hh.Weight = calcDnsWeight(hh)
	}
	DnsHealthMap.Store(server, hh)
}

func selectWeightedServers(servers []string, count int) []string {
	type weightedServer struct {
		server string
		weight float64
	}
	ws := make([]weightedServer, 0, len(servers))
	var totalWeight float64
	for _, s := range servers {
		h, ok := DnsHealthMap.Load(s)
		var w float64
		if ok {
			w = calcDnsWeight(h.(*DnsHealth))
		} else {
			w = 1.0 / dnsHealthEpsilon
		}
		ws = append(ws, weightedServer{s, w})
		totalWeight += w
	}
	if totalWeight <= 0 {
		return servers[:count]
	}
	selected := make([]string, 0, count)
	used := make(map[string]bool)
	for len(selected) < count && len(selected) < len(servers) {
		dnsRngMu.Lock()
		r := dnsRng.Float64() * totalWeight
		dnsRngMu.Unlock()
		cumulative := 0.0
		for _, w := range ws {
			if used[w.server] {
				continue
			}
			cumulative += w.weight
			if r <= cumulative {
				selected = append(selected, w.server)
				used[w.server] = true
				totalWeight -= w.weight
				break
			}
		}
	}
	for _, s := range servers {
		if len(selected) >= count {
			break
		}
		if !used[s] {
			selected = append(selected, s)
			used[s] = true
		}
	}
	return selected
}

var isShuttingDown atomic.Bool

func SetShuttingDown(v bool) {
	isShuttingDown.Store(v)
}

func ShuffleStrings(s []string) {
	for i := len(s) - 1; i > 0; i-- {
		j := randIndex(i + 1)
		s[i], s[j] = s[j], s[i]
	}
}

func resolveWithUDP(ctx context.Context, domain string) ([]string, error) {
	msg := new(dns.Msg)
	msg.SetQuestion(dns.Fqdn(domain), dns.TypeA)

	servers := make([]string, len(DefaultDNSServers.UDP))
	copy(servers, DefaultDNSServers.UDP)
	ShuffleStrings(servers)

	baseTimeout := TimeoutCtrl.GetTimeout("dns")
	var lastErr error

	for round := 0; round < dnsRetryRounds; round++ {
		roundServers := selectWeightedServers(servers, dnsMaxServers)
		for _, server := range roundServers {
			if isShuttingDown.Load() {
				return nil, fmt.Errorf("shutting down")
			}

			release, err := DnsRateLimiterInstance.Acquire(ctx, server, false)
			if err != nil {
				lastErr = err
				continue
			}

			c := &dns.Client{
				Timeout: baseTimeout,
				Net:     "udp4",
			}

			start := time.Now()
			in, _, err := c.ExchangeContext(ctx, msg, server+":53")
			elapsed := time.Since(start)
			TimeoutCtrl.Record(elapsed, "dns")

			release()

			if err != nil {
				errStr := err.Error()
				if strings.Contains(errStr, "NXDOMAIN") ||
					strings.Contains(errStr, "no such host") {
					updateDnsHealth(server, false)
					return nil, err
				}
				lastErr = err
				updateDnsHealth(server, false)
				continue
			}

			var ips []string
			for _, rr := range in.Answer {
				if a, ok := rr.(*dns.A); ok {
					ips = append(ips, a.A.String())
				}
			}
			if len(ips) > 0 {
				updateDnsHealth(server, true)
				return ips, nil
			}
		}

		if round < dnsRetryRounds-1 {
			time.Sleep(dnsRetryDelay * time.Duration(round+1))
		}
	}

	return nil, lastErr
}

func lookupHostDoHWire(client *http.Client, endpoint string, name string) ([]string, error) {
	if client == nil {
		client = &http.Client{Timeout: 5 * time.Second}
	}

	msg := new(dns.Msg)
	msg.SetQuestion(dns.Fqdn(name), dns.TypeA)

	packed, err := msg.Pack()
	if err != nil {
		return nil, fmt.Errorf("failed to pack DNS message: %w", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "POST", endpoint, bytes.NewReader(packed))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/dns-message")
	req.Header.Set("Accept", "application/dns-message")

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("DoH HTTP status %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	respMsg := new(dns.Msg)
	if err := respMsg.Unpack(body); err != nil {
		return nil, fmt.Errorf("failed to unpack DNS response: %w", err)
	}

	var ips []string
	for _, rr := range respMsg.Answer {
		if a, ok := rr.(*dns.A); ok {
			ips = append(ips, a.A.String())
		}
	}

	if len(ips) == 0 {
		return nil, fmt.Errorf("no A records in DoH response")
	}
	return ips, nil
}

func lookupHostDoT(server string, name string) ([]string, error) {
	msg := new(dns.Msg)
	msg.SetQuestion(dns.Fqdn(name), dns.TypeA)

	timeout := 5 * time.Second

	conn, err := dns.DialWithTLS("tcp", server, &tls.Config{
		ServerName: strings.Split(server, ":")[0],
		MinVersion: tls.VersionTLS12,
	})
	if err != nil {
		return nil, fmt.Errorf("DoT connection failed: %w", err)
	}
	defer conn.Close()

	conn.SetDeadline(time.Now().Add(timeout))

	if err := conn.WriteMsg(msg); err != nil {
		return nil, fmt.Errorf("DoT write failed: %w", err)
	}

	resp, err := conn.ReadMsg()
	if err != nil {
		return nil, fmt.Errorf("DoT read failed: %w", err)
	}

	var ips []string
	for _, rr := range resp.Answer {
		if a, ok := rr.(*dns.A); ok {
			ips = append(ips, a.A.String())
		}
	}

	if len(ips) == 0 {
		return nil, fmt.Errorf("no A records in DoT response")
	}
	return ips, nil
}

func resolveWithDNS(ctx context.Context, domain string) ([]string, error) {
	if len(DefaultDNSServers.DoH) > 0 {
		dohServers := selectWeightedServers(DefaultDNSServers.DoH, 3)
		for _, server := range dohServers {
			if isShuttingDown.Load() {
				return nil, fmt.Errorf("shutting down")
			}
			start := time.Now()
			ips, err := lookupHostDoHWire(nil, server, domain)
			latency := time.Since(start)
			TimeoutCtrl.Record(latency, "dns")

			if err == nil && len(ips) > 0 {
				updateDnsHealth(server, true)
				return ips, nil
			}
			errStr := ""
			if err != nil {
				errStr = err.Error()
				if strings.Contains(errStr, "NXDOMAIN") || strings.Contains(errStr, "no such host") {
					updateDnsHealth(server, false)
					return nil, err
				}
			}
			updateDnsHealth(server, false)
		}
	}

	if len(DefaultDNSServers.DoT) > 0 {
		dotServers := selectWeightedServers(DefaultDNSServers.DoT, 3)
		for _, server := range dotServers {
			if isShuttingDown.Load() {
				return nil, fmt.Errorf("shutting down")
			}
			start := time.Now()
			ips, err := lookupHostDoT(server, domain)
			latency := time.Since(start)
			TimeoutCtrl.Record(latency, "dns")

			if err == nil && len(ips) > 0 {
				updateDnsHealth(server, true)
				return ips, nil
			}
			errStr := ""
			if err != nil {
				errStr = err.Error()
				if strings.Contains(errStr, "NXDOMAIN") || strings.Contains(errStr, "no such host") {
					updateDnsHealth(server, false)
					return nil, err
				}
			}
			updateDnsHealth(server, false)
		}
	}

	return resolveWithUDP(ctx, domain)
}

func ResolveWithFailover(ctx context.Context, domain string) ([]string, error) {
	return resolveWithDNS(ctx, domain)
}

type TimeoutController struct {
	mu         sync.Mutex
	samples    []float64
	dnsSamples []float64
	tlsSamples []float64
	index      int
	baseDNS    time.Duration
	baseTLS    time.Duration
}

func NewTimeoutController() *TimeoutController {
	return &TimeoutController{
		samples:    make([]float64, 100),
		dnsSamples: make([]float64, 100),
		tlsSamples: make([]float64, 100),
		baseDNS:    2 * time.Second,
		baseTLS:    10 * time.Second,
	}
}

func (t *TimeoutController) Record(duration time.Duration, kind string) {
	t.mu.Lock()
	defer t.mu.Unlock()

	var samples []float64
	switch kind {
	case "dns":
		samples = t.dnsSamples
	case "tls":
		samples = t.tlsSamples
	default:
		samples = t.samples
	}

	samples[t.index] = duration.Seconds()
	t.index = (t.index + 1) % len(samples)
}

func (t *TimeoutController) GetTimeout(kind string) time.Duration {
	t.mu.Lock()
	defer t.mu.Unlock()

	var samples []float64
	var base time.Duration
	switch kind {
	case "dns":
		samples = t.dnsSamples
		base = t.baseDNS
	case "tls":
		samples = t.tlsSamples
		base = t.baseTLS
	default:
		samples = t.samples
		base = t.baseDNS
	}

	sum := 0.0
	count := 0
	for _, s := range samples {
		if s > 0 {
			sum += s
			count++
		}
	}
	if count == 0 {
		return base
	}

	avg := sum / float64(count)

	variance := 0.0
	for _, s := range samples {
		if s > 0 {
			diff := s - avg
			variance += diff * diff
		}
	}
	std := 0.0
	if count > 1 {
		std = variance / float64(count-1)
	}
	std = math.Sqrt(std)

	timeout := avg + 2*std

	minTimeout := base
	maxTimeout := base * 5
	if timeout < minTimeout.Seconds() {
		timeout = minTimeout.Seconds()
	}
	if timeout > maxTimeout.Seconds() {
		timeout = maxTimeout.Seconds()
	}

	return time.Duration(timeout * 1e9)
}

var TimeoutCtrl = NewTimeoutController()
