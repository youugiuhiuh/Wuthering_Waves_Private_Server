# Design: Subscription Server Deploy — Progress Bar + Auto IPv4

**Date**: 2026-07-07
**Status**: Draft
**Mode**: Normal

## Goal

Fix the subscription server (`sub-server`) deploy flow so users see **real-time progress updates** in Telegram and IP-only mode **auto-detects the machine's public IPv4** instead of silently using `0.0.0.0`.

## Root Cause

1. `deploy::run_deploy` in `rust/aegis/src/core/subscription/deploy.rs:173` is a single monolithic async function with **no progress reporting**. It fetches from GitHub, verifies, writes files, sets up TLS, configures systemd, opens firewall, and creates tokens all at once. The Telegram handler (`subscription.rs:252`) sends one "downloading" message, then blocks until the entire flow finishes or fails.
2. When user chooses "IP only" mode, `setup.domain` stays empty and gets replaced with `"0.0.0.0"` in `handle_deploy_execute:274-278`, making certificates and the final subscription URL useless.

## Out of Scope

- Download progress percentage (requires streaming `reqwest` body). Step-level progress is sufficient.
- Discord/Matrix adapters for subscription deploy (Telegram-only handler for now).
- Changing the deployment file layout or systemd unit structure.
- Real IPv4 detection in `cert::setup_acme_ip` itself (only needs the IP to be passed correctly).

## Files Changed

| File | Action |
|------|--------|
| `rust/aegis/src/core/subscription/deploy.rs` | Modify: add `DeployStep` enum; add progress callback param to `run_deploy`; call callback at each step |
| `rust/aegis/src/adapters/telegram/handlers/subscription.rs` | Modify: send & edit progress message; auto-detect IPv4; simplify IP-only flow (no EnterIp step needed) |
| `rust/aegis/src/app/state.rs` | Modify: keep `domain` field; no structural change needed for IPv4 auto-detection |
| `rust/aegis/src/resources/i18n/zh.yml` | Add: progress step labels |
| `rust/aegis/src/resources/i18n/en.yml` | Add: progress step labels |
| `rust/aegis/src/resources/i18n/ja.yml` | Add: progress step labels |

## Design Details

### 1. `deploy.rs` — DeployStep + Progress Callback

```rust
pub enum DeployStep {
    DownloadBinary,   // 1/8
    VerifyBinary,     // 2/8
    WriteBinary,      // 3/8
    SetupTls,         // 4/8
    WriteConfig,      // 5/8
    SetupService,     // 6/8
    OpenFirewall,     // 7/8
    CreateToken,      // 8/8
}

impl DeployStep {
    pub fn index(&self) -> u8 {
        match self {
            Self::DownloadBinary => 0,
            Self::VerifyBinary   => 1,
            Self::WriteBinary    => 2,
            Self::SetupTls       => 3,
            Self::WriteConfig    => 4,
            Self::SetupService   => 5,
            Self::OpenFirewall   => 6,
            Self::CreateToken    => 7,
        }
    }
    pub const TOTAL: u8 = 8;
}
```

Change `run_deploy` signature:

```rust
pub async fn run_deploy<F>(
    params: &DeployParams,
    tm: &TokenManager,
    on_progress: F,
) -> Result<DeployResult, String>
where
    F: Fn(DeployStep, u8, u8) + Send + Sync,
{
    on_progress(DeployStep::DownloadBinary, 0, 8);
    let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;

    on_progress(DeployStep::VerifyBinary, 1, 8);
    verify_binary(...)?;

    on_progress(DeployStep::WriteBinary, 2, 8);
    deploy_binary(...)?;

    on_progress(DeployStep::SetupTls, 3, 8);
    let tls_result = match params.tls_mode { ... };

    on_progress(DeployStep::WriteConfig, 4, 8);
    config::write_config(...)?;

    on_progress(DeployStep::SetupService, 5, 8);
    write_systemd_service(...)?;

    on_progress(DeployStep::OpenFirewall, 6, 8);
    open_firewall_port(...);

    on_progress(DeployStep::CreateToken, 7, 8);
    token_record = tm.create_token()?;

    Ok(DeployResult { ... })
}
```

