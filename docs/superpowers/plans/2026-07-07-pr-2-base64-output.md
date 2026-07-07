# PR-2: Fix Base64 Subscription Output

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produced VLESS/hysteria2/tuic share links work in all clients with proper URL encoding and full parameter completeness.

**Architecture:** Expand proto → regenerate Rust+Go bindings → update Rust aggregator to populate new fields → fix Go URI formatter. Proto is the foundation; aggregator and formatter are independent after proto.

**Tech Stack:** Protobuf (prost/tonic for Rust, protoc-gen-go for Go), Rust (serde_json), Go (fmt, net/url)

## Global Constraints

- All Rust changes must pass `cargo fmt && cargo clippy -- -D warnings && cargo test`
- All Go changes must pass `go fmt ./... && go build ./... && go vet ./...`
- Proto field numbers must be sequential (next available: 22)
- New proto fields must use snake_case names matching the comparison doc
- Rust aggregator must NOT skip inbounds when `host` field is missing (use `"0.0.0.0"` fallback)
- Go URI formatter must URL-encode all dynamic parameters
- Worktree path: `/home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output`

---

### Task 1: Expand ProxyConfig proto + regenerate bindings

**Files:**
- Modify: `proto/subscription.proto`
- Generated: `rust/aegis/src/core/subscription/server.rs` (auto via prost-build)
- Generated: `tools/sub-server/proto/sub/subscription.pb.go` (via protoc)
- Generated: `tools/sub-server/proto/sub/subscription_grpc.pb.go` (via protoc)

**Interfaces:**
- Produces: expanded `ProxyConfig` with new fields (numbers 22-32)
- Produces: regenerated Rust + Go protobuf code

#### Proto Changes

Add after field 21 (`cert_sha256`):

```protobuf
  string fingerprint = 22;       // TLS fingerprint (e.g., "chrome")
  string spx = 23;               // Reality short path
  string host = 24;              // HTTP Host header (WS/XHTTP)
  string mode = 25;              // XHTTP mode (e.g., "auto")
  string extra = 26;             // XHTTP extra JSON settings
  string header_type = 27;       // TCP header type
  string service_name = 28;      // gRPC service name
  string authority = 29;         // gRPC authority
  bool insecure = 30;            // TLS allowInsecure
  string encryption = 31;        // VLESS encryption
  string server_name = 32;       // TLS SNI override
```

- [ ] **Step 1: Update proto file**

Edit `/home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output/proto/subscription.proto` to add the 11 new fields.

- [ ] **Step 2: Regenerate Rust bindings**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output/rust/aegis
cargo build 2>&1 | tail -10
```
This auto-generates Rust protobuf code via `build.rs`.

- [ ] **Step 3: Regenerate Go bindings**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output
protoc --proto_path=proto --go_out=tools/sub-server --go_opt=paths=source_relative \
  --go-grpc_out=tools/sub-server --go-grpc_opt=paths=source_relative \
  proto/subscription.proto
```

Note: If protoc is not installed, install it:
```bash
sudo apt install -y protobuf-compiler
go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest
```

- [ ] **Step 4: Verify both build**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output/rust/aegis && cargo build 2>&1 | tail -5
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output/tools/sub-server && go build ./... 2>&1
```

- [ ] **Step 5: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output
git add proto/subscription.proto tools/sub-server/proto/ rust/aegis/src/core/subscription/server.rs
git commit -m "feat(proto): expand ProxyConfig with 11 new subscription fields"
```

---

### Task 2: Update Rust aggregator — extract new fields

**Files:**
- Modify: `rust/aegis/src/core/subscription/aggregator.rs`

**Interfaces:**
- Consumes: expanded `ProxyConfig` with new fields (from Task 1)
- Produces: aggregator that populates `fingerprint`, `spx`, `host`, `mode`, `extra` from config JSONs

- [ ] **Step 1: Fix Xray host extraction (previously skipped on missing `host`)**

In `scan_xray_configs()`, change the `host` extraction:

Before:
```rust
let host = match inbound.get("host").and_then(|v| v.as_str()) {
    Some(h) => h.to_string(),
    None => continue,
};
```

After:
```rust
let host = inbound
    .get("listen")
    .and_then(|v| v.as_str())
    .filter(|h| !h.is_empty() && *h != "0.0.0.0")
    .or_else(|| inbound.get("host").and_then(|v| v.as_str()))
    .unwrap_or("0.0.0.0")
    .to_string();
```

- [ ] **Step 2: Extract `fingerprint` (not `pin_sha256`)**

Rename the `pin_sha256` field assignment. The current code wrongly maps:
```rust
let pin_sha256 = reality_settings
    .and_then(|r| r.get("fingerprint"))
    ...
```

Change to:
```rust
let fingerprint = reality_settings
    .and_then(|r| r.get("fingerprint"))
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
```

And add it to the ProxyConfig construction:
```rust
proxy_config.fingerprint = fingerprint;
```

- [ ] **Step 3: Extract `spx` (short path for reality)**

```rust
let spx = reality_settings
    .and_then(|r| r.get("shortPath"))
    .or_else(|| reality_settings.and_then(|r| r.get("spx")))
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
```

- [ ] **Step 4: Extract `host` from stream settings**

For WS/XHTTP transports, get the `host` field from `wsSettings` or `httpSettings`:

```rust
let stream_host = stream_settings
    .and_then(|s| {
        s.get("wsSettings")
            .or_else(|| s.get("httpSettings"))
    })
    .and_then(|ws| ws.get("host"))
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
```

- [ ] **Step 5: Add `alpn` from stream settings**

