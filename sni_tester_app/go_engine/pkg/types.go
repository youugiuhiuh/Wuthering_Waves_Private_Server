package pkg

import utls "github.com/refraction-networking/utls"

type DNSProvider int

const (
	ProviderAliyunDoH DNSProvider = iota
	ProviderAliyunUDP
	ProviderTencent
	ProviderDomestic
	ProviderGlobal
)

type DnsHealth struct {
	SuccessCount    uint32
	FailCount       uint32
	ConsecutiveFail uint32
	Weight          float64
}

type ASNResult struct {
	ASN uint32
	Org string
}

type DomainResult struct {
	Domain  string
	Success bool
	IP      string
	Country string
	ASN     uint32
	Org     string
	Info    string
}

type ASNInfo struct {
	Org     string
	Country string
	AddedAt int64
}

type SuccessInfo struct {
	Domain   string
	Country  string
	ASN      uint32
	Org      string
	TestedAt int64
}

type BlockedInfo struct {
	Domain   string
	Reason   string
	Code     string
	TestedAt int64
}

type TLSResult struct {
	Domain      string
	IP          string
	TLSVersion  uint16
	ALPN        string
	KeyGroup    utls.CurveID
	HandshakeOK bool
	Error       string
}

type ProgressEvent struct {
	Type     string  `json:"type"`
	Domain   string  `json:"domain"`
	Success  bool    `json:"success"`
	Country  string  `json:"country"`
	IP       string  `json:"ip"`
	Info     string  `json:"error"`
	Progress float64 `json:"progress"`
	Stats    Stats   `json:"stats"`
}

type Stats struct {
	Total      int     `json:"total"`
	Success    int     `json:"success"`
	Failed     int     `json:"fail"`
	Skipped    int     `json:"skipped"`
	RatePerSec float64 `json:"rate_per_sec"`
}

type Result struct {
	Stats Stats
}

type ProgressCallback func(event ProgressEvent)

type ValidationResult = DomainResult