### 2. Auto IPv4 Detection

When `setup.has_domain == false`, auto-detect public IPv4 before constructing `DeployParams`:

```rust
async fn get_public_ipv4() -> Result<String, String> {
    // Try external API first
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;

    let resp = client
        .get("https://api.ipify.org?format=text")
        .send()
        .await
        .map_err(|_| "external IP check failed")?;

    let ip = resp.text().await.map_err(|_| "read IP response failed")?;
    let ip = ip.trim().to_string();

    if !is_valid_ipv4(&ip) {
        return Err("invalid IPv4 returned".to_string());
    }
    Ok(ip)
}

fn is_valid_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        p.parse::<u8>().is_ok()
    }) && !s.starts_with("127.") && !s.starts_with("10.") && !s.starts_with("0.")
}
```

If auto-detection fails, fall back to prompting the user for manual IP input via `EnterIp` step.

### 3. `subscription.rs` — Progress Message

Add a helper to build the progress bar text (mirrors `xray/installer.rs`):

```rust
fn progress_bar(current: u8, total: u8) -> String {
    let segments = 10;
    let ratio = current as f32 / total as f32;
    let filled = (ratio * segments as f32).round() as usize;
    let mut bar = String::from("```\n[");
    for i in 0..segments {
        bar.push(if i < filled { '█' } else { '░' });
    }
    bar.push(']');
    let pct = (ratio * 100.0).round() as i32;
    format!("{} {}%", bar, pct.clamp(0, 100))
}
```

In `handle_deploy_execute`:

```rust
// 1. Send initial "starting" message, get msg_id
// 2. Run deploy with progress callback:
//    - on each step: edit_message to show `progress_bar(current, total) + "当前: {step desc}"`
// 3. On success: edit final "done" message with subscription URL
// 4. On failure: edit error message
```

### 4. i18n Keys

```yaml
# zh.yml
sub:
  setup_step_download: "⬇️ 正在下载 sub-server..."
  setup_step_verify: "🔍 正在验证二进制签名..."
  setup_step_write: "💾 正在写入二进制文件..."
  setup_step_tls: "🔐 正在配置 TLS 证书..."
  setup_step_config: "⚙️ 正在写入配置文件..."
  setup_step_service: "🚀 正在注册系统服务..."
  setup_step_firewall: "🛡️ 正在开放防火墙端口..."
  setup_step_token: "🎫 正在创建订阅令牌..."
  setup_step_done: "✅ 部署完成！"
  setup_progress: "🔄 <b>sub-server 部署中</b>\n\n{bar}\n\n<b>步骤 {current}/{total}</b>：{desc}"
  setup_auto_ip: "🌐 正在自动获取公网 IPv4..."
  setup_auto_ip_done: "🌐 公网 IPv4：<code>{ip}</code>"
  setup_auto_ip_fail: "❌ 自动获取 IP 失败，请手动输入："
  setup_q_ip: "✏️ 请输入服务器公网 IPv4 地址："
  setup_q_ip_invalid: "❌ IP 格式无效，请重新输入："
```

## Testing

1. Unit test `is_valid_ipv4`
2. Unit test `progress_bar` formatting
3. Unit test `DeployStep::index` and `TOTAL`
4. Verify `cargo clippy --all-targets` passes
5. Verify `cargo test` passes
6. Verify `cargo fmt` is applied

## Implementation Order (6 tasks)

1. Add `DeployStep` enum + progress callback to `deploy.rs`
2. Add `get_public_ipv4` + `is_valid_ipv4` to `deploy.rs`
3. Add `progress_bar` helper + update `handle_deploy_execute` in `subscription.rs`
4. Add i18n keys (zh/en/ja)
5. Add auto IP detection call in `handle_deploy_execute`
6. Run quality gates: fmt, clippy, test
