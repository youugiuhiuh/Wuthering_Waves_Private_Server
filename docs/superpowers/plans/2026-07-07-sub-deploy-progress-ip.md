# Sub-Server Deploy Progress + Auto IPv4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add step-level progress reporting to subscription server deploy flow and auto-detect public IPv4 for IP-only TLS mode.

**Architecture:** Inject a progress callback (`Fn(DeployStep, u8, u8)`) into `deploy::run_deploy`; auto-detect IPv4 via external API before deployment; display a teletype progress bar that the Telegram handler edits in real time.

**Tech Stack:** Rust, tokio, reqwest, teloxide, i18n (rust-i18n yaml)

## Global Constraints

- All Rust code must pass `cargo clippy --all-targets` (no warnings) and `cargo fmt`
- Use `String` error type for `deploy.rs` (consistent with existing error handling)
- No new dependencies — use `reqwest` (already in Cargo.toml) for IPv4 API call
- Keep existing TlsMode/TlsResult structures unchanged
- Must not break existing domain-based deploy flow

---

### Task 1: DeployStep Enum + Progress Callback in `deploy.rs`

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**
- Consumes: existing `DeployParams`, `TokenManager`, `download_binary`, `verify_binary`, `deploy_binary`, `write_systemd_service`, `open_firewall_port`, `config::write_config`, `cert::*`
- Produces: `DeployStep` enum; `run_deploy` with `on_progress: F` signature

- [ ] **Step 1: Add `DeployStep` enum with step descriptions**

Add after `use` block in `deploy.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployStep {
    DownloadBinary,
    VerifyBinary,
    WriteBinary,
    SetupTls,
    WriteConfig,
    SetupService,
    OpenFirewall,
    CreateToken,
}

impl DeployStep {
    pub fn index(self) -> u8 {
        match self {
            Self::DownloadBinary => 0,
            Self::VerifyBinary => 1,
            Self::WriteBinary => 2,
            Self::SetupTls => 3,
            Self::WriteConfig => 4,
            Self::SetupService => 5,
            Self::OpenFirewall => 6,
            Self::CreateToken => 7,
        }
    }

    pub fn desc(&self) -> &'static str {
        match self {
            Self::DownloadBinary => "step_download",
            Self::VerifyBinary => "step_verify",
            Self::WriteBinary => "step_write",
            Self::SetupTls => "step_tls",
            Self::WriteConfig => "step_config",
            Self::SetupService => "step_service",
            Self::OpenFirewall => "step_firewall",
            Self::CreateToken => "step_token",
        }
    }

    pub const TOTAL: u8 = 8;
}
```

- [ ] **Step 2: Change `run_deploy` signature to include progress callback**

```rust
pub async fn run_deploy<F>(
    params: &DeployParams,
    tm: &TokenManager,
    on_progress: F,
) -> Result<DeployResult, String>
where
    F: Fn(DeployStep, u8, u8) + Send + Sync,
{
    let repo_owner = "NicholasDewar";
    let repo_name = "Wuthering_Waves_Private_Server";

    on_progress(DeployStep::DownloadBinary, 0, DeployStep::TOTAL);
    let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;

    on_progress(DeployStep::VerifyBinary, 1, DeployStep::TOTAL);
    verify_binary(&binary_data, &sig_data, "3", "sub-server")?;

    on_progress(DeployStep::WriteBinary, 2, DeployStep::TOTAL);
    deploy_binary(&binary_data)?;

    on_progress(DeployStep::SetupTls, 3, DeployStep::TOTAL);
    let tls_result = match params.tls_mode {
        TlsMode::DomainAcme => cert::setup_acme_domain(&params.domain)?,
        TlsMode::IpAcme => cert::setup_acme_ip(&params.domain)?,
        TlsMode::SelfSigned => cert::setup_self_signed()?,
        TlsMode::ReverseProxy => TlsResult::SkippedReverseProxy,
    };

    let (tls_cert, tls_key) = match &tls_result {
        TlsResult::Ready { cert_path, key_path } => (cert_path.clone(), key_path.clone()),
        TlsResult::SkippedReverseProxy => (String::new(), String::new()),
    };

    on_progress(DeployStep::WriteConfig, 4, DeployStep::TOTAL);
    let addr = format!("0.0.0.0:{}", params.port);
    config::write_config(&addr, &tls_cert, &tls_key, params.rate_limit)?;

    on_progress(DeployStep::SetupService, 5, DeployStep::TOTAL);
    write_systemd_service(params.port)?;

    on_progress(DeployStep::OpenFirewall, 6, DeployStep::TOTAL);
    open_firewall_port(params.port);

    on_progress(DeployStep::CreateToken, 7, DeployStep::TOTAL);
    let token_record = tm
        .create_token("default", &[])
        .map_err(|e| format!("create token failed: {e}"))?;

    let port_part = if params.tls_mode == TlsMode::ReverseProxy {
        String::new()
    } else {
        format!(":{}", params.port)
    };
    let sub_url = format!(
        "https://{}{}/sub/{}",
        params.domain, port_part, token_record.token
    );

    Ok(DeployResult {
        sub_url,
        token: token_record.token,
    })
}
```

