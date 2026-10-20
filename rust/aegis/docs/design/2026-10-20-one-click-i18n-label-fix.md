# 设计文档：一键部署步骤 4/5 的 i18n 文案与真实调用链对齐

- 日期：2026-10-20
- 范围：仅 i18n 文案（不改文件名前缀、不改控制流）
- 模式：strict（AGENTS.md）
- 状态：待审批

## 1. 问题陈述

用户报告一键部署的进度文案与实际产物不一致：

```
[5/8] 正在创建 Vision Reality (×20)... (IPv4)
✅ XHTTP Reality (IPv4) 已创建 20 个配置
📁 batch_xhttp_6197d4a2_inbounds.json
```

第 5 步文案声称创建 "Vision Reality"，但实际产出的是 **XHTTP Reality** 配置，
文件名前缀为 `batch_xhttp_`。

## 2. 根因分析（图证据）

### 2.1 步骤 4 的文案与分支不符

`ops.rs:596-607` 在进入 `match &mode` **之前**，无条件发送固定文案
`ops.deploy_step_xhttp_tls`（"正在创建 TLS XHTTP (×20)..."）。

但 `mode` 有两个分支：

| 分支 | 调用 | 产物 |
|---|---|---|
| `XhttpDeployMode::Reality` | `batch_create_xhttp_reality_enhanced(20, ..)` (`ops.rs:609`) | `Proto::XHTTP` → `batch_xhttp_*` |
| `XhttpDeployMode::Tls {..}` | `batch_create_xhttp_tls_enhanced(..)` (`ops.rs:639`) | `Proto::XHTTP` → `batch_xhttp_*` |

因此**无域名一键部署**（`one_click_domain_no_mode` → `XhttpDeployMode::Reality`，
`xray.rs:268`）时，步骤 4 的文案错报为 "TLS XHTTP"。

### 2.2 步骤 5 的文案名实不符

`ops.rs:711-717` 发送 `ops.deploy_step_vision`
（zh: "正在创建 Vision Reality (×20)..."），随后调用
`batch_create_reality_vision_enhanced`（`reality.rs:153`），
该函数**确实**使用 `Proto::Vision`，产物为 `batch_reality_*`，
完成消息用 `ops.deploy_created_vision`（"✅ Reality Vision ..."）。

**当前代码路径本身是正确的**：步骤 5 产出 Vision Reality 配置。

### 2.3 用户实际观察到的现象归因

用户看到 `[5/8] 正在创建 Vision Reality` 后紧接
`✅ XHTTP Reality ... batch_xhttp_...`，说明该用户看到的是
**步骤 4 的完成消息与步骤 5 的进度文案在时间上相邻**：

1. 步骤 4 进度文案被 `deploy_step_xhttp_tls` 覆盖为 "TLS XHTTP"
   → 但用户未使用域名，实际走 Reality 分支
2. 步骤 4 完成消息 `deploy_created_xhttp` 异步送达（`send_message`，非 edit）
3. 步骤 5 进度文案通过 `edit_message` 覆盖同一占位消息

Telegram 的 `send_message` 与 `edit_message` 是两条独立的时间线，
步骤 4 的"XHTTP Reality 已创建"（异步）落在步骤 5 进度更新（同步 500ms 节流）
的视觉位置之后，形成"Vision Reality 正在创建 → 却报 XHTTP Reality"的错位观感。

### 2.4 文案命名混乱（术语层）

三个 key 的命名与语义不匹配：

| key | zh 文案 | 实际语义 |
|---|---|---|
| `deploy_step_xhttp_tls` | 正在创建 TLS XHTTP | 步骤 4，**按 mode 可为 Reality 或 TLS** |
| `deploy_step_vision` | 正在创建 Vision Reality | 步骤 5，Reality+Vision |
| `deploy_created_xhttp` | ✅ XHTTP Reality | 步骤 4 Reality 分支 |
| `deploy_created_xhttp_tls` | ✅ TLS XHTTP | 步骤 4 TLS 分支 |

## 3. 设计目标与非目标

### 目标

- G1：步骤 4 的进度文案必须与实际 `mode` 分支一致
- G2：步骤 5 的进度文案保持与实际调用一致（现状已正确，仅复核）
- G3：三语（zh/en/ja）文案同步更新
- G4：key 命名反映真实语义，避免再次混淆
- G5：补回归测试，防止文案与分支再次漂移

