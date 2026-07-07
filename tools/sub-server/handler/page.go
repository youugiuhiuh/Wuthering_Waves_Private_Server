package handler

import (
	"html/template"
	"net/http"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

const htmlTemplateStr = `<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Subscription</title>
    <style>
        body { font-family: -apple-system, sans-serif; max-width: 800px; margin: 0 auto; padding: 16px; background: #f5f5f5; }
        h1 { font-size: 1.5em; }
        .proxy { background: white; border-radius: 8px; padding: 12px; margin: 8px 0; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
        .tag { font-weight: 600; font-size: 1.1em; }
        .detail { color: #666; font-size: 0.9em; margin-top: 4px; }
        .badge { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 0.8em; background: #e3f2fd; color: #1565c0; margin-right: 4px; }
        .links { margin-top: 16px; }
        .links a { color: #1976d2; text-decoration: none; margin-right: 12px; }
        .links a:hover { text-decoration: underline; }
    </style>
</head>
<body>
    <h1>Subscription</h1>
    <div class="links">
        <a href="?format=base64">Base64</a>
        <a href="?format=clash">Clash</a>
        <a href="?format=singbox">Sing-box</a>
        <a href="?format=xray">Xray</a>
    </div>
    <div id="proxies">
    {{range .}}
        <div class="proxy">
            <div class="tag">{{.Tag}}</div>
            <div class="detail">
                <span class="badge">{{.Protocol}}</span>
                <span class="badge">{{.Transport}}</span>
                {{.Host}}:{{.Port}}
            </div>
        </div>
    {{end}}
    </div>
</body>
</html>`

var htmlTmpl = template.Must(template.New("sub").Parse(htmlTemplateStr))

func renderHTML(w http.ResponseWriter, configs []*pb.ProxyConfig) error {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	return htmlTmpl.Execute(w, configs)
}
