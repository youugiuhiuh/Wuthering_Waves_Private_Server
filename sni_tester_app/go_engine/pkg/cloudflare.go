package pkg

import (
	"net"
	"strings"
	"sync"
)

var (
	cfNetsOnce sync.Once
	cfNets     []*net.IPNet
)

var rawCloudflareCIDRs = []string{
	"173.245.48.0/20",
	"103.21.244.0/22",
	"103.22.200.0/22",
	"103.31.4.0/22",
	"141.101.64.0/18",
	"108.162.192.0/18",
	"190.93.240.0/20",
	"188.114.96.0/20",
	"197.234.240.0/22",
	"198.41.128.0/17",
	"162.158.0.0/15",
	"104.16.0.0/13",
	"104.24.0.0/14",
	"172.64.0.0/13",
	"131.0.72.0/22",
	"1.1.1.0/24",
	"1.0.0.0/24",
}

func initCloudflareCIDRs() {
	cfNets = make([]*net.IPNet, 0, len(rawCloudflareCIDRs))
	for _, cidr := range rawCloudflareCIDRs {
		_, ipNet, err := net.ParseCIDR(cidr)
		if err == nil {
			cfNets = append(cfNets, ipNet)
		}
	}
}

// IsCloudflareIP 检查 IP 是否属于 Cloudflare Anycast/CDN 网段
func IsCloudflareIP(rawIP string) bool {
	if rawIP == "" {
		return false
	}
	parsed := net.ParseIP(rawIP)
	if parsed == nil {
		return false
	}

	cfNetsOnce.Do(initCloudflareCIDRs)
	for _, ipNet := range cfNets {
		if ipNet.Contains(parsed) {
			return true
		}
	}
	return false
}

// IsCloudflareASN 检查 ASN 或 Org 名称是否归属于 Cloudflare
func IsCloudflareASN(asn uint32, org string) bool {
	if asn == 13335 || asn == 209242 {
		return true
	}
	if org != "" {
		lower := strings.ToLower(org)
		if strings.Contains(lower, "cloudflare") {
			return true
		}
	}
	return false
}

// IsCloudflare 综合判定该目标是否属于 Cloudflare（REALITY 无法借用 Cloudflare 做 SNI）
func IsCloudflare(ip string, asn uint32, org string) bool {
	return IsCloudflareIP(ip) || IsCloudflareASN(asn, org)
}