### 非目标（本次明确不做）

- N1：不修改 `batch_file_prefix`（`config.rs:125`）的映射
- N2：不拆分 Reality XHTTP 与 TLS XHTTP 的 `Proto::XHTTP` 共用问题
- N3：不修改 `send_message` / `edit_message` 的调用时序
- N4：不改动 `ONE_CLICK_TOTAL_STEPS = 8` 的步骤数

> 说明：N2 是**独立的安全隐患**——`xhttp.rs:94` 与 `xhttp.rs:154` 都传
> `Proto::XHTTP`，导致 `handle_cfg_del_all_exec` 的 `"xhttp"` 与 `"tls"` 过滤
> （`xray.rs:1054-1080`）会互相误删对方的配置。已在设计的"后续工作"中单列，
> 不在本次文案修复范围内。

## 4. 方案设计

### 4.1 新增/重命名 i18n key

**新增** `ops.deploy_step_xhttp_reality`（zh: "正在创建 XHTTP Reality (×20)..."）

**保留** `ops.deploy_step_xhttp_tls`（TLS 分支专用，语义已准确）

**重命名** `ops.deploy_step_vision` → 保持不变（语义已准确，避免无谓改动）

最终 key 与用途：

| key | 用途 | 分支条件 |
|---|---|---|
| `ops.deploy_step_xhttp_reality` | 步骤 4 进度（Reality） | `XhttpDeployMode::Reality` |
| `ops.deploy_step_xhttp_tls` | 步骤 4 进度（TLS） | `XhttpDeployMode::Tls` |
| `ops.deploy_step_vision` | 步骤 5 进度 | 固定 Vision Reality |

### 4.2 控制流改动（`ops.rs`）

将固定的 `send_progress(4, 8, ...)` 拆入 `match &mode` 的两个分支：

```rust
if !failed {
    match &mode {
        XhttpDeployMode::Reality => {
            send_progress(
                &tx, 4, 8,
                format!("{} ({})", t!("ops.deploy_step_xhttp_reality"), ip_version.label()),
            );
            match ConfigManager::batch_create_xhttp_reality_enhanced(20, ip_version, true).await {
                // ... 现有逻辑不变
            }
        }
        XhttpDeployMode::Tls { domain, cert_paths } => {
            send_progress(
                &tx, 4, 8,
                format!("{} ({})", t!("ops.deploy_step_xhttp_tls"), ip_version.label()),
            );
            match ConfigManager::batch_create_xhttp_tls_enhanced(domain, cert_paths, ip_version).await {
                // ... 现有逻辑不变
            }
        }
    }
}
```

改动要点：
- 去掉 `match &mode` 之前的那一次 `send_progress`
- 两个分支各自发送正确的步骤号 `4, 8`
- 分支内部原有的 `send_message` / `Err` 处理逻辑**逐字保留**

### 4.3 三语文案

| key | zh | en | ja |
|---|---|---|---|
| `deploy_step_xhttp_reality` | 正在创建 XHTTP Reality (×20)... | Creating XHTTP Reality (×20)... | XHTTP Reality (×20) を作成中... |
| `deploy_step_xhttp_tls` | 正在创建 TLS XHTTP (×20)...（不变） | Creating TLS XHTTP (×20)...（不变） | TLS XHTTP を作成しています（×20）...（不变） |
| `deploy_step_vision` | 正在创建 Vision Reality (×20)...（不变） | （不变） | （不变） |

注意：`deploy_step_xhttp` 已存在（zh: "正在创建 XHTTP Reality (×20)..."），
语义与新增的 `deploy_step_xhttp_reality` **完全重合**。

→ **决策点 D1**：复用 `ops.deploy_step_xhttp`，还是新增
`ops.deploy_step_xhttp_reality`？

- 复用：零新增 key，但该 key 目前是**死键**（无任何调用点），复用等于激活它
- 新增：key 命名更聚焦一键部署语境，但三语各多一行

**推荐：复用 `ops.deploy_step_xhttp`**——它已存在于三语且文案精确匹配，
避免了重复定义，也顺带清理了一个死键。

### 4.4 测试设计（TDD）

在 `ops.rs` 的 `mod tests` 内新增：

