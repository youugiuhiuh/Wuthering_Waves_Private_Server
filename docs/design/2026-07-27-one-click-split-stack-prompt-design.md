# One-Click Deployment: IPv6 Split-Stack Prompt

**Date:** 2026-07-27
**Status:** Design approved, pending implementation

## Problem

The one-click deployment (`handle_one_click`) always hardcodes `SplitStackV4Primary` when both IPv4 and IPv6 are available. There is no user-facing option to choose `SplitStackV6Primary` or to opt out of split-stack. Users with specific routing preferences (e.g., better IPv6 backhaul, wanting IPv6 upload) cannot express them.

## Design

### Interaction Flow

```
handle_one_click
  ├─ Step 1-3: tune → xray_init → pq  (unchanged)
  │
  ├─ Step 3.5: [NEW] IP version detection + interactive prompt
  │    ├─ IPv4 only   → ip_version = IPv4, continue directly
  │    ├─ IPv6 only   → ip_version = IPv6, continue directly
  │    ├─ Both          → send_message with inline keyboard:
  │    │                  [v4↑v6↓]  [v6↑v4↓]  [no split]
  │    │                  ┌─ user clicks → ip_version set, resume Step 4
  │    │                  └─ 30s timeout → cancel deployment, send "deploy cancelled" message
  │
  ├─ Step 4: xhttp  (uses ip_version, XHTTP split-stack via host_secondary)
  ├─ Step 5-8: vision, singbox, h2, security  (unchanged — use own internal IP detection)
```

### Scope

**Only XHTTP** receives the user-chosen `ip_version`. Vision and Hysteria2 steps are unaffected; they continue to auto-detect IPs internally in their batch functions.

### Files Changed

| File | Change |
|---|---|
| `rust/aegis/src/shared/handlers/ops.rs` | Add callback route, extract IP resolution into `resolve_one_click_ip_version()`, add `handle_one_click_ip_response()` |
| `rust/aegis/i18n/zh.toml` | Add 2 i18n keys (prompt title, cancelled message) |
| `rust/aegis/i18n/en.toml` | Same keys for English |

### Code Changes (Detailed)

#### 1. New callback route (`handle`, line 33)

```rust
d if d.starts_with("a_one_click_ip:") => handle_one_click_ip_response(event, d).await,
```

#### 2. Replace IP detection in `handle_one_click` (lines 588-599)

Before:
```rust
let ip_version = {
    let (v4, v6) = tokio::join!(...);
    match (&v4, &v6) { ... }
};
```

After:
```rust
let ip_version = match resolve_one_click_ip_version(
    &adapter, &target, &msg_id,
).await {
    Ok(v) => v,
    Err(_) => {
        let _ = tx.send(t!("ops.deploy_ip_cancelled").to_string());
        return;
    }
};
```

#### 3. New function `resolve_one_click_ip_version`

```rust
async fn resolve_one_click_ip_version(
    adapter: &Arc<dyn BotAdapter>,
    target: &TargetId,
    msg_id: &AegisMsgId,
) -> Result<IpVersion> {
    let (v4, v6) = tokio::join!(
        SystemMonitor::get_public_ip(),
        SystemMonitor::get_public_ipv6(),
    );

    match (&v4, &v6) {
        (Ok(_), Ok(_)) => {
            // Both available → prompt user
            let (tx, mut rx) = tokio::sync::oneshot::channel::<IpVersion>();

            let markup = Markup {
                buttons: vec![vec![
                    InlineButton {
                        text: "v4 ↑ v6 ↓ (XHTTP)".into(),
                        data: "a_one_click_ip:split4".into(),
                    },
                    InlineButton {
                        text: "v6 ↑ v4 ↓ (XHTTP)".into(),
                        data: "a_one_click_ip:split6".into(),
                    },
                    InlineButton {
                        text: "no split (IPv4 only)".into(),
                        data: "a_one_click_ip:v4".into(),
                    },
                ]],
            };

            adapter
                .send_message(
                    target,
                    MessageContent {
                        text: t!("ops.deploy_ip_split_title").into_owned(),
                        markup: Some(markup),
                    },
                )
                .await?;

            // Store the tx in a global/session map for callback handler to find
            ONE_CLICK_IP_PENDING.lock().insert(target.to_string(), tx);

            match tokio::time::timeout(Duration::from_secs(30), &mut rx).await {
                Ok(Ok(version)) => Ok(version),
                _ => {
                    ONE_CLICK_IP_PENDING.lock().remove(&target.to_string());
                    Err(anyhow!("timeout or channel closed"))
                }
            }
        }
        (Ok(_), Err(_)) => Ok(IpVersion::IPv4),
        (Err(_), Ok(_)) => Ok(IpVersion::IPv6),
        _ => Ok(IpVersion::IPv4),
    }
}
```

#### 4. New callback handler `handle_one_click_ip_response`

```rust
async fn handle_one_click_ip_response(
    event: &CallbackEvent,
    data: &str,
) -> HandlerResult {
    let ip_version = match data.strip_prefix("a_one_click_ip:") {
        Some("split4") => IpVersion::SplitStackV4Primary,
        Some("split6") => IpVersion::SplitStackV6Primary,
        Some("v4") => IpVersion::IPv4,
        _ => return Ok(HandlerAction::Done),
    };

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(format!("Selected: {}", ip_version.label())),
        )
        .await?;

    if let Some(tx) = ONE_CLICK_IP_PENDING
        .lock()
        .remove(&event.target.to_string())
    {
        let _ = tx.send(ip_version);
    }

    Ok(HandlerAction::Done)
}
```

#### 5. Global state for callback routing

Store the pending `oneshot::Sender` in a static `LazyLock<Mutex<HashMap<String, Sender<IpVersion>>>>`.

### Error / Edge Case Handling

| Scenario | Behavior |
|---|---|
| Both IPs, user clicks a button | `oneshot` channel delivered, deployment continues |
| Both IPs, 30s timeout | Send cancellation message, `return` from spawned task |
| Both IPs, IP resolution fails unexpectedly | Treat as single-IP case (no prompt) |
| Only IPv4/IPv6 available | Skip prompt entirely, use single-IP IpVersion |
| User navigates away during prompt | Callback data won't match; no-op |

### i18n Keys

| Key | zh | en |
|---|---|---|
| `ops.deploy_ip_split_title` | 你的机器同时支持 IPv4 和 IPv6，XHTTP 是否启用上下行分离？ | Your machine supports both IPv4 and IPv6. Enable XHTTP split-stack? |
| `ops.deploy_ip_cancelled` | 部署已取消 | Deployment cancelled |

### Tests

- Unit test: `resolve_one_click_ip_version` logic for single-IP cases (no async prompt part)
- Existing regression tests (582 tests) must continue to pass

### Not Changed

- Vision batch creation (`reality.rs`) — uses own internal IP detection
- Hysteria2 batch creation (`hy2_batch.rs`) — uses own internal IP detection
- `xhttp.rs` / `config.rs` — already support both `SplitStackV4Primary` and `SplitStackV6Primary`
