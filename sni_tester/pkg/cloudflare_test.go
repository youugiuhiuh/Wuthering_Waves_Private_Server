package pkg

import "testing"

func TestIsCloudflareIP(t *testing.T) {
	tests := []struct {
		ip       string
		expected bool
	}{
		// Cloudflare IPs (from our production survey)
		{"104.21.94.56", true},
		{"104.18.24.66", true},
		{"172.67.146.36", true},
		{"172.66.40.214", true},
		{"162.159.198.2", true},
		{"108.162.192.1", true},
		{"198.41.128.5", true},

		// Non-Cloudflare IPs (Fastly, AWS, Google, Independent)
		{"151.101.130.132", false}, // Fastly (endlessm.com)
		{"192.104.182.209", false}, // Independent (timeswv.com)
		{"8.8.8.8", false},         // Google
		{"1.1.1.1", true},          // Cloudflare DNS
		{"1.0.0.1", true},          // Cloudflare DNS
		{"", false},
		{"invalid-ip", false},
	}

	for _, tt := range tests {
		t.Run(tt.ip, func(t *testing.T) {
			got := IsCloudflareIP(tt.ip)
			if got != tt.expected {
				t.Errorf("IsCloudflareIP(%q) = %v; want %v", tt.ip, got, tt.expected)
			}
		})
	}
}

func TestIsCloudflareASN(t *testing.T) {
	tests := []struct {
		asn      uint32
		org      string
		expected bool
	}{
		{13335, "Cloudflare, Inc.", true},
		{209242, "Cloudflare London", true},
		{13335, "", true},
		{0, "CLOUDFLARENET", true},
		{54113, "Fastly, Inc.", false},
		{15169, "Google LLC", false},
		{16509, "Amazon.com, Inc.", false},
		{0, "", false},
	}

	for _, tt := range tests {
		got := IsCloudflareASN(tt.asn, tt.org)
		if got != tt.expected {
			t.Errorf("IsCloudflareASN(%d, %q) = %v; want %v", tt.asn, tt.org, got, tt.expected)
		}
	}
}