- [ ] **Step 3: Update existing `run_deploy` callers (none outside subscription.rs but verify)**

Search for `deploy::run_deploy(` to confirm only `subscription.rs` calls it.

```bash
cd rust/aegis && rg "deploy::run_deploy" src/
```

Expected: only `src/adapters/telegram/handlers/subscription.rs`

- [ ] **Step 4: Run quality gates**

```bash
cd rust/aegis && cargo fmt && cargo clippy --all-targets 2>&1 | head -20
```

---

### Task 2: Auto IPv4 Detection in `deploy.rs`

**Files:**
- Modify: `rust/aegis/src/core/subscription/deploy.rs`

**Interfaces:**
- Consumes: reqwest (already in Cargo.toml)
- Produces: `pub fn is_valid_ipv4(s: &str) -> bool`; `pub async fn get_public_ipv4() -> Result<String, String>`

- [ ] **Step 1: Add `is_valid_ipv4` helper function**

Add after `DeployStep` impl:

```rust
pub fn is_valid_ipv4(s: &str) -> bool {
    if s.is_empty() || s.contains(' ') || s.starts_with('.') || s.ends_with('.') {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u8>().is_ok())
}
```

- [ ] **Step 2: Add `get_public_ipv4` function**

```rust
pub async fn get_public_ipv4() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("create HTTP client failed: {e}"))?;

    let resp = client
        .get("https://api.ipify.org?format=text")
        .send()
        .await
        .map_err(|e| format!("ipify request failed: {e}"))?;

    let ip = resp
        .text()
        .await
        .map_err(|e| format!("ipify response failed: {e}"))?;
    let ip = ip.trim().to_string();

    if !is_valid_ipv4(&ip) {
        return Err(format!("ipify returned invalid IPv4: '{}'", ip));
    }
    if ip.starts_with("127.") || ip.starts_with("10.") || ip.starts_with("172.") || ip.starts_with("192.168.") {
        return Err(format!("ipify returned private IP: '{}'", ip));
    }
    if ip == "0.0.0.0" {
        return Err("ipify returned 0.0.0.0".to_string());
    }

    Ok(ip)
}
```

- [ ] **Step 3: Run quality gates**

```bash
cd rust/aegis && cargo fmt && cargo clippy --all-targets 2>&1 | head -20
```

---

