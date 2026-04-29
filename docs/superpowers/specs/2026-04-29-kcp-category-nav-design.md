# KCP 分类导航 UI 重设计规格

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 KCP 遮罩选择 UI 从平铺分类列表改为 4 个分类按钮导航，每个分类子菜单直接显示简介+添加按钮。

**Architecture:** 纯 UI 层改动，不涉及数据模型。KcpMask 枚举新增 `brief()` 和 `category_code()` 方法，main.rs 重写 3 个回调处理器、新增 2 个、删除 1 个。

**Tech Stack:** Rust, teloxide (Telegram Bot API)

---

## 1. KcpMask 新增方法 (`config.rs`)

### `brief()` → `&'static str`

| 变体 | brief |
|------|-------|
| MkcpOriginal | "轻量级XOR混淆，仅FNV1a校验" |
| MkcpAes128Gcm | "AES-128-GCM认证加密，推荐首选" |
| Noise | "随机噪声填充，抗流量分析" |
| Salamander | "蝾螈混淆协议，抗深度包检测" |
| Sudoku | "数独混淆算法，强度更高" |
| HeaderDns | "DNS查询流量伪装" |
| HeaderWechat | "微信视频通话流量伪装" |
| HeaderSrtp | "SRTP音视频流媒体伪装" |
| HeaderUtp | "BitTorrent uTP协议伪装" |
| HeaderDtls | "DTLS 1.2加密数据包伪装" |
| HeaderWireguard | "WireGuard VPN流量伪装" |
| Xdns | "扩展DNS，支持自定义域名和解析器" |
| Xicmp | "ICMP数据包伪装，极端限制网络适用" |
| HeaderCustom | "自定义UDP头部格式" |

### `category_code()` → `&'static str`

| category | code |
|----------|------|
| 🔐 加密层 | `enc` |
| 🌀 混淆层 | `obf` |
| 🎭 伪装层 | `dis` |
| ⚡ 扩展层 | `ext` |

---

## 2. UI 流程 (`main.rs`)

### 2.1 `u_kcp_init` (修改)

4 个分类按钮 + 返回按钮。按钮格式 `[emoji 分类名 (数量)]`。

消息文本：
```
🚀 <b>KCP (mKCP+FinalMask) 配置</b>

✨ <b>特点:</b>
• 基于 mKCP 协议的可靠传输
• FinalMask 多层遮罩任意叠加(1-5层)
• 支持加密、混淆、伪装、扩展四大类遮罩

📋 <b>步骤 1: 选择遮罩类别</b>
⚠️ 至少选择1层，建议加密层+伪装层组合
```

按钮布局（2x2 网格）：
```
[🔐 加密层 (2)] [🌀 混淆层 (3)]
[🎭 伪装层 (6)] [⚡ 扩展层 (3)]
[⬅️ 返回]
```

回调数据格式：
- `u_kcp_cat:enc` — 加密层分类子菜单
- `u_kcp_cat:obf` — 混淆层分类子菜单
- `u_kcp_cat:dis` — 伪装层分类子菜单
- `u_kcp_cat:ext` — 扩展层分类子菜单

### 2.2 `u_kcp_cat:{cat}` (新增 — 首次选择)

消息顶部显示当前遮罩栈（首次为空则不显示）。分类子菜单中每个遮罩显示：emoji + 名称 + brief 简介文本。

以加密层为例：
```
📋 <b>当前遮罩栈:</b>
(空)

🔐 <b>加密层</b> — 选择要添加的遮罩

🔀 <b>mKCP Original</b>
轻量级XOR混淆，仅FNV1a校验

🔐 <b>mKCP AES-128-GCM</b>
AES-128-GCM认证加密，推荐首选
```

按钮（每个类型一行）：
```
[✅ mKCP Original] [✅ mKCP AES-128-GCM]
[⬅️ 返回分类] [🗑️ 清空重选]
```

回调数据格式：
- `u_kcp_add:mo` — 直接添加（复用现有处理器）

### 2.3 `u_kcp_add:{code}` (微调)

栈显示改为动态显示。按钮改为：
```
[➕ 继续添加遮罩层]  →  u_kcp_more:{code}
[✅ 完成配置]        →  u_kcp_done:{code}
[🗑️ 清空重选]       →  u_kcp_init
```

### 2.4 `u_kcp_more:{existing}` (修改)

同样改为 4 个分类按钮（替代旧版平铺列表）。

消息格式：
```
📋 <b>当前遮罩栈:</b>
1️⃣ mKCP Original

➕ <b>选择要添加的遮罩类别</b> (已达1层，最多5层)
```

按钮布局：
```
[🔐 加密层 (1)] [🌀 混淆层 (3)]
[🎭 伪装层 (6)] [⚡ 扩展层 (3)]
[✅ 完成配置] [🗑️ 清空重选]
```

注意：按钮数字为该分类剩余可选数量（all_variants 中排除已添加 code）。

### 2.5 `u_kcp_mcat:{existing},{cat}` (新增 — 叠加选择)

与 `u_kcp_cat` 类似，但：
- 顶部显示当前遮罩栈
- 已添加的类型显示为 ☑️ 已添加（禁用按钮），或直接不显示
- 添加按钮回调是 `u_kcp_push:{existing}:{code}`

### 2.6 删除 `u_kcp_sel:{code}`

不再需要单独的详情确认页。简介文本已提供足够信息。

### 2.7 `u_kcp_push:{existing}:{code}` (不变)

逻辑保持不变，只是后续 `u_kcp_more` 现在使用分类导航。

---

## 3. 回调数据长度

Telegram 限制 64 字节：

| 回调 | 长度 | 示例 |
|------|------|------|
| `u_kcp_cat:enc` | 12 | ✓ |
| `u_kcp_mcat:mo,sa,hd,hw,hdt:dis` | 33 | ✓ |
| `u_kcp_push:mo,sa,hd,hw,hdt:no` | 34 | ✓ |
| `u_kcp_done:mo,sa,hd,hw,hdt` | 28 | ✓ |

所有均在限制内。

---

## 4. 边界情况

- **分类无可选项：** 从按钮列表中移除该分类，或显示 `(0)` 时禁用。
- **仅剩1层空间：** 正常显示分类按钮，`u_kcp_push` 仍显示 "已达最大层数(5层)"。
- **HeaderCustom：** from_code 直接创建，无需额外参数。