# One-Click Deploy TLS Reality Bonus — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一键部署 TLS+CDN 模式自动补齐 Reality XHTTP 至 20 端口；Route53 `cdn_ports` 改为 `[443]`；删除 `batch_create_xhttp_tls_enhanced` 死 else 分支。

**Architecture:** Route53 DNS provider gets `cdn_ports = [443]` (was empty). In `run_one_click()` TLS branch, after TLS batch succeeds, unconditionally pad with `batch_create_xhttp_reality_enhanced(20 - tls_created)`. Dead else branch removed from `batch_create_xhttp_tls_enhanced` since both providers now have non-empty cdn_ports.

**Tech Stack:** Rust (aegis), rust_i18n YAML locales

## Global Constraints

- Run `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run` after each task
- All 671 existing tests must pass
- i18n keys must exist in all 3 locales (zh, en, ja)

---

## Task 1: Route53 cdn_ports → [443]

**Files:**
- Modify: `rust/aegis/src/core/types.rs:31`
- Test: `rust/aegis/src/core/types.rs:245-248`

**Interfaces:**
- Produces: `DnsProvider::Route53.cdn_ports()` returns `&[443]`

- [ ] **Step 1: Update Route53 cdn_ports**

```rust
// rust/aegis/src/core/types.rs:31
Self::Route53 => &[443],  // was &[]
```

- [ ] **Step 2: Update test**

```rust
// rust/aegis/src/core/types.rs:245-248
fn route53_cdn_ports_contains_443() {
    let ports = DnsProvider::Route53.cdn_ports();
    assert_eq!(ports, &[443]);
    assert!(!ports.is_empty());
}
```

- [ ] **Step 3: Run cargo nextest -p aegis types**

Run: `cargo nextest run -p aegis types 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/types.rs
git commit -m "feat: set Route53 cdn_ports to [443] for TLS CDN support"
```

---

## Task 2: Remove dead else branch from batch_create_xhttp_tls_enhanced

**Files:**
- Modify: `rust/aegis/src/core/xray/xhttp.rs:111-182`

**Interfaces:**
- Consumes: `batch_create_xhttp_tls_enhanced(domain, certs, ip_version)`
- Produces: `batch_create_xhttp_tls_enhanced` with only CDN branch (no else)

- [ ] **Step 1: Read current function to confirm else branch location**

Confirm: lines 111-144 = if branch, lines 145-182 = else branch to delete

- [ ] **Step 2: Replace if/else with single for loop**

```rust
// Before (lines 111-182):
if !cdn_ports.is_empty() {
    for (i, &cdn_port) in cdn_ports.iter().enumerate() {
        // CDN port logic
    }
} else {
    for i in 0..20 {
        // random port logic — DELETE THIS BLOCK
    }
}

// After:
for (i, &cdn_port) in cdn_ports.iter().enumerate() {
    // CDN port logic
}
```

Exact replacement oldString:
```rust
        if !cdn_ports.is_empty() {
            for (i, &cdn_port) in cdn_ports.iter().enumerate() {
                let port: i32 =
                    if crate::core::system::maintenance::MaintenanceManager::is_port_available(
                        cdn_port,
                    )
                    .await
                    {
                        cdn_port as i32
                    } else {
                        loop {
                            let p = rng.gen_range(10000..60000);
                            if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                            continue;
                        }
                            if crate::core::system::maintenance::MaintenanceManager::is_port_available(p).await {
                            break p as i32;
                        }
                        }
                    };

                let uuid = ConfigManager::generate_wwps_uuid().await?;
                let path = ConfigManager::generate_random_path();

                let (config, link) = ConfigManager::build_tls_xhttp_node(
                    i, port, &uuid, domain, certs, ip_version, &path,
                );

                batch_configs.push(config);
                links.push(link);

                let _ = crate::core::system::maintenance::MaintenanceManager::allow_port(cdn_port)
                    .await;
            }
        } else {
            let port_443_available =
                crate::core::system::maintenance::MaintenanceManager::is_port_available(443).await;

            for i in 0..20 {
                let port: i32 = if i == 0 && port_443_available {
                    443
                } else {
                    loop {
                        let p = rng.gen_range(10000..60000);
                        if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                            continue;
                        }
                        if crate::core::system::maintenance::MaintenanceManager::is_port_available(
                            p,
                        )
                        .await
                        {
                            break p as i32;
                        }
                    }
                };

                let uuid = ConfigManager::generate_wwps_uuid().await?;
                let path = ConfigManager::generate_random_path();

                let (config, link) = ConfigManager::build_tls_xhttp_node(
                    i, port, &uuid, domain, certs, ip_version, &path,
                );

                batch_configs.push(config);
                links.push(link);

                let _ =
                    crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16)
                        .await;
            }
        }
```

Exact replacement newString:
```rust
        for (i, &cdn_port) in cdn_ports.iter().enumerate() {
            let port: i32 =
                if crate::core::system::maintenance::MaintenanceManager::is_port_available(
                    cdn_port,
                )
                .await
                {
                    cdn_port as i32
                } else {
                    loop {
                        let p = rng.gen_range(10000..60000);
                        if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                            continue;
                        }
                        if crate::core::system::maintenance::MaintenanceManager::is_port_available(p).await {
                            break p as i32;
                        }
                    }
                };

            let uuid = ConfigManager::generate_wwps_uuid().await?;
            let path = ConfigManager::generate_random_path();

            let (config, link) = ConfigManager::build_tls_xhttp_node(
                i, port, &uuid, domain, certs, ip_version, &path,
            );

            batch_configs.push(config);
            links.push(link);

            let _ = crate::core::system::maintenance::MaintenanceManager::allow_port(cdn_port)
                .await;
        }
```