### Task 3: Update Telegram Handler — Progress Bar + Auto IP Wiring

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/subscription.rs`

**Interfaces:**
- Consumes: `DeployStep` enum from `deploy.rs`; `get_public_ipv4` from `deploy.rs`
- Produces: Updated `handle_deploy_execute` with progress editing and auto IP detection

- [ ] **Step 1: Add the `progress_bar` helper function**

Add after the last `use` block (around line 12):

```rust
fn deploy_progress_bar(current: u8, total: u8) -> String {
    if total == 0 {
        return "[░░░░░░░░░░] 0%".to_string();
    }
    let segments = 10;
    let ratio = current as f32 / total as f32;
    let filled = (ratio * segments as f32).round() as usize;
    let filled = filled.min(segments);
    let mut bar = String::from("[");
    for i in 0..segments {
        bar.push(if i < filled { '█' } else { '░' });
    }
    bar.push(']');
    let percent = (ratio * 100.0).round() as i32;
    format!("{} {}%", bar, percent.clamp(0, 100))
}
```

- [ ] **Step 2: Rewrite `handle_deploy_execute` to send progress**

Replace the entire function body (starts line 252):

```rust
async fn handle_deploy_execute(ctx: &CallbackContext) -> HandlerResult {
    let chat_id = ctx.chat_id.0.to_string();
    let Some(setup) = ctx.state.sub_setup_status(&chat_id).await else {
        return Ok(HandlerAction::Done);
    };
    ctx.state.remove_sub_setup(&chat_id).await;

    let Some(tm) = ctx.state.token_manager() else {
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("menu.sub_not_installed"))
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(HandlerAction::Done);
    };
    let tm = tm.clone();

    // Auto-detect public IPv4 if no domain
    let domain = if setup.has_domain {
        setup.domain.clone()
    } else {
        // Send "auto-detecting IP" message
        ctx.bot
            .edit_message_text(ctx.chat_id, ctx.msg_id, t!("sub.setup_auto_ip"))
            .parse_mode(ParseMode::Html)
            .await?;

        match deploy::get_public_ipv4().await {
            Ok(ip) => {
                let ip_msg = t!("sub.setup_auto_ip_done", "0" => &ip);
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, ip_msg)
                    .parse_mode(ParseMode::Html)
                    .await?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                ip
            }
            Err(_) => {
                ctx.bot
                    .edit_message_text(ctx.chat_id, ctx.msg_id, t!("sub.setup_auto_ip_fail"))
                    .parse_mode(ParseMode::Html)
                    .await?;
                // Fallback to 0.0.0.0 — will fail at TLS step but user sees error
                "0.0.0.0".to_string()
            }
        }
    };

    let tls_mode = match setup.tls_mode {
        0 => TlsMode::DomainAcme,
        1 => TlsMode::IpAcme,
        _ => TlsMode::SelfSigned,
    };
    let params = DeployParams {
        domain: domain.clone(),
        port: setup.port,
        rate_limit: setup.rate_limit,
        tls_mode,
    };

    // Send initial progress message
    let step_descs: [&str; 8] = [
        t!("sub.setup_step_download").as_ref(),
        t!("sub.setup_step_verify").as_ref(),
        t!("sub.setup_step_write").as_ref(),
        t!("sub.setup_step_tls").as_ref(),
        t!("sub.setup_step_config").as_ref(),
        t!("sub.setup_step_service").as_ref(),
        t!("sub.setup_step_firewall").as_ref(),
        t!("sub.setup_step_token").as_ref(),
    ];

    let initial_text = t!(
        "sub.setup_progress",
        "0" => deploy_progress_bar(0, 8),
        "1" => "0",
        "2" => "8",
        "3" => step_descs[0],
    );
    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, initial_text)
        .parse_mode(ParseMode::Html)
        .await?;

    match deploy::run_deploy(&params, &tm, |step, current, total| {
        let bar = deploy_progress_bar(current, total);
        let text = t!(
            "sub.setup_progress",
            "0" => bar,
            "1" => (current + 1).to_string(),
            "2" => total.to_string(),
            "3" => t!(format!("sub.setup_{}", step.desc())),
        );
        let _ = ctx.bot.edit_message_text(ctx.chat_id, ctx.msg_id, text)
            .parse_mode(ParseMode::Html)
            .await;
    }).await {
        Ok(result) => {
            let success_msg = t!(
                "sub.setup_success",
                "0" => &domain,
                "1" => params.port.to_string(),
                "2" => &result.token,
            );
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("menu.back"),
                "m_sub",
            )]]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, success_msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
        Err(e) => {
            let fail_msg = t!("sub.setup_fail", "0" => e);
            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                t!("menu.back"),
                "m_sub",
            )]]);
            ctx.bot
                .edit_message_text(ctx.chat_id, ctx.msg_id, fail_msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
    }
    Ok(HandlerAction::Done)
}
```

---

### Task 4: Add i18n Keys

- [ ] **Step 1: Add to `zh.yml`**

Open `rust/aegis/src/resources/i18n/zh.yml`, find `sub:` section (around line 550), add after `setup_step_download`:

```yaml
  setup_step_verify: "🔍 正在验证二进制签名..."
  setup_step_write: "💾 正在写入二进制文件..."
  setup_step_tls: "🔐 正在配置 TLS 证书..."
  setup_step_config: "⚙️ 正在写入配置文件..."
  setup_step_service: "🚀 正在注册系统服务..."
  setup_step_firewall: "🛡️ 正在开放防火墙端口..."
  setup_step_token: "🎫 正在创建订阅令牌..."
  setup_step_done: "✅ 部署完成！"
  setup_progress: "🔄 <b>sub-server 部署中</b>\n\n{0}\n\n<b>步骤 {1}/{2}</b>：{3}"
  setup_auto_ip: "🌐 正在自动获取公网 IPv4..."
  setup_auto_ip_done: "🌐 公网 IPv4：<code>{0}</code>"
  setup_auto_ip_fail: "❌ 自动获取 IP 失败，将尝试 0.0.0.0"
