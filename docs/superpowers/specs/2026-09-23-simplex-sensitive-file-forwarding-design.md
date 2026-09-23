# 设计：敏感内容转发 —— 为 SimpleX 补齐「文本转附件」与文件分流

> 模式：**strict**（设计先行） ｜ 状态：**待批准** ｜ 日期：2026-09-23
> 触发：P2 真机验收（murky-pull）暴露的三点不对称之一（记为 C）

## 1. 现状（已核实，非推断）

**敏感转发目前有两套，且只覆盖一半平台：**

| 层 | 位置 | 行为 |
| --- | --- | --- |
| 路由层 | `common/routing.rs::RoutingAdapter::send_message` | 文本命中 `is_sensitive` → 改投 secondary（TG+SimpleX 下即 SimpleX） |
| 平台层 | `gateways/matrix/adapter.rs::MatrixAdapter::send_message`（`:120`） | 文本命中 `is_sensitive` → **改写为附件** `batch_result.txt`（`send_attachment`），不落普通消息 |

**三点不对称（C 的核心）：**

1. **SimpleX 没有「文本转附件」**：`SimplexAdapter::send_message` 直接把敏感文本当普通消息发。Matrix 会转成 `.txt` 附件（规避纯文本泄露/便于保存），SimpleX 不会。
2. **`RoutingAdapter` 的文件类发送不分流**：`send_file`/`send_image`/`send_voice` 一律走 `primary`（`routing.rs`）。即敏感内容**以文件形式**发出时永远落在 TG，绕过 secondary。
3. **文件类判定缺位**：`is_sensitive` 只接受 `&str`（文本）；对二进制/文件名无判定。

`is_sensitive` 判据（`routing.rs`）：协议前缀 `vmess:// vless:// trojan:// ss:// hysteria:// hysteria2:// tuic://` + 字段 `"privateKey" "secretKey" "password":`。

## 2. 目标

在 `--tg-simplex` 形态下，敏感内容的落点行为与 Matrix 形态**对齐**：

- G1：命中 `is_sensitive` 的**文本**，投递到 SimpleX 时以**附件**（`.txt`）形式，而非可转发的普通消息。
- G2：`send_file`/`send_image`/`send_voice` 若属于敏感内容，改投 secondary；否则维持 primary。
- G3：不改变现有 Matrix 行为与 `--simplex`（纯）行为。

## 3. 设计

### 3.1 把「敏感文本转附件」下沉为适配器能力（推荐）

不要在每个调用点判断，而是在 `SimpleXAdapter::send_message` 内与 Matrix 对称处理：

```rust
// gateways/simplex/adapter.rs（impl BotAdapter for SimplexAdapter::send_message）
if crate::common::routing::is_sensitive(&content.text) {
    // 复用既有 write_temp_file(0600) → File::new(path) → send_msg → remove
    return self.send_file(target, "batch_result.txt", content.text.into_bytes(), "text/plain").await;
}
// 否则照旧发文本
```

- `is_sensitive` 已是 `pub(crate)`，`MatrixAdapter` 已在用；SimpleX 直接复用。
- SimpleX 的 `send_file` 已存在（`write_temp_file` + `0600` + 发送后删除），无新增依赖、无新增临时文件生命周期。

> 备选（不推荐）：在 `RoutingAdapter` 里用 `secondary.send_file(...)` 代替 `send_message`。缺点：路由层被迫知道「平台用什么 mime/文件名」，且纯 `--simplex`（无 RoutingAdapter）仍不生效。

### 3.2 文件类发送的分流

`RoutingAdapter` 目前 `send_file/send_image/send_voice` 一律 primary。两种可选策略（需你裁决）：

| 策略 | 行为 | 适用 |
| --- | --- | --- |
| **S-A 全量分流** | TG+SimpleX 下，**所有**文件/图片/语音都发 secondary | 保守：tg-simplex 本就是「文件即敏感」的部署假设 |
| **S-B 判定分流** | 只有 `name`/内容命中敏感判据才分流；二进制内容可做关键字扫描（成本高、易漏） | 精确，但二进制不可靠 |

**默认建议 S-A**：`--tg-simplex` 的定位就是「敏感内容落 SimpleX」，且 aegis 发送的文件基本都是配置/备份（`batch_result.txt`、`*_inbounds.json`、证书包），全量分流与部署意图一致、实现最简。

### 3.3 与 P2.1 的关系

P2.1 已修：`RoutingAdapter.secondary_target` 动态化（重钉即时生效）+ 敏感发送失败记 `log::error`。本设计**依赖 P2.1**（否则落点仍是启动快照）。

## 4. 边界与风险

1. **纯 `--simplex` 无 secondary**：3.1 的适配器级转换对该形态**也生效**（敏感文本→附件），但目标就是管理员本身。行为变化需确认可接受；若不接受，可用 `capabilities`/开关限定。
2. **`is_sensitive` 是启发式**：非敏感文本若误判会变附件（体验下降，不泄露）；敏感文本若漏判仍以普通消息发出（**这是主要残余风险**，与 Matrix 完全一致）。
3. **SimpleX 附件**：bot 已用 `--create-bot-allow-files` 启动（安装器单元），接收方需能收文件。
4. **测试**：适配器级转换可单测（构造敏感文本 → 断言走 `send_file` 路径的 mock 期望）；文件分流可单测 RoutingAdapter。真机验收需 SimpleX 客户端确认收到 `.txt` 附件。

## 5. 交付拆分（建议）

| # | 内容 | 依赖 |
| --- | --- | --- |
| C1 | `SimpleXAdapter::send_message` 敏感文本转 `.txt` 附件（对称 Matrix） | P2.1 |
| C2 | `RoutingAdapter` 文件类发送按 S-A 分流（默认） | C1、裁决 S-A/S-B |
| C3 | 单测 + 真机验收 + 文档勘误 | C1/C2 |

## 6. 待裁决

| # | 决策 | 默认 |
| --- | --- | --- |
| D-C1 | 文件类分流策略 | **S-A 全量分流** |
| D-C2 | 纯 `--simplex` 是否也做「敏感文本→附件」 | **是**（与适配器行为一致，最简单） |
| D-C3 | 附件文件名/mime | `batch_result.txt` / `text/plain`（与 Matrix 一致） |
