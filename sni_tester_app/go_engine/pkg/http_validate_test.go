package pkg

import (
	"net/http"
	"testing"
)

func TestEvaluateHTTPHeaders(t *testing.T) {
	tests := []struct {
		name       string
		domain     string
		statusCode int
		headers    map[string]string
		wantValid  bool
		wantReason string
	}{
		{
			name:       "Normal 200 OK nginx",
			domain:     "example.com",
			statusCode: 200,
			headers: map[string]string{
				"Server": "nginx",
			},
			wantValid:  true,
			wantReason: "",
		},
		{
			name:       "Normal 404 Not Found (valid target)",
			domain:     "example.com",
			statusCode: 404,
			headers: map[string]string{
				"Server": "Apache",
			},
			wantValid:  true,
			wantReason: "",
		},
		{
			name:       "Cloudflare server header",
			domain:     "example.com",
			statusCode: 200,
			headers: map[string]string{
				"Server": "cloudflare",
			},
			wantValid:  false,
			wantReason: "Incompatible CDN (cloudflare)",
		},
		{
			name:       "Cloudflare cf-ray header on Render or SaaS",
			domain:     "expresobrasilia.com",
			statusCode: 200,
			headers: map[string]string{
				"Server": "Render",
				"Cf-Ray": "a4f6639449c0b8b6-SJC",
			},
			wantValid:  false,
			wantReason: "Incompatible CDN (cf-ray)",
		},
		{
			name:       "Cloudflare cf-cache-status header",
			domain:     "example.com",
			statusCode: 200,
			headers: map[string]string{
				"Cf-Cache-Status": "DYNAMIC",
			},
			wantValid:  false,
			wantReason: "Incompatible CDN (cloudflare proxy)",
		},
		{
			name:       "Hostinger CDN hcdn header",
			domain:     "primeinside.co.uk",
			statusCode: 200,
			headers: map[string]string{
				"Server": "hcdn",
			},
			wantValid:  false,
			wantReason: "Incompatible CDN (hcdn)",
		},
		{
			name:       "301 Redirect to www is allowed for REALITY",
			domain:     "example.com",
			statusCode: 301,
			headers: map[string]string{
				"Location": "https://www.example.com/",
			},
			wantValid:  true,
			wantReason: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			header := make(http.Header)
			for k, v := range tt.headers {
				header.Set(k, v)
			}

			valid, reason := EvaluateHTTPResponse(tt.domain, tt.statusCode, header)
			if valid != tt.wantValid {
				t.Errorf("EvaluateHTTPResponse() valid = %v, want %v (reason: %q)", valid, tt.wantValid, reason)
			}
			if !tt.wantValid && tt.wantReason != "" && reason != tt.wantReason {
				t.Errorf("EvaluateHTTPResponse() reason = %q, want %q", reason, tt.wantReason)
			}
		})
	}
}
