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
