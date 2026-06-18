# 设计文档：一键部署结果转发 Matrix

**日期**: 2026-06-18
**状态**: 已设计 / 待实施
**影响文件**: `rust/aegis/src/adapters/telegram/handlers/ops.rs`

---

## 1. 问题陈述

`a_one_click`（一键部署）执行三个批量创建操作：

- XHTTP Reality × 20
- Reality Vision × 20
- TUIC × 3

每个操作返回 `BatchCreationResult`（含分享链接），但当前代码：

1. **丢弃 Ok 值** — `if let Err(e) = ...` 模式忽略 `Ok(result)`
2. **未捕获 adapter** — `tokio::spawn` 闭包不包含 `ctx.state.adapter`
3. **未利用 Matrix 路由** — `RoutingAdapter` 的敏感链接自动路由机制未被触发

结果：部署成功文本不含实际链接，Matrix 收不到任何信息。

---

## 2. 目标

- 捕获所有批量创建的分享链接
- 通过 `RoutingAdapter` 将链接同时发到 Telegram 和 Matrix
- 保持失败处理不变
- 不对现有架构做侵入式修改

---

## 3. 设计方案

### 3.1 数据流

```
a_one_click (ops.rs)
  ├─ 步骤 4: batch_create_xhttp_reality_enhanced(20)
  │   └─ Ok(result) → send_singbox_batch_result(adapter, chat_id, "XHTTP Reality", &result)
  │                      └─ RoutingAdapter: vless:// → Matrix + Telegram
  ├─ 步骤 5: batch_create_reality_vision_enhanced(20)
  │   └─ Ok(result) → send_singbox_batch_result(adapter, chat_id, "Reality Vision", &result)
  └─ 步骤 7: batch_create_tuic(3)
      └─ Ok(result) → send_singbox_batch_result(adapter, chat_id, "TUIC", &result)
```

### 3.2 改动点

**A. `tokio::spawn` 闭包捕获 adapter** (`ops.rs:440`)

```rust
// 改前
tokio::spawn(async move {

// 改后
let adapter = ctx.state.adapter.clone();
tokio::spawn(async move {
```

**B. 步骤 4 从忽略 Ok 改为转发** (`ops.rs` 步骤 4 块)

```rust
// 改前
if let Err(e) = ConfigManager::batch_create_xhttp_reality_enhanced(20, ip_version).await {
    let _ = tx.send(...);
    failed = true;
}

// 改后
match ConfigManager::batch_create_xhttp_reality_enhanced(20, ip_version).await {
    Ok(result) => {
        let _ = send_singbox_batch_result(
            adapter.clone(), chat_id_clone, "XHTTP Reality", &result,
        ).await;
    }
    Err(e) => {
        let _ = tx.send(...);
        failed = true;
    }
}
```

**C. 步骤 5 同理** — `batch_create_reality_vision_enhanced` → 协议名 `"Reality Vision"`

**D. 步骤 7 同理** — `batch_create_tuic` → 协议名 `"TUIC"`

**E. 新增 import**

```rust
use crate::app::batch_handler::send_singbox_batch_result;
```

### 3.3 不改动的部分

- `send_singbox_batch_result` 函数名（保留原样）
- `RoutingAdapter` 路由逻辑不变
- `ctx.state.adapter` 类型不变
- 失败处理逻辑不变

---

## 4. 安全与回退

- 所有 `send_singbox_batch_result` 调用使用 `let _ = ...`，发送失败不影响主流程
- 链接仅在 `Ok` 时发送，`Err` 时保持原有失败处理
- adapter 是 `Arc` 克隆，不增加生命周期复杂度

---

## 5. 验证

- `cargo check` 编译通过
- 触发一键部署：
  - Telegram 收到每条链接 + 部署进度
  - Matrix 收到 `vless://` 协议链接
