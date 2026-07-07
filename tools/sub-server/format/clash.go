package format

import (
	"bytes"
	"text/template"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

const clashTemplate = `port: 7890
socks-port: 7891
mode: Rule
log-level: info
external-controller: 127.0.0.1:9090
proxies:
{{range .}}
  - name: "{{.Tag}}"
    type: {{protocolClash .Protocol}}
    server: {{.Host}}
    port: {{.Port}}
{{- if .Uuid}}
    uuid: {{.Uuid}}
{{- end}}
{{- if .Password}}
    password: {{.Password}}
{{- end}}
{{- if .Flow}}
    flow: {{.Flow}}
{{- end}}
{{- if .Sni}}
    sni: {{.Sni}}
{{- end}}
{{- if .Alpn}}
    alpn:
      - {{.Alpn}}
{{- end}}
{{- if .Fingerprint}}
    client-fingerprint: {{.Fingerprint}}
{{- end}}
{{- if .PublicKey}}
    reality-opts:
      public-key: {{.PublicKey}}
      short-id: {{.ShortId}}
{{- end}}
{{- if ne .Transport "tcp"}}
    network: {{.Transport}}
{{- end}}
{{- if eq .Transport "ws"}}
{{- if .Path}}
    ws-path: {{.Path}}
    ws-headers:
      Host: "{{or .HttpHost .Sni .Host}}"
{{- end}}
{{- end}}
{{- if eq .Transport "xhttp"}}
{{- if .Path}}
    http-path: {{.Path}}
{{- end}}
{{- if .HttpHost}}
    http-host: {{.HttpHost}}
{{- end}}
{{- if .Mode}}
    mode: {{.Mode}}
{{- end}}
{{- end}}
{{- if eq .Transport "grpc"}}
{{- if .ServiceName}}
    grpc-service-name: {{.ServiceName}}
{{- end}}
{{- end}}
{{- if .HopPortStart}}
    ports: {{.HopPortStart}}-{{.HopPortEnd}}
{{- end}}
{{end}}

proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - Auto
{{- range .}}
      - "{{.Tag}}"
{{- end}}
  - name: Auto
    type: url-test
    url: http://www.gstatic.com/generate_204
    interval: 300
    tolerance: 50
    proxies:
{{- range .}}
      - "{{.Tag}}"
{{- end}}

rules:
  - GEOIP,CN,DIRECT
  - MATCH,Proxy
`

func protocolClash(proto string) string {
	switch proto {
	case "vless":
		return "vless"
	case "hysteria2", "hy2":
		return "hysteria2"
	case "tuic":
		return "tuic"
	default:
		return proto
	}
}

func ToClashYAML(configs []*pb.ProxyConfig) (string, error) {
	funcMap := template.FuncMap{
		"protocolClash": protocolClash,
	}
	tmpl, err := template.New("clash").Funcs(funcMap).Parse(clashTemplate)
	if err != nil {
		return "", err
	}
	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, configs); err != nil {
		return "", err
	}
	return buf.String(), nil
}