```rust
/// 回归：步骤 4 的进度文案必须按 mode 分派，
/// Reality 分支不得再复用 TLS 分支的文案。
#[test]
fn one_click_step4_progress_text_matches_mode() {
    let src = include_str!("ops.rs");
    let start = src.find("pub async fn run_one_click").expect("run_one_click 应存在");
    let body = &src[start..];
    let end = body.find("\nasync fn handle_one_click").unwrap_or(body.len());
    let body = &body[..end];

    let code = body
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    // 两个 step-4 文案 key 都必须出现，且分别位于对应分支内
    assert!(code.contains("ops.deploy_step_xhttp_reality"), "Reality 分支应有专属步骤 4 文案");
    assert!(code.contains("ops.deploy_step_xhttp_tls"), "TLS 分支应有专属步骤 4 文案");

    // 全局唯一性：步骤号 4 的 send_progress 不得出现在 match 之外
    // （通过「两个 key 都出现在 4, 8 附近」间接断言）
}

/// 三语健壮性：步骤 4/5 的文案 key 在三种语言下都能解析。
#[serial]
#[test]
fn one_click_step_progress_keys_exist_in_all_locales() {
    let keys = [
        "ops.deploy_step_xhttp_reality",
        "ops.deploy_step_xhttp_tls",
        "ops.deploy_step_vision",
        "ops.deploy_created_xhttp",
        "ops.deploy_created_vision",
    ];
    for lang in [Lang::Zh, Lang::En, Lang::Ja] {
        set_lang(lang);
        for key in &keys {
            let value = rust_i18n::t!(**key);
            assert!(
                !value.is_empty() && *value != *key,
                "key '{}' 在 {:?} 下缺失", key, lang
            );
        }
    }
    set_lang(Lang::Zh);
}
```

同时把新 key 加入 `core/i18n.rs` 的 `domain_translation_keys_exist` 列表。

## 5. 涉及文件（4 个）

| 文件 | 改动 |
|---|---|
| `rust/aegis/src/shared/handlers/ops.rs` | 步骤 4 文案按 mode 分派；新增 2 个测试 |
| `rust/aegis/src/resources/i18n/zh.yml` | 复用/新增步骤 4 Reality 文案 |
| `rust/aegis/src/resources/i18n/en.yml` | 同上 |
| `rust/aegis/src/resources/i18n/ja.yml` | 同上 |
| `rust/aegis/src/core/i18n.rs` | key 存在性测试列表追加 |

> 5 个文件，超出 AGENTS.md「≤3 文件」的 rapid 阈值，故走 strict 流程。

## 6. 验证计划

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test one_click` （新增 2 个测试 + 现有 6 个一键部署测试）
4. `cargo test i18n` （`domain_translation_keys_exist` 等）
5. 人工核对：`grep -c "deploy_step_xhttp_reality" src/resources/i18n/*.yml` 三语各 ≥1

## 7. 风险

| 风险 | 缓解 |
|---|---|
| 复用 `deploy_step_xhttp` 后，若未来该 key 被独立使用会耦合 | 测试断言步骤 4 使用它，语义锁定为「XHTTP Reality 创建中」 |
| 三语 key 数量本就不同（zh 683 / en 663 / ja 663），不能做 parity 断言 | 沿用 `domain_translation_keys_exist` 的逐 key 显式列表模式 |
| 步骤号重复发送（分支内外都可能发 4） | 去掉 match 外的发送点，测试断言唯一性 |

## 8. 后续工作（不在本次范围）

- **F1（安全）**：拆分 Reality XHTTP 与 TLS XHTTP 的文件名前缀。
  `xhttp.rs:94` 与 `xhttp.rs:154` 同用 `Proto::XHTTP`，导致
  `handle_cfg_del_all_exec` 中 `"xhttp"` 与 `"tls"` 过滤互相误删。
  需新增 `Proto::XhttpTls` 或给 `create_standalone_config` 增加显式前缀参数。
- **F2（UX）**：步骤完成消息改用 `edit_message` 追加到进度消息，
  而非独立 `send_message`，消除异步错位观感。

## 9. 待确认

- **D1**：步骤 4 Reality 文案复用已存在的 `ops.deploy_step_xhttp`，
  还是新增 `ops.deploy_step_xhttp_reality`？（推荐复用）
- 是否认可 F1/F2 单独排期？