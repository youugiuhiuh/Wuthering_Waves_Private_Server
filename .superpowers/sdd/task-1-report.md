# Task 1 Report: Sing-box JSON Rewrite

## Summary
Successfully rewrote `tools/sub-server/format/singbox.go` with full transport/REALITY/TLS + DNS/route support and added comprehensive tests.

## Files Changed
- `tools/sub-server/format/singbox.go` — Full rewrite (216 lines)
- `tools/sub-server/format/singbox_test.go` — New test file (109 lines)

## Implementation Details

### New Functions
- `buildSingboxTransport(cfg)` — Handles ws, xhttp, grpc transport types with proper field mapping
- `buildSingboxTLS(cfg)` — Builds TLS section with SNI, UTLS fingerprint, certificate pinning, ALPN, insecure toggle
- `buildSingboxReality(cfg)` — Builds Reality section with public_key, short_id, server_name, short_path, and UTLS fingerprint

### Protocol Support
- **VLESS**: uuid, flow, encryption, transport, reality, TLS (non-reality only)
- **Hysteria2/hy2**: password, hop_port range, obfs with type+password, TLS
- **TUIC**: password, congestion_control, udp_relay_mode, heartbeat, TLS

### Global Config
- **DNS**: HTTPS resolver (1.1.1.1) with local fallback, CN geosite rules
- **Route**: CN geoip/geosite routing to direct outbound
- **Inbound**: TUN interface (172.19.0.1/30) with auto_route

### Deviation from Brief
- Made `short_id` conditional (only set when non-empty) to avoid `"short_id": ""` in JSON output
- Added UTLS fingerprint support to Reality section (`reality.utls.fingerprint`) since Reality configs skip the TLS block and need their own fingerprint

## Test Results
```
=== RUN   TestToSingBox_VLESS_Reality    — PASS
=== RUN   TestToSingBox_Hysteria2         — PASS
=== RUN   TestToSingBox_VLESS_Ws          — PASS
ok  	github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/format
```

## Quality Gates
| Gate | Result |
|------|--------|
| `go fmt ./...` | Pass |
| `go build ./...` | Pass |
| `go vet ./...` | Pass |
| `go test ./... -v` | Pass (3/3) |
| `staticcheck ./...` | Pass (no warnings) |
