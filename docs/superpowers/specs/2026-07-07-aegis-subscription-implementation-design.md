# Aegis Subscription Feature Implementation Design

> **Date:** 2026-07-07
> **Based on:** `docs/analysis/aegis-subscription-comparison.md`
> **Scope:** 5-phase implementation cycle for `rust/aegis` + `tools/sub-server` subscription system
> **PR Strategy:** Sequential PRs, one per Phase

## Architecture

```
client ──HTTPS──► sub-server (Go / chi router)
                        │
                   gRPC Unix Socket
                        │
                   aegis (Rust / tonic)
                        │
              ┌─────────┴──────────┐
         /etc/wwps/wwps-core/   /etc/wwps/wwps-box/
         *xray*_inbounds.json   hysteria2/tuic.json
```

Retain existing two-process architecture. All format conversion happens in Go (`tools/sub-server/format/`). Rust aggregator (`aggregator.rs`) reads Xray and sing-box config JSONs from disk and populates `ProxyConfig` protobuf. Go gRPC client fetches configs, formats per UA detection, returns to client.

## PR-1: Phase 1 Deployment Fixes (Rust + CI)

**Goal:** aegis can reliably deploy sub-server on amd64 + arm64, with proper systemd dependency and pre-flight gRPC readiness check.

### Files
- `rust/aegis/src/core/subscription/deploy.rs` — systemd unit, readiness check, retry, arch-aware binary name
- `.github/workflows/public-release.yml` — confirm arm64 binary + signature artifacts

### Changes

1. **Systemd unit** (`write_systemd_service`):
   ```
   After=network.target wwps-aegis.service
   BindsTo=wwps-aegis.service
   Requires=wwps-aegis.service
   ```

2. **gRPC readiness check** (`run_deploy` after restart):
   - Poll `/var/run/aegis/sub.sock` existence for up to 30s (backoff 500ms, 1s, 2s, 4s, 8s…)
   - Then attempt a `GetConfigs` gRPC call; if fails, log warning but return success URL anyway (recoverable)

3. **Arch-aware binary download** (`download_binary`):
   - `resolve_binary_name()`: `std::env::consts::ARCH` → `"arm64"` → `"sub-server-arm64"`, else `"sub-server"`
   - Fallback: if arm64 binary download fails (404), retry with `"sub-server"`

4. **Download retry** (`download_binary`):
   - 3 attempts with 2s, 5s, 10s backoff on non-404 HTTP errors or connection failures

5. **Minisign signature optional** (`verify_binary` → `run_deploy`):
   - If sig download fails (404 or empty), skip verification with warning
   - If verification fails, log error but continue deployment (degraded mode)

6. **CI confirm**:
   - Ensure `sub-server-arm64` and `sub-server-arm64.minisig` are attached to release (already done in recent PR)

### Testing
- `cargo test` passes
- Manual: run `/sub setup` on test VPS, verify systemd unit content, verify auto-restart on aegis restart, verify arm64 binary download on arm64 VPS

---

## PR-2: Phase 2 Base64 Subscription Output

**Goal:** Produced VLESS/hysteria2/tuic links work in all clients (Shadowrocket, v2rayNG, NekoBox, etc.)

### Files
- `proto/subscription.proto` — expand `ProxyConfig` message
- `rust/aegis/src/core/subscription/aggregator.rs` — extract new fields from config JSONs
- `tools/sub-server/format/uri.go` — URL encoding + parameter completeness
- `tools/sub-server/format/list.go` — no changes needed if URI is fixed

### 2.1 Proto Expansion (`subscription.proto`)

Add fields to `ProxyConfig`:
```protobuf
message ProxyConfig {
  // existing fields 1-21...
  string fingerprint = 22;      // TLS fingerprint (e.g. "chrome")
  string spx = 23;              // REALITY short path (spiderX)
  string host = 24;             // HTTP Host for WS/XHTTP
  string mode = 25;             // XHTTP mode ("auto" / "packet-up")
  string extra = 26;            // XHTTP extra settings (JSON string)
  string header_type = 27;      // TCP header type ("none" / "http")
  string service_name = 28;     // gRPC service name
  string authority = 29;        // gRPC authority
  bool insecure = 30;           // allowInsecure TLS
  string encryption = 31;       // VLESS encryption ("none" / "aes-128-gcm")
  string server_name = 32;      // TLS SNI override
}
```

Recompile Rust proto (built via `build.rs` with `prost-build` + `tonic-build`) and Go proto (pre-compiled `.pb.go` — regenerate with `protoc`).

### 2.2 Aggregator Field Extraction (`aggregator.rs`)

