package pkg

import (
	"strings"
	"testing"

	utls "github.com/refraction-networking/utls"
)

func TestValidateDomainCertLimits(t *testing.T) {
	tests := []struct {
		name         string
		result       *TLSResult
		wantValid    bool
		wantErrMatch string
	}{
		{
			name: "Normal 2-tier certificate chain within limits",
			result: &TLSResult{
				Domain:         "example.com",
				TLSVersion:     utls.VersionTLS13,
				KeyGroup:       utls.X25519,
				ALPN:           "h2",
				CertChainLen:   2,
				TotalCertBytes: 2400,
			},
			wantValid:    true,
			wantErrMatch: "",
		},
		{
			name: "Normal 3-tier certificate chain within limits",
			result: &TLSResult{
				Domain:         "example.com",
				TLSVersion:     utls.VersionTLS13,
				KeyGroup:       utls.X25519,
				ALPN:           "h2",
				CertChainLen:   3,
				TotalCertBytes: 3800,
			},
			wantValid:    true,
			wantErrMatch: "",
		},
		{
			name: "4-tier certificate chain exceeds depth limit (e.g. misaplis.com)",
			result: &TLSResult{
				Domain:         "misaplis.com",
				TLSVersion:     utls.VersionTLS13,
				KeyGroup:       utls.X25519,
				ALPN:           "h2",
				CertChainLen:   4,
				TotalCertBytes: 3430,
			},
			wantValid:    false,
			wantErrMatch: "Certificate chain too long",
		},
		{
			name: "Certificate exceeds 5500 bytes (Xray 8192 buffer overflow guard)",
			result: &TLSResult{
				Domain:         "bloated-cert.com",
				TLSVersion:     utls.VersionTLS13,
				KeyGroup:       utls.X25519,
				ALPN:           "h2",
				CertChainLen:   3,
				TotalCertBytes: 6200,
			},
			wantValid:    false,
			wantErrMatch: "Certificate total size too large",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			valid, reason := ValidateCertLimits(tt.result)
			if valid != tt.wantValid {
				t.Errorf("ValidateCertLimits() valid = %v, want %v (reason: %s)", valid, tt.wantValid, reason)
			}
			if !tt.wantValid && tt.wantErrMatch != "" {
				if !strings.Contains(reason, tt.wantErrMatch) {
					t.Errorf("ValidateCertLimits() reason = %q, want match %q", reason, tt.wantErrMatch)
				}
			}
		})
	}
}

// REALITY 开启 ML-DSA-65 时用的假凭证是固定 3499 字节（含 3309 字节签名扩展），
// 服务端会把自己的 Certificate record 用 padding 补到与目标站一样长；
// 目标链太短则 padding < 0，握手当场失败（payload[0]: 22, padding: -N）。
func TestSupportsRealityPQRequiresMinCertBytes(t *testing.T) {
	// 假凭证 3499 + 握手消息头 10 = 3509 明文 → record 3514+17 = 3531，链 DER 需 ≥3499
	if MinCertBytes < 3499 {
		t.Fatalf("MinCertBytes = %d, must be >= 3499 to fit REALITY's 3499-byte ML-DSA-65 fake cert", MinCertBytes)
	}

	tests := []struct {
		name           string
		isPQ           bool
		totalCertBytes int
		want           bool
	}{
		{"PQ with exact minimum", true, MinCertBytes, true},
		{"PQ one byte below minimum", true, MinCertBytes - 1, false},
		{"PQ with short ECDSA chain (e.g. annsummers.com 2363B)", true, 2363, false},
		{"PQ with RSA fullchain", true, 4800, true},
		{"classic long chain is not PQ", false, 4800, false},
		{"classic short chain is not PQ", false, 2363, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := &TLSResult{IsPQ: tt.isPQ, TotalCertBytes: tt.totalCertBytes}
			if got := r.SupportsRealityPQ(); got != tt.want {
				t.Errorf("SupportsRealityPQ() = %v, want %v (isPQ=%v certBytes=%d)", got, tt.want, tt.isPQ, tt.totalCertBytes)
			}
		})
	}
}