- [ ] **Step 3: Run cargo fmt && cargo clippy && cargo nextest run**

Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run 2>&1 | tail -20`
Expected: All 671 tests pass

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/xray/xhttp.rs
git commit -m "refactor: remove dead else branch from batch_create_xhttp_tls_enhanced"
```

---

## Task 3: Add Reality bonus padding in run_one_click TLS branch

**Files:**
- Modify: `rust/aegis/src/shared/handlers/ops.rs:634-668`

**Interfaces:**
- Consumes: `ConfigManager::batch_create_xhttp_tls_enhanced()`, `ConfigManager::batch_create_xhttp_reality_enhanced()`
- Produces: `run_one_click` TLS branch appends Reality ports after TLS success

- [ ] **Step 1: Read current TLS match branch to find insertion point**

Read ops.rs lines 637-668. After the `Ok(tls_result)` block's message send (around line 653), insert Reality padding before the closing braces.

- [ ] **Step 2: Add Reality bonus after TLS success message**

Find this block (lines 641-655):
```rust
Ok(result) => {
    all_links.extend(result.links);
    let _ = adapter
        .send_message(
            &target,
            MessageContent {
                text: t!("ops.deploy_created_xhttp_tls",
                        "0" => ip_version.label(),
                        "1" => result.created_count.to_string(),
                        "2" => result.config_file.as_deref().unwrap_or("?"))
                    .into_owned(),
                markup: None,
            },
        )
        .await;
}
```

Replace with:
```rust
Ok(result) => {
    all_links.extend(result.links);
    let _ = adapter
        .send_message(
            &target,
            MessageContent {
                text: t!("ops.deploy_created_xhttp_tls",
                        "0" => ip_version.label(),
                        "1" => result.created_count.to_string(),
                        "2" => result.config_file.as_deref().unwrap_or("?"))
                    .into_owned(),
                markup: None,
            },
        )
        .await;

    // Pad with Reality XHTTP to reach 20 total
    let reality_count = 20_usize.saturating_sub(result.created_count);
    match ConfigManager::batch_create_xhttp_reality_enhanced(reality_count, ip_version).await {
        Ok(reality_result) => {
            all_links.extend(reality_result.links);
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("ops.deploy_created_xhttp_bonus",
                                "0" => ip_version.label(),
                                "1" => reality_result.created_count.to_string(),
                                "2" => reality_result.config_file.as_deref().unwrap_or("?"))
                            .into_owned(),
                        markup: None,
                    },
                )
                .await;
        }
        Err(e) => {
            let _ = tx.send(
                t!("ops.deploy_fail",
                    "0" => format!("{}: {}", t!("ops.deploy_fail_xhttp"), e)
                )
                .to_string(),
            );
            failed = true;
        }
    }
}
```

- [ ] **Step 3: Run cargo fmt && cargo clippy && cargo nextest run**

Run: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run 2>&1 | tail -20`
Expected: All 671 tests pass

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/shared/handlers/ops.rs
git commit -m "feat: pad TLS CDN path with Reality XHTTP to reach 20 ports total"
```

---

## Task 4: Add i18n key ops.deploy_created_xhttp_bonus

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml`
- Modify: `rust/aegis/src/resources/i18n/en.yml`
- Modify: `rust/aegis/src/resources/i18n/ja.yml`
- Modify: `rust/aegis/src/core/i18n.rs:166` (test keys list)

**Interfaces:**
- Produces: New i18n key `ops.deploy_created_xhttp_bonus` in all 3 locales

- [ ] **Step 1: Add zh.yml key**

Find `ops.deploy_created_xhttp_tls:` in zh.yml and add after it:
```yaml
  deploy_created_xhttp_bonus: "✅ 额外创建了 %{1} 个 Reality XHTTP (%{0}) 配置\n📁 %{2}"
```

- [ ] **Step 2: Add en.yml key**

```yaml
  deploy_created_xhttp_bonus: "✅ Additionally created %{1} Reality XHTTP (%{0}) config(s)\n📁 %{2}"
```

- [ ] **Step 3: Add ja.yml key**

```yaml
  deploy_created_xhttp_bonus: "✅ 追加で %{1} 個の Reality XHTTP（%{0}）を作成しました\n📁 %{2}"
```

- [ ] **Step 4: Add key to domain_translation_keys_exist test**

Find `ops.deploy_created_xhttp_tls` in i18n.rs test keys array (around line 166) and add:
```rust
"ops.deploy_created_xhttp_bonus",
```

- [ ] **Step 5: Run domain_translation_keys_exist test**

Run: `cargo nextest run -p aegis domain_translation_keys_exist 2>&1`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml rust/aegis/src/resources/i18n/ja.yml rust/aegis/src/core/i18n.rs
git commit -m "feat: add ops.deploy_created_xhttp_bonus i18n key (zh/en/ja)"
```

---

## Verification

After all tasks, run full quality gate:
```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
```
Expected: All tests pass, 0 clippy warnings.

## Summary

| Task | File | Change |
|------|------|--------|
| 1 | `types.rs:31` | `Route53 => &[443]` |
| 2 | `types.rs:245-248` | Update test |
| 3 | `xhttp.rs:111-182` | Delete else branch |
| 4 | `ops.rs:637-668` | Add Reality padding |
| 5 | `zh/en/ja.yml` | New i18n key |
| 6 | `i18n.rs` | Add key to test |