| Existing/New | Source Field | Target Field |
|---|---|---|
| Bug fix: `host` | `streamSettings.realitySettings.serverName` | `sni` |
| Bug fix: `pin_sha256` | `streamSettings.tlsSettings.certificates[].sha256` or fallback empty | `pin_sha256` |
| New | `streamSettings.realitySettings.shortPath` | `spx` |
| New | `streamSettings.xhttpSettings.mode` | `mode` |
| New | `streamSettings.xhttpSettings.host` | `host` |
| New | `streamSettings.xhttpSettings.extra` (JSON → string) | `extra` |
| New | `streamSettings.tlsSettings.fingerprint` | `fingerprint` |
| New | `streamSettings.tcpSettings.header.type` | `header_type` |
| New | `streamSettings.wsSettings.host` | `host` |
| New | `streamSettings.grpcSettings.serviceName` | `service_name` |
| New | `streamSettings.grpcSettings.authority` | `authority` |
| New | `streamSettings.tlsSettings.allowInsecure` | `insecure` |
| New | `settings.clients[].encryption` | `encryption` |
| New | `streamSettings.tlsSettings.serverName` | `server_name` |

For `host` resolution in Xray aggregator: if `inbound.host` field is missing/empty, try `SystemMonitor::get_public_ip()` or `/etc/wwps/host`, else use `"0.0.0.0"`.

### 2.3 URI Builder Rewrite (`uri.go`)

- Replace raw `fmt.Sprintf` parameter assembly with `net/url.Values` for proper URL encoding
- VLESS URI format:
  ```
  vless://{uuid}@{host}:{port}?encryption={encryption}&security={security}
    &sni={sni}&fp={fingerprint}&pbk={public_key}&sid={short_id}
    &spx={spx}&type={transport}&flow={flow}&mode={mode}&path={path}
    &host={host}&headerType={header_type}&alpn={alpn}&serviceName={service_name}
    #{tag}
  ```
- `security` toggle: if `spx` / `public_key` / `short_id` are present → `reality`, else if TLS fields → `tls`, else none
- hysteria2: `insecure=1` only when `cert_sha256` empty; otherwise use `cert_sha256={hex}`
- TUIC: preserve existing params, add `alpn` (already present)

### Testing
- Rust: cargo test passes
- Go: Add table-driven tests in `uri_test.go` for:
  - VLESS URI with all params URL-encoded
  - VLESS URI with special chars in sni/pbk/sid
  - hysteria2 with hop ports
  - TUIC with congestion control + alpn

---

## PR-3: Phase 3 Clash YAML

**Goal:** Produced Clash YAML works in Clash Verge / Mihomo / Stash.

### Files
- `tools/sub-server/format/clash.go` — rewrite template + add proxy-groups + routing

### 3.1 Transport-Specific Template

Replace flat template with transport-conditional fields:

Clash proxy entry for vless:
```yaml
- name: "{tag}"
  type: vless
  server: {host}
  port: {port}
  uuid: {uuid}
  flow: {flow}
  client-fingerprint: {fingerprint}
  udp: true
```

Conditional blocks:
- `network: xhttp` → `xhttp-opts: { path, mode, host, extra }`
- `network: ws` → `ws-opts: { path, headers: { Host: {host} } }`
- `network: grpc` → `grpc-opts: { grpc-service-name: {service_name} }`
- TLS enabled → `tls: true, alpn: [{alpn}], servername: {server_name or sni}`
- REALITY → `reality-opts: { public-key, short-id, spider-x (spx), server-name (sni), client-fingerprint }`

### 3.2 Proxy Groups + Routing

```
proxy-groups:
  - name: ♻️ 自动选择
    type: url-test
    proxies: [all proxy tags]
    url: http://www.gstatic.com/generate_204
    interval: 300
  - name: 🚀 节点选择
    type: select
    proxies: [♻️ 自动选择, DIRECT]
  - name: 🐟 漏网之鱼
    type: select
    proxies: [🚀 节点选择, DIRECT]

rules:
  - GEOSITE,cn,DIRECT
  - GEOIP,CN,DIRECT
  - MATCH,🚀 节点选择
```

### 3.3 hysteria2 & TUIC

hysteria2 proxy type: `hysteria2`
- `password`, `sni`, `obfs`, `alpn` (h3)
- `ca` and `ca-str` for cert_sha256

TUIC proxy type: `tuic`
- `token`, `congestion_control`, `alpn`, `sni`, `udp_relay_mode`

### Testing
- Add `clash_test.go` with table-driven tests for:
  - VLESS REALITY proxy entry
  - VLESS WS proxy entry
  - Hysteria2 proxy entry
  - TUIC proxy entry
  - Full output with proxy-groups and rules

---

## PR-4: Phase 4 Sing-box JSON

**Goal:** Produced Sing-box JSON works in SFA/SFI / Hiddify / NekoBox.

### Files
- `tools/sub-server/format/singbox.go` — rewrite with full transport, reality, TLS, DNS, route

### 4.1 Outbound Structure

```json
{
  "type": "vless",
  "tag": "tag-name",
  "server": "host",
  "server_port": 443,
  "uuid": "uuid",
  "flow": "flow",
  "packet_encoding": "xudp"
}
```

Conditional objects:
- **transport**: based on `network` → `transport` object with `type` + type-specific opts
- **tls**: if TLS enabled → `{ enabled: true, server_name, insecure, utls: { enabled: true, fingerprint }, certificate: [sha256] }`
- **reality**: if REALITY → `{ enabled: true, public_key, short_id, server_name, spider_x }`

