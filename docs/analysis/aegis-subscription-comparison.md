# WWPS Aegis Subscription Feature vs Mainstream Xray Panels & Scripts

> **Date:** 2026-07-07
> **Scope:** rust/aegis + tools/sub-server vs XTLS/Xray-core ecosystem panels & scripts
> **Focus:** Deployment pipeline, Base64 subscription, Clash YAML, Sing-box JSON, Xray JSON

---

## Table of Contents

1. [Investigated Projects](#1-investigated-projects)
2. [Feature Comparison Matrix](#2-feature-comparison-matrix)
3. [Protocol Support Comparison](#3-protocol-support-comparison)
4. [VLESS URL Parameter Completeness](#4-vless-url-parameter-completeness)
5. [Architecture Comparison](#5-architecture-comparison)
6. [Deployment Pipeline Diagnostic](#6-deployment-pipeline-diagnostic)
7. [Format Output Diagnostic](#7-format-output-diagnostic)
8. [Gap Summary by Severity](#8-gap-summary-by-severity)
9. [Recommended Roadmap](#9-recommended-roadmap)

---

## 1. Investigated Projects

### Web Panels

| Name | Repository | Sub Endpoint | Status |
|------|-----------|-------------|--------|
| **Remnawave** | `remnawave/backend` | `/api/sub/:shortUuid` | Active |
| **Marzban** | `Gozargah/Marzban` | `/sub/{token}/` | Active |
| **3X-UI** | `MHSanaei/3x-ui` | `/sub/<subId>`, `/json/`, `/clash/` | Active |
| **Xray-UI** | `qist/xray-ui` | None (panel-only share links) | Active |
| **X-Panel** | `xeefei/X-Panel` | `/sub/<subId>`, `/json/<subId>` | Active |
| **PasarGuard** | `PasarGuard/panel` | `/sub/{token}/` | Active |
| **Hiddify** | `hiddify/Hiddify-Panel` | `/<uuid>/sub/`, `/sub64/`, `/clashmeta/`, `/singbox/`, `/xray/` | Active |
| **TX-UI** | `Incognito-Coder/tx-ui` | `/sub/<subid>`, `/json/<subid>` | Active |
| **CELERITY** | `ClickDevTech/CELERITY-panel` | `/api/files/:token` | Active |

### One-Click Scripts

| Name | Repository | Sub Endpoint | Status |
|------|-----------|-------------|--------|
| **v2ray-agent** | `mack-a/v2ray-agent` | `/s/default/<md5>`, `/s/clashMeta/<md5>`, `/s/sing-box/<md5>` | Has subscription |
| **Xray_onekey** | `wulabing/Xray_onekey` | None | Single link only |
| **ProxySU** | `proxysu/ProxySU` | None | GUI share links only |
| **Xray-REALITY** | `zxcvos/Xray-script` | None | Links + QR |
| **reality-ezpz** | `aleskxyz/reality-ezpz` | None | Links/QR/Bot |
| **XTool** | `LordPenguin666/XTool` | None | Single link/QR |

---

## 2. Feature Comparison Matrix

| Feature | WWPS Aegis/sub-server | Remnawave | Marzban | 3X-UI | Hiddify | PasarGuard | v2ray-agent | Industry Standard |
|---------|----------------------|-----------|---------|-------|---------|------------|-------------|-------------------|
| **Base64 subscription** | ⚠️ Draft, format incomplete | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Required |
| **Clash YAML** | ⚠️ Draft, template broken | ✅ (Meta/legacy/Stash) | ✅ (Meta/legacy) | ✅ | ✅ | ✅ (Meta) | ✅ (Meta + profile) | Required |
| **Sing-box JSON** | ⚠️ Draft, fields missing | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Required |
| **Xray JSON** | ❌ Missing | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Common |
| **User-Agent detection** | ⚠️ Present, basic rules | ✅ SRR engine | ✅ | ❌ (path-based) | ✅ | ✅ | ❌ (path-based) | Growing trend |
| **URL path format override** | ✅ `?format=` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Required |
| **Token management (CRUD)** | ✅ Create/list/revoke | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Required |
| **Token revoke / expire** | ✅ Both | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Common |
| **Rate limiting** | ✅ per-token | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Common |
| **TLS auto-cert** | ⚠️ acme.sh + rcgen (buggy) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (nginx) | Required |
| **Self-signed / IP support** | ✅ Present but cert generation wrong | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Common |
| **Multi-arch release** | ❌ amd64 only | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ (script, no release) | Expected |
| **Subscription info headers** | ❌ Missing | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Common (Subscription-Userinfo, Profile-Update-Interval, Profile-Title) |
| **HTML info page** | ❌ Missing | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Common |
| **Custom templates** | ❌ Missing | ✅ | ✅ (Jinja2) | ✅ (HTML) | ✅ | ❌ | ❌ | Advanced |
| **HWID / device binding** | ❌ Missing | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | Niche |
| **WARP / external sub merge** | ❌ Missing | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | Niche |
| **QR code page** | ⚠️ `/qr` redirect to base64 | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Common |
| **Subscription protocol list** | VLESS, hy2, tuic only | VLESS, Trojan, SS, hy2 | VLESS, VMess, Trojan, SS | VMess, VLESS, Trojan, SS, hy1/hy2, WG, MTProto | All (VLESS, VMess, Trojan, hy2, TUIC, SS, WG, SSH, naive, DNSTT) | VMess, VLESS, Trojan, SS, hy2, WG | VLESS, VMess, Trojan, hy2, TUIC, Naive, anytls, SS | Broad coverage |

---

## 3. Protocol Support Comparison

| Protocol | WWPS Aegis | Remnawave | Marzban | 3X-UI | Hiddify | PasarGuard | v2ray-agent |
|----------|-----------|-----------|---------|-------|---------|------------|-------------|
| **VLESS Vision REALITY** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **VLESS XHTTP REALITY** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **VLESS WS/gRPC/TCP TLS** | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **VMess** | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Trojan** | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Shadowsocks** | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Hysteria2** | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ |
| **TUIC** | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ |
| **WireGuard** | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ | ❌ |
| **NaiveProxy / SSH** | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ |

---

## 4. VLESS URL Parameter Completeness

The VLESS share link specification (XTLS/Xray-core) defines these parameters for REALITY + XHTTP configurations.

| Parameter | WWPS sub-server | Mainstream Panels | Status |
|-----------|----------------|-------------------|--------|
| `encryption=none` | ✅ | ✅ | OK |
| `flow=xtls-rprx-vision` | ✅ | ✅ | OK |
| `security=reality\|tls` | ✅ | ✅ | OK |
| `sni={servername}` | ✅ | ✅ | OK |
| `fp={fingerprint}` | Hardcoded `chrome` | Configurable | Needs improvement |
| `pbk={publickey}` | ✅ | ✅ | OK |
| `sid={shortid}` | ✅ | ✅ | OK |
| `spx={path}` | ❌ **Missing** | ✅ | **Critical gap** |
| `type={tcp\|xhttp\|ws\|grpc}` | ✅ | ✅ | OK |
| `mode=auto` | ❌ **Missing** | ✅ (XHTTP) | **Critical gap** |
| `path={path}` | ✅ | ✅ | OK |
| `host={host}` | ❌ **Missing** | ✅ | **Critical gap** |
| `headerType=none` | ❌ **Missing** | ✅ | **Gap** |
| `extra={downloadSettings JSON}` | ⚠️ Partial | ✅ (XHTTP) | **Gap** |
| `pqv={mldsa65_verify}` | ✅ | Partial | **Ahead** |
| `alpn=h2,http/1.1` | ❌ **Missing** | ✅ | **Gap** |
| **URL encoding** | ❌ **Missing** | ✅ | **Critical gap** |

---

## 5. Architecture Comparison

### WWPS Aegis (Two-Process)

```
Client ──HTTPS──► sub-server (Go / chi router)
                        │
                   gRPC Unix Socket
                        │
                   aegis (Rust / tonic)
                        │
              ┌─────────┴──────────┐
         /etc/wwps/wwps-core/   /etc/wwps/wwps-box/
         *xray*_inbounds.json   hysteria2/tuic.json
```

**Pros:**
- Language boundary (Rust ↔ Go) enforces API contract
- gRPC over Unix socket: no network exposure, 0600 permissions
- Token management in Rust (secure SQLite)
- sub-server is optional component

**Cons:**
- Two binaries to deploy and update
- Deployment chain is fragile (download → verify → TLS → systemd → gRPC)
- Debugging requires tracing through both codebases

### Marzban (Single Process)

```
Client ──HTTPS──► Marzban (Python / FastAPI)
                        │
                   SQLite / MySQL
```

**Pros:**
- Single process, simpler deployment
- Jinja2 templates for output customization
- Large ecosystem of forks and extensions

**Cons:**
- Python performance under load
- All features in one codebase

### Hiddify (Nginx + Python)

```
Client ──HTTPS──► nginx (proxy_path routing)
                        │
                   gunicorn / Hiddify-Panel (Python / Flask)
                        │
                   SQLite
```

**Pros:**
- Huge protocol support (20+ protocols)
- WARP integration, TLS tricks, fragment
- Mature template system

**Cons:**
- Complex installation (multiple services)
- Python heavy

### Remnawave (Modern TypeScript)

```
Client ──HTTPS──► Remnawave Backend (NestJS / TypeScript)
                        │
                   PostgreSQL
```

**Pros:**
- Modern tech stack
- Sophisticated Subscription Response Rules (SRR) engine
- HWID device binding

**Cons:**
- Requires PostgreSQL
- Higher resource usage

---

## 6. Deployment Pipeline Diagnostic

The Rust aegis deploys sub-server by:
1. Downloading from GitHub releases (`deploy.rs:57-58`)
2. Verifying minisign signature (`deploy.rs:178`)
3. Setting up TLS cert (`cert.rs`)
4. Writing config JSON + systemd unit (`deploy.rs:197-199`)
5. Starting systemd service (`deploy.rs:156-158`)

### Critical Bugs

| # | Issue | File:Line | Impact |
|---|-------|-----------|--------|
| **D01** | **gRPC socket path format mismatch** — aegis writes `unix:///var/run/aegis/sub.sock` into config JSON (with `unix://` prefix), but Go client `grpc/client.go:29` dials `net.Dial("unix", addr)` which expects a bare filesystem path. The `unix://` prefix will cause dial failure. | `config.rs:25` ↔ `grpc/client.go:29` | sub-server starts but every gRPC call fails → `/sub/<token>` returns 500 |
| **D02** | **Self-signed cert is a CA, not a server cert** — `cert.rs:99` sets `params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained)`. This creates a CA certificate that clients will not accept as a server TLS certificate. | `cert.rs:97-101` | TLS handshake fails on most clients (insecure exception required, but even then CA != server cert) |
| **D03** | **IP cert acme.sh command incomplete** — `cert.rs:60` calls `acme.sh --issue --standalone -d <ip>` without `--server letsencrypt` or the `--dnssleep` / `--force` flags needed for IP certificate issuance. Let's Encrypt IP certificates also require port 80. | `cert.rs:59-62` | IP certificate issuance fails silently, falls through with missing cert files |
| **D04** | **No multi-arch releases** — `.github/workflows/public-release.yml:113` hardcodes `GOARCH=amd64`. No arm64/aarch64 binary is produced. | `release.yml:113` | arm64 VPS users cannot deploy sub-server |
| **D05** | **Minisign signature can be missing** — release workflow at `release.yml:121` skips signing when `MINISIGN_PRIVATE_KEY` secret is unset. But `deploy.rs:178` unconditionally calls `verify_binary()` which fails if no `.minisig` file. | `deploy.rs:178` ↔ `release.yml:121-133` | If CI secret is not configured, every deployment fails |
| **D06** | **systemd service has no ordering dependency on aegis** — `deploy.rs:130` writes `After=network.target`. The gRPC Unix socket is created by aegis (Rust tonic), so sub-server may start before the socket exists. | `deploy.rs:122-135` | Sub-server ready but gRPC dial fails → service appears healthy but returns errors |
| **D07** | **No pre-flight gRPC readiness check** — After systemctl start, `deploy.rs` returns immediately. There is no retry loop or socket existence check before returning the subscription URL to the user. | `deploy.rs:156-164` | User receives a subscription URL that may not work yet |
| **D08** | **Config file vs CLI flags overlap** — `config.rs` writes a JSON config file, but the systemd unit (and documented `--` flags) duplicate the same parameters. One source of truth is used, the other is ignored. | `config.rs` ↔ `deploy.rs:130-136` | Confusion about which config path is authoritative |

### Medium Issues

| # | Issue | Details |
|---|-------|---------|
| D09 | Download timeout too high | `deploy.rs:24` sets 300s but no progress reporting; on slow VPS user sees no feedback |
| D10 | No retry on download failure | Single attempt; if GitHub API rate-limited, deployment fails permanently |
| D11 | acme.sh may not be installed | No check or installation step before calling `acme.sh` |
| D12 | UFW rule doesn't check for firewall | `deploy.rs:168-170` runs `ufw allow` but ignores non-UFW firewalls |
| D13 | Systemd unit missing aegis dependency | No `BindsTo=` / `After=` on the aegis service |

---

## 7. Format Output Diagnostic

### 7.1 VLESS URI (`tools/sub-server/format/uri.go`)

| Issue | File:Line | Severity |
|-------|-----------|----------|
| Parameters not URL-encoded (sni, pbk, sid may contain `+`, `/`, `=` etc.) | `uri.go:11-12` | 🔴 **Critical** — broken links |
| Missing `spx` parameter (needed by REALITY clients) | `uri.go:11` | 🔴 **Critical** — some clients fail |
| Missing `mode=auto` for XHTTP | `uri.go:11` | 🟠 High |
| Missing `host` parameter for WS/XHTTP | `uri.go:11` | 🟠 High |
| Missing `headerType=none` | `uri.go:11` | 🟡 Medium |
| Missing `alpn` parameter | `uri.go:11` | 🟡 Medium |
| VLESS REALITY hardcodes `security=reality`, should vary by config | `uri.go:11` | 🟡 Medium |
| Hysteria2 `insecure=1` hardcoded, should use `cert_sha256` when available | `uri.go:29` | 🟡 Medium |

### 7.2 Clash YAML (`tools/sub-server/format/clash.go`)

| Issue | File:Line | Severity |
|-------|-----------|----------|
| Template uses `ws-path` unconditionally (`ws-path` only valid for WebSocket) | `clash.go:41` | 🔴 **Critical** — wrong field for TCP/XHTTP/gRPC |
| Missing `client-fingerprint` for REALITY (Clash Meta) | `clash.go` | 🔴 **Critical** — REALITY won't work |
| Missing `reality-opts` for non-REALITY TLS | `clash.go` | 🟠 High |
| Missing `network` field for XHTTP (`type: vless` + `network: xhttp`) | `clash.go:37-38` | 🟠 High |
| No proxy-groups, DNS, or routing rules | `clash.go` | 🟡 Medium |
| No `alpn` field | `clash.go` | 🟡 Medium |
| No `servername` / `sni` inside `reality-opts` | `clash.go:26-33` | 🟠 High |
| `Tag` may be empty → proxy name is empty string | `clash.go:16` | 🟡 Medium |

### 7.3 Sing-box JSON (`tools/sub-server/format/singbox.go`)

| Issue | File:Line | Severity |
|-------|-----------|----------|
| Missing transport settings for all protocols (network, path, etc.) | `singbox.go:9-55` | 🔴 **Critical** — generated config cannot connect |
| Missing REALITY settings (public_key, short_id, server_name, fingerprint) for VLESS | `singbox.go` | 🔴 **Critical** |
| No `tls` field for non-REALITY TLS configs | `singbox.go` | 🟠 High |
| `utls` is boolean `enabled: true`, should configure fingerprint | `singbox.go:41` | 🟠 High |
| TUIC missing transport settings (JSON field names) | `singbox.go` | 🟠 High |
| Hysteria2 missing `hop_interval`, `download_mbps`, `up_mbps` | `singbox.go` | 🟡 Medium |
| Missing DNS, route, and inbound sections | `singbox.go:47-49` | 🟡 Medium |

### 7.4 Subscription Headers

| Header | WWPS | Mainstream | Status |
|--------|------|-----------|--------|
| `Subscription-Userinfo` | ❌ | ✅ (upload, download, total, expire bytes) | **Gap** |
| `Profile-Update-Interval` | ❌ | ✅ | **Gap** |
| `Profile-Title` | ❌ | ✅ (Base64-encoded) | **Gap** |
| `Support-Url` | ❌ | ✅ | **Gap** |
| `Profile-Web-Page-Url` | ❌ | ✅ | **Gap** |

### 7.5 ProxyConfig Protobuf Field Coverage (`proto/subscription.proto`)

Current fields: `config_id, protocol, host, port, password, uuid, sni, pin_sha256, public_key, short_id, transport, path, flow, tag, obfs_type, obfs_password, hop_port_start, hop_port_end, alpn, congestion_control, cert_sha256`

**Missing fields needed for parity:**

| Missing Field | Used By | Priority |
|--------------|---------|----------|
| `fingerprint` (fp) | VLESS REALITY / TLS | 🔴 Critical |
| `spx` (reality path) | VLESS REALITY | 🔴 Critical |
| `host` (HTTP host) | WS / XHTTP | 🔴 Critical |
| `mode` (XHTTP mode) | XHTTP | 🟠 High |
| `extra` (JSON extra settings) | XHTTP downloadSettings | 🟠 High |
| `header_type` | TCP headerType | 🟡 Medium |
| `service_name` | gRPC | 🟡 Medium |
| `authority` | gRPC | 🟡 Medium |
| `insecure` (allowInsecure) | TLS | 🟡 Medium |
| `encryption` | VLESS encryption setting | 🟡 Medium |
| `server_name` | TLS SNI override | 🟡 Medium |

### 7.6 Aggregator Extraction Issues (`rust/aegis/src/core/subscription/aggregator.rs`)

| Issue | File:Line | Severity |
|-------|-----------|----------|
| Xray `host` is read from inbound `host` field which doesn't exist in Xray JSON — should resolve public IP or use config `host` from external | `aggregator.rs:50` | 🔴 **Critical** — host will be empty |
| `pin_sha256` is mapped from `fingerprint` — fingerprint is `fp` (TLS fingerprint like `chrome`), not cert SHA-256 | `aggregator.rs:94-98` | 🟠 High |
| `spx` (reality `shortPath`) not extracted | `aggregator.rs:88-108` | 🟠 High |
| No `host` extraction from wsSettings/xhttpSettings | `aggregator.rs:78-87` | 🟠 High |
| Only VLESS protocol scanned from Xray — no VMess, Trojan, Shadowsocks | `aggregator.rs:46-47` | 🟠 High |
| `hop_port_start/end` for hysteria2 not extracted | `aggregator.rs:210-233` | 🟡 Medium |
| TUIC `udp_relay_mode`, `heartbeat` not extracted | `aggregator.rs:235-298` | 🟡 Medium |

---

## 8. Gap Summary by Severity

### 🔴 Critical (blocks usage)

| ID | Gap | Component |
|----|-----|-----------|
| G01 | gRPC socket path format mismatch (`unix://` prefix in config vs bare path in Go client) | Deployment |
| G02 | Self-signed cert is CA, not server cert | Deployment |
| G03 | VLESS URI not URL-encoded | sub-server format |
| G04 | VLESS missing `spx`, `mode`, `host`, `extra` | sub-server format |
| G05 | Clash YAML uses `ws-path` for all transports | sub-server format |
| G06 | Clash YAML missing `client-fingerprint`, REALITY settings | sub-server format |
| G07 | Sing-box JSON missing transport, reality, and TLS settings | sub-server format |
| G08 | Aggregator cannot read Xray `host` (reads non-existent `host` field) | aegis aggregator |
| G09 | `ProxyConfig` protobuf missing fingerprint, spx, host, mode, extra fields | Proto + all consumers |

### 🟠 High (severe UX degradation)

| ID | Gap | Component |
|----|-----|-----------|
| G10 | No arm64 release | CI/CD |
| G11 | minisign signing optional in CI but mandatory in deploy | Deployment |
| G12 | systemd has no ordering dependency on aegis | Deployment |
| G13 | No pre-flight gRPC readiness check | Deployment |
| G14 | Missing Subscription-Userinfo and other standard headers | sub-server handler |
| G15 | Aggregator pin_sha256 ↔ fingerprint mapping wrong | aegis aggregator |
| G16 | Aggregator only supports VLESS (no VMess, Trojan, SS) | aegis aggregator |
| G17 | No Xray JSON output format | sub-server format |

### 🟡 Medium (nice to have)

| ID | Gap | Component |
|----|-----|-----------|
| G18 | No HTML info page for browser visitors | sub-server handler |
| G19 | No custom template system | sub-server format |
| G20 | Acme.sh not pre-installed, no fallback | Deployment |
| G21 | No retry on download failure | Deployment |
| G22 | User-Agent detection rules too simplistic (no SRR) | sub-server handler |
| G23 | Singular config format for config JSON vs CLI flags | Deployment |

---

## 9. Recommended Roadmap

### Phase 1: Fix Deployment Pipeline

> Goal: `aegis` can reliably deploy `sub-server` on both amd64 and arm64.

| Step | Description | Files |
|------|-------------|-------|
| 1.1 | Fix gRPC socket path: strip `unix://` prefix in config, or make Go client parse it | `config.rs`, `grpc/client.go` |
| 1.2 | Fix self-signed cert: remove `is_ca`, add proper SAN | `cert.rs` |
| 1.3 | Add `After=wwps-aegis.service` + `Requires=wwps-aegis.service` to systemd unit | `deploy.rs` |
| 1.4 | Add gRPC readiness retry loop before returning success | `deploy.rs` |
| 1.5 | Add arm64 build target to release workflow | `release.yml` |
| 1.6 | Make minisign verification optional (skip if no .minisig) | `deploy.rs` |
| 1.7 | Add acme.sh auto-install or skip with warning | `cert.rs` |

### Phase 2: Fix Base64 Subscription Output

> Goal: Produced VLESS/hysteria2/tuic links work in all clients.

| Step | Description | Files |
|------|-------------|-------|
| 2.1 | URL-encode all VLESS parameters (sni, pbk, sid, spx, path, host) | `uri.go` |
| 2.2 | Add `spx`, `mode`, `host`, `headerType`, `alpn` to VLESS URI | `uri.go` |
| 2.3 | Expand `ProxyConfig` protobuf with all missing fields | `subscription.proto` |
| 2.4 | Update aggregator to extract new fields from config JSONs | `aggregator.rs` |
| 2.5 | Update Go format builder (`BuildURI`) to use all fields | `uri.go` |

### Phase 3: Fix Clash YAML

> Goal: Produced Clash YAML works in Clash Verge / Mihomo / Stash.

| Step | Description | Files |
|------|-------------|-------|
| 3.1 | Rewrite template: use correct field names per transport type | `clash.go` |
| 3.2 | Add `client-fingerprint` (for REALITY) and `reality-opts` | `clash.go` |
| 3.3 | Add basic proxy-groups (select, url-test, fallback) | `clash.go` |
| 3.4 | Add basic routing rules (geosite/geoip for bypass) | `clash.go` |

### Phase 4: Fix Sing-box JSON

> Goal: Produced Sing-box JSON works in SFA/SFI / Hiddify / NekoBox.

| Step | Description | Files |
|------|-------------|-------|
| 4.1 | Rewrite VLESS outbound with full transport settings | `singbox.go` |
| 4.2 | Add REALITY settings (server_name, public_key, short_id) | `singbox.go` |
| 4.3 | Add TLS settings (enabled, utls fingerprint, insecure) | `singbox.go` |
| 4.4 | Add hysteria2 / TUIC full transport | `singbox.go` |
| 4.5 | Add DNS and route sections | `singbox.go` |

### Phase 5: Feature Parity

> Goal: Feature-complete subscription server comparable to Marzban/Hiddify.

| Step | Description | Files |
|------|-------------|-------|
| 5.1 | Add Xray JSON output format | `format/xray.go` |
| 5.2 | Add HTML subscription info page | `handler/page.go` |
| 5.3 | Add standard subscription headers | `handler/subscription.go` |
| 5.4 | Enrich User-Agent detection rules | `handler/subscription.go` |
| 5.5 | Add external subscription merge (optional) | `handler/merge.go` |
| 5.6 | Add customizable output templates (JSON/YAML config) | `config/`, `format/` |

---

## Appendix: Key Source File References

### aegis (Rust)

| File | Purpose |
|------|---------|
| `rust/aegis/src/core/subscription/server.rs` | gRPC server (tonic) over Unix socket |
| `rust/aegis/src/core/subscription/token.rs` | Token CRUD (SQLite) |
| `rust/aegis/src/core/subscription/aggregator.rs` | Config aggregator (Xray + sing-box → ProxyConfig) |
| `rust/aegis/src/core/subscription/deploy.rs` | Download → verify → deploy sub-server binary |
| `rust/aegis/src/core/subscription/cert.rs` | TLS cert: acme.sh domain/IP, rcgen self-signed |
| `rust/aegis/src/core/subscription/minisign.rs` | Minisign signature verification |
| `rust/aegis/src/core/subscription/config.rs` | Write sub-server config JSON |
| `rust/aegis/src/core/paths.rs` | Path constants (BIN, GRPC_SOCK, CERTS_DIR, etc.) |

### sub-server (Go)

| File | Purpose |
|------|---------|
| `tools/sub-server/main.go` | Entry, chi router, TLS/plain HTTP |
| `tools/sub-server/config/config.go` | Config struct + flag parsing |
| `tools/sub-server/handler/subscription.go` | Subscription handler, UA detection, format routing |
| `tools/sub-server/format/uri.go` | Plain URI + Base64 list |
| `tools/sub-server/format/clash.go` | Clash YAML (text/template) |
| `tools/sub-server/format/singbox.go` | Sing-box JSON |
| `tools/sub-server/format/list.go` | Base64 + plain list output |
| `tools/sub-server/grpc/client.go` | gRPC client → aegis |
| `tools/sub-server/cache/lru.go` | LRU cache |
| `tools/sub-server/middleware/ratelimit.go` | per-token rate limiter |

### Shared

| File | Purpose |
|------|---------|
| `proto/subscription.proto` | Protobuf: ProxyConfig, SubscriptionToken, services |
| `.github/workflows/public-release.yml` | CI/CD: build + sign + release |
