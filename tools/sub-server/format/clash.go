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
{{- if .Sni}}
    sni: {{.Sni}}
{{- end}}
{{- if .PublicKey}}
    reality-opts:
      public-key: {{.PublicKey}}
      short-id: {{.ShortId}}
{{- end}}
{{- if .Flow}}
    flow: {{.Flow}}
{{- end}}
{{- if ne .Transport "tcp"}}
    network: {{.Transport}}
{{- end}}
{{- if .Path}}
    ws-path: {{.Path}}
{{- end}}
{{end}}
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
