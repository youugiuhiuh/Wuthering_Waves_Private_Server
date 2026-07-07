# PR-5: Feature Parity — Xray JSON, HTML page, Headers, UA

**Goal:** Feature-complete subscription server comparable to Marzban/Hiddify — Xray JSON output, browser HTML info page, standard subscription headers, enriched UA detection.

**Tech Stack:** Go (chi router, html/template, encoding/json)

## Global Constraints
- All Go changes must pass `go fmt ./... && go build ./... && go vet ./... && go test ./...`
- Xray JSON must be valid JSON
- HTML page should be lightweight (no external deps)
- Headers: `Subscription-Userinfo`, `Profile-Update-Interval`, `Profile-Title`, `Support-Url`, `Profile-Web-Page-Url`

---

### Task 1: Add Xray JSON output format

**Files:**
- Create: `tools/sub-server/format/xray.go`
- Create: `tools/sub-server/format/xray_test.go`
- Modify: `tools/sub-server/handler/subscription.go` (add "xray" case)

Xray JSON format mirrors sing-box but uses Xray-core's JSON structure (outbounds with `protocol`, `settings`, `streamSettings`, etc.). 

The handler needs an "xray" case added in the switch:
```go
case "xray":
    output, err = format.ToXrayJSON(configs)
```

And in `detectFormat`, add xray tools:
```go
xray := []string{"xray", "x-ui", "3x-ui", "nekobox", "v2rayng"}
for _, kw := range xray {
    if strings.Contains(uaLower, kw) {
        return "xray"
    }
}
```

### Task 2: Add HTML info page + subscription headers + UA enrichment

**Files:**
- Create: `tools/sub-server/handler/page.go`
- Modify: `tools/sub-server/handler/subscription.go`

HTML info page: Embedded template showing proxy list with protocol/transport/tag, basic HTML/CSS.

Subscription headers in `writeResponse`:
- `Subscription-Userinfo`: upload/download/total/expiry bytes
- `Profile-Update-Interval`: seconds
- `Profile-Title`: base64-encoded title
- `Support-Url`, `Profile-Web-Page-Url`