### 4.2 Protocol-Specific

VLESS:
- transport: `tcp` / `ws` / `grpc` / `xhttp` with proper field mapping
- TODO: `xhttp` requires sing-box >= 1.8, specify `max_early_data`, `early_data_header` from `extra`

Hysteria2:
- `password`, `quic: { congestion_control }`, `obfs: { type, password }`
- `hop_interval`, `hop_ports` (string like "10000-20000")

TUIC:
- `password`, `congestion_control`, `udp_relay_mode`, `zero_rtt_handshake`, `heartbeat`

### 4.3 DNS + Route

```json
{
  "dns": {
    "servers": [{"tag": "dns-remote", "address": "https://1.1.1.1/dns-query"}]
  },
  "route": {
    "rules": [
      {"geoip": "cn", "action": "route", "route": "block"},
      {"geosite": "cn", "action": "route", "route": "direct"}
    ],
    "rule_set": [
      {"type": "remote", "tag": "geosite-cn", "url": "https://...", "download_detour": "direct"},
      {"type": "remote", "tag": "geoip-cn", "url": "https://..."}
    ]
  }
}
```

### Testing
- Go: Add `singbox_test.go` with table-driven tests for each protocol + empty configs

---

## PR-5: Phase 5 Feature Parity

**Goal:** Feature-complete subscription server with Xray JSON output, HTML info page, standard headers.

### Files
- `tools/sub-server/format/xray.go` — new file: Xray JSON outbound format
- `tools/sub-server/handler/page.go` — new file: HTML info page
- `tools/sub-server/handler/subscription.go` — add subscription headers, enrich UA detection
- `tools/sub-server/main.go` — register `/page/{token}` or `?format=html`

### 5.1 Xray JSON Output (`format/xray.go`)

Generate Xray outbound JSON array from `ProxyConfig[]`, field mapping following Xray-core `outbounds[].settings` + `streamSettings` format. Supports `?format=xray`.

### 5.2 HTML Info Page (`handler/page.go`)

When UA contains `mozilla` OR `?format=html`:
- Return styled HTML page with:
  - Token identifier (first 4 chars)
  - Config count
  - Expiration date
  - Links to each format: `?format=base64`, `?format=clash`, `?format=singbox`, `?format=xray`, `?format=uri`

### 5.3 Subscription Headers (`handler/subscription.go`)

Add standard subscription headers to every response:
- `Subscription-Userinfo: upload=0; download=0; total=0; expire={unix_timestamp}`
- `Profile-Update-Interval: 60`
- `Profile-Title: {base64("WWPS")}`
- `Support-Url: https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server`

### 5.4 UA Detection Enrichment

Add missing clients to UA detection:
- `NekoBox` → singbox
- `Stash` → clash
- `Shadowrocket` → base64 (already present)
- `v2rayNG` → base64 (already present)
- `Karing` → singbox
- `Flora` → base64
- `Streisand` → base64
- `V2Box` → base64

### Testing
- Go: test each new format output, test header injection, test UA detection

---

## Error Handling & Testing Strategy

### Per-Vector Errors

| Layer | Error | Handling |
|---|---|---|
| config aggregation | Missing Xray inbound `host` | Fallback: public IP → host file → "0.0.0.0" |
| config aggregation | Missing any optional field | Default to empty string |
| URI building | Special chars in params | URL encoding with `net/url.Values` |
| Clash template | nil config fields | Go template zero-value handling |
| Sing-box JSON | nil config fields | Omit empty fields in marshaling |
| Rust deployment | gRPC readiness timeout | Log warning, return URL anyway |
| Rust deployment | Binary download 404 | Retry fallback name, if still fail → error |
| Rust deployment | Minisign verification fail | Log warning, continue |
| Rust deployment | TLS cert acquisition fail | Log, fallback self-signed |

### Test Requirements

Each PR must pass:
- `cargo test` (Rust aegis)
- `go test ./...` (Go sub-server)
- `cargo clippy -- -D warnings`
- `go fmt ./...`

### Rust vs Go Test Division

- Rust tests: aggregator field extraction, token CRUD, deployment flow (mocked), path constants
- Go tests: URI building, Clash YAML, Sing-box JSON, Xray JSON, UA detection, headers

---

## Implementation Order & Dependencies

```
PR-1 (deploy) ────── no deps
PR-2 (proto+uri) ─── deps on: proto recompile
PR-3 (clash) ─────── deps on: PR-2 proto (same ProxyConfig fields)
PR-4 (singbox) ───── deps on: PR-2 proto
PR-5 (xray+html) ─── deps on: PR-2 proto, can start after or with PR-3/4
```

PR-3 and PR-4 can be parallelized since they only consume `ProxyConfig` from proto.

## Rollback Plan

Each PR is self-contained and can be reverted individually:
- Revert the commit / merge commit via `git revert`
- Rebuild and re-deploy aegis + sub-server via existing CI
- Rolling back PR-1: `systemctl stop wwps-sub-server`, replace binary with previous release
