package pkg

import (
	"strings"
	"testing"
)

// RED 测试: 确保 DefaultDNSServers 中完全不包含任何中国大陆 DNS
func TestDefaultDNSServers_NoChinaDNS(t *testing.T) {
	chinaKeywords := []string{
		"alidns", "doh.pub", "dot.pub", "360.cn", "dnspod",
		"114.114", "223.5.5", "223.6.6", "119.29", "180.76",
		"180.184", "1.2.4.8", "210.2.4.8", "117.50", "120.53", "1.12.12",
	}

	for _, s := range DefaultDNSServers.DoH {
		for _, kw := range chinaKeywords {
			if strings.Contains(s, kw) {
				t.Errorf("DefaultDNSServers.DoH contains China DNS %q (keyword: %q)", s, kw)
			}
		}
	}

	for _, s := range DefaultDNSServers.DoT {
		for _, kw := range chinaKeywords {
			if strings.Contains(s, kw) {
				t.Errorf("DefaultDNSServers.DoT contains China DNS %q (keyword: %q)", s, kw)
			}
		}
	}

	for _, s := range DefaultDNSServers.UDP {
		for _, kw := range chinaKeywords {
			if strings.Contains(s, kw) {
				t.Errorf("DefaultDNSServers.UDP contains China DNS %q (keyword: %q)", s, kw)
			}
		}
	}
}

// RED 测试: 确保 ResolveFast 测速列表中完全不包含中国大陆 DNS
func TestResolveFast_NoChinaDNS(t *testing.T) {
	chinaIPs := map[string]bool{
		"223.5.5.5":       true,
		"119.29.29.29":    true,
		"180.184.1.1":     true,
		"114.114.114.114": true,
	}

	for _, s := range FastDNSServers {
		if chinaIPs[s] {
			t.Errorf("FastDNSServers contains China DNS IP %q", s)
		}
	}
}