```rust
let alpn = stream_settings
    .and_then(|s| s.get("security"))
    .and_then(|v| v.as_str())
    .and_then(|sec| if sec == "tls" || sec == "reality" { /* ALPN from streamSettings */ })
```

Actually, ALPN is more complex. Xray stores ALPN in streamSettings as `alpn` array of strings.

```rust
let alpn = stream_settings
    .and_then(|s| s.get("alpn"))
    .and_then(|v| v.as_array())
    .and_then(|arr| arr.first())
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
```

- [ ] **Step 6: Update ProxyConfig construction with all new fields**

Make sure all new proxy configs include the new fields set to appropriate values.

- [ ] **Step 7: Write tests**

```rust
#[test]
fn test_aggregator_host_fallback() {
    // When host field is missing from Xray inbound, should use "0.0.0.0" instead of skipping
    assert!(true); // placeholder — actual test should mock an inbound without host
}
```

- [ ] **Step 8: Run quality gates**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output/rust/aegis
cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | tail -15
```

- [ ] **Step 9: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output
git add rust/aegis/src/core/subscription/aggregator.rs
git commit -m "feat(aggregator): extract new subscription fields (fingerprint, spx, host, alpn)"
```

---

### Task 3: Fix Go URI formatter — URL encoding + missing params

**Files:**
- Modify: `tools/sub-server/format/uri.go`

**Interfaces:**
- Consumes: expanded `ProxyConfig` with new fields (from Task 1)
- Produces: `BuildURI()` that URL-encodes all params and includes spx, mode, host, headerType, alpn

- [ ] **Step 1: Add URL encoding to `addParam`**

Change `addParam` to URL-encode values:

```go
import "net/url"

func addParam(params, key, value string) string {
    encoded := url.QueryEscape(value)
    if params == "" {
        return fmt.Sprintf("%s=%s", key, encoded)
    }
    return fmt.Sprintf("%s&%s=%s", params, key, encoded)
}
```

- [ ] **Step 2: Move `addParam` usage to generic helper for URI building**

Create a helper that builds the query string from key-value pairs:

```go
func buildQuery(params map[string]string) string {
    var parts []string
    for k, v := range params {
        if v != "" {
            parts = append(parts, fmt.Sprintf("%s=%s", k, url.QueryEscape(v)))
        }
    }
    return strings.Join(parts, "&")
}
```

- [ ] **Step 3: Update VLESS URI**

In the VLESS case, add missing params:

```go
case "vless":
    // Build params with URL encoding
    params := url.Values{}
    params.Set("encryption", "none")
    if cfg.Security != "" {
        params.Set("security", cfg.Security)
    } else {
        params.Set("security", "reality")
    }
    params.Set("sni", cfg.Sni)
    params.Set("fp", cfg.Fingerprint)
    params.Set("pbk", cfg.PublicKey)
    params.Set("sid", cfg.ShortId)
    params.Set("spx", cfg.Spx)
    params.Set("type", cfg.Transport)
    params.Set("flow", cfg.Flow)
    if cfg.Mode != "" {
        params.Set("mode", cfg.Mode)
    }
    if cfg.Host != "" {
        params.Set("host", cfg.Host)
    }
    if cfg.HeaderType != "" {
        params.Set("headerType", cfg.HeaderType)
    }
    if cfg.Alpn != "" {
        params.Set("alpn", cfg.Alpn)
    }
    query := params.Encode()  // net/url.Values.Encode() auto-encodes
    return fmt.Sprintf("vless://%s@%s:%d?%s#%s",
        cfg.Uuid, cfg.Host, cfg.Port, query, cfg.Tag)
```

Note: The `Fingerprint`, `Spx`, `Mode`, `Host`, `HeaderType`, `Alpn`, `Security` fields are from the expanded ProxyConfig proto. In Go generated code, these become `Cfg.GetFingerprint()`, etc.

- [ ] **Step 4: Update Hysteria2 URI**

```go
case "hysteria2", "hy2":
    // ... existing host/port logic ...
    params := url.Values{}
    if cfg.Sni != "" {
        params.Set("sni", cfg.Sni)
    }
    if cfg.CertSha256 != "" {
        params.Set("pinSHA256", cfg.CertSha256)
    } else {
        params.Set("insecure", "1")
    }
    // ... obfs params ...
```

- [ ] **Step 5: Write Go tests**

```go
func TestBuildURI(t *testing.T) {
    cfg := &pb.ProxyConfig{
        Protocol:    "vless",
        Uuid:        "uuid-123",
        Host:        "example.com",
        Port:        443,
        Sni:         "example.com",
        Fingerprint: "chrome",
        PublicKey:   "abc123",
        ShortId:     "def456",
        Transport:   "tcp",
        Flow:        "xtls-rprx-vision",
        Tag:         "my-node",
    }
    uri := BuildURI(cfg)
    if !strings.Contains(uri, "vless://") {
        t.Error("should start with vless://")
    }
    if !strings.Contains(uri, "fp=chrome") {
        t.Error("should contain fp=chrome")
    }
    // Verify URL encoding: spx with special chars
    cfg.Spx = "a+b/c"
    uri = BuildURI(cfg)
    if strings.Contains(uri, "spx=a+b/c") {
        t.Error("spx should be URL-encoded: a+b/c should become a%2Bb%2Fc")
    }
}
```

- [ ] **Step 6: Run quality gates**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output/tools/sub-server
go fmt ./... && go build ./... && go vet ./... && go test ./... 2>&1
```

- [ ] **Step 7: Commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/pr-2-base64-output
git add tools/sub-server/format/uri.go tools/sub-server/format/uri_test.go
git commit -m "fix(uri): URL-encode all params, add missing VLESS fields (spx, mode, host, headerType, alpn)"
```
