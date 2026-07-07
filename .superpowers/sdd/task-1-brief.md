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