```

- [ ] **Step 2: Add to `en.yml`**

Open `rust/aegis/src/resources/i18n/en.yml`, in `sub:` section:

```yaml
  setup_step_verify: "🔍 Verifying binary signature..."
  setup_step_write: "💾 Writing binary..."
  setup_step_tls: "🔐 Configuring TLS certificate..."
  setup_step_config: "⚙️ Writing config file..."
  setup_step_service: "🚀 Registering system service..."
  setup_step_firewall: "🛡️ Opening firewall port..."
  setup_step_token: "🎫 Creating subscription token..."
  setup_step_done: "✅ Deployment complete!"
  setup_progress: "🔄 <b>sub-server deploying</b>\n\n{0}\n\n<b>Step {1}/{2}</b>：{3}"
  setup_auto_ip: "🌐 Auto-detecting public IPv4..."
  setup_auto_ip_done: "🌐 Public IPv4：<code>{0}</code>"
  setup_auto_ip_fail: "❌ Auto IP detection failed, will try 0.0.0.0"
```

- [ ] **Step 3: Add to `ja.yml`**

Open `rust/aegis/src/resources/i18n/ja.yml`, in `sub:` section:

```yaml
  setup_step_verify: "🔍 バイナリ署名を検証中..."
  setup_step_write: "💾 バイナリを書き込み中..."
  setup_step_tls: "🔐 TLS 証明書を設定中..."
  setup_step_config: "⚙️ 設定ファイルを書き込み中..."
  setup_step_service: "🚀 システムサービスを登録中..."
  setup_step_firewall: "🛡️ ファイアウォールポートを開放中..."
  setup_step_token: "🎫 サブスクリプショントークンを作成中..."
  setup_step_done: "✅ デプロイ完了！"
  setup_progress: "🔄 <b>sub-server デプロイ中</b>\n\n{0}\n\n<b>ステップ {1}/{2}</b>：{3}"
  setup_auto_ip: "🌐 パブリック IPv4 を自動検出中..."
  setup_auto_ip_done: "🌐 パブリック IPv4：<code>{0}</code>"
  setup_auto_ip_fail: "❌ 自動 IP 検出に失敗しました。0.0.0.0 を使用します"
```

---

### Task 5: Quality Gates

**Files:** (test only, no production code changes)

- [ ] **Step 1: Run fmt and clippy**

```bash
cd rust/aegis && cargo fmt && cargo clippy --all-targets 2>&1
```
Expected: no errors, no warnings

- [ ] **Step 2: Run tests**

```bash
cd rust/aegis && cargo test 2>&1
```
Expected: all tests pass (including existing subscription tests if any)

---

### Task 6: Commit

- [ ] **Step 1: Create a feature branch and commit**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server
git checkout -b feat/sub-deploy-progress-ip
git add rust/aegis/src/core/subscription/deploy.rs
git add rust/aegis/src/adapters/telegram/handlers/subscription.rs
git add docs/superpowers/specs/2026-07-07-sub-deploy-progress-ip.md
git add docs/superpowers/plans/2026-07-07-sub-deploy-progress-ip.md
git commit -m "feat: add deploy progress bar and auto IPv4 detection for sub-server"
```
