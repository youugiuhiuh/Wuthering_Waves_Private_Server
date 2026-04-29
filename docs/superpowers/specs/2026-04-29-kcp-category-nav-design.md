# KCP 分类导航 UI 重设计规格

**日期：** 2026-04-29
**状态：** 已批准

## 目标

将 KCP 遮罩选择 UI 从平铺分类列表改为 4 个分类按钮导航，每个分类子菜单显示简介描述和直接添加按钮。

## 架构

纯 UI 层改动，不涉及数据模型。需：
1. 在 `KcpMask` 枚举上新增 `brief()`、`category_code()`、`variants_by_category()` 方法
2. 重写 main.rs 中的 `u_kcp_init` 和 `u_kcp_more` 处理器
3. 新增 `u_kcp_cat` 和 `u_kcp_mcat` 处理器
4. 删除 `u_kcp_sel` 处理器（不再需要）
5. 微调 `u_kcp_add` 处理器

## 1. KcpMask 新增方法 (config.rs)

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

### `variants_by_category(code: &str)` → `Vec<KcpMask>`

返回该分类下所有变体。

### `category_from_code(code: &str)` → `Option<&'static str>`

映射代码到分类全名。

## 2. UI 流程 (main.rs)

### 2.1 `u_kcp_init` (修改)

显示 4 个分类按钮 + 返回按钮，代替当前平铺列表。

按钮：
```
[🔐 加密层 (2)] [🌀 混淆层 (3)]
[🎭 伪装层 (6)] [⚡ 扩展层 (3)]
[⬅️ 返回]
```

回调：`u_kcp_cat:{cat_code}`

### 2.2 `u_kcp_cat:{cat_code}` (新增)

进入分类子菜单，显示该分类下所有遮罩的简介和添加按钮。

示例（加密层）：
```
🔐 <b>加密层</b>

🔀 <b>mKCP Original</b>
轻量级XOR混淆，仅FNV1a校验

🔐 <b>mKCP AES-128-GCM</b>
AES-128-GCM认证加密，推荐首选

[✅ mKCP Original] [✅ mKCP AES-128-GCM]
[⬅️ 返回分类]
```

回调：选择遮罩 → `u_kcp_add:{code}`

### 2.3 `u_kcp_add:{code}` (微调)

保持现有逻辑不变，按钮"继续添加"指向 `u_kcp_more:{existing}`，"清空重选"指向 `u_kcp_init`。

### 2.4 `u_kcp_more:{existing}` (修改)

改为 4 个分类按钮导航，隐藏已满的分类。

消息顶部显示当前遮罩栈，按钮：
```
[🔐 加密层 (1)] [🌀 混淆层 (3)]
[🎭 伪装层 (6)] [⚡ 扩展层 (3)]
[✅ 完成配置] [🗑️ 清空重选]
```

回调：`u_kcp_mcat:{existing},{cat_code}`

### 2.5 `u_kcp_mcat:{existing},{cat_code}` (新增)

与 `u_kcp_cat` 类似，但：
- 消息顶部显示当前遮罩栈
- 已添加的类型显示为 ☑️ 已添加 禁用按钮
- 添加按钮回调：`u_kcp_push:{existing}:{code}`
- 底部有 "⬅️ 返回分类"、"✅ 完成配置"、"🗑️ 清空重选"

### 2.6 删除 `u_kcp_sel:{code}`

不再需要，简介直接在分类子菜单中显示。

### 2.7 其余处理器不变

`u_kcp_push`、`u_kcp_done`、`u_kcp_ip`、`u_kcp_ok` 保持原样。

## 3. 回调数据长度

所有回调均远低于 Telegram 64 字节限制。

## 4. 边界情况

- 某分类所有类型已添加时，隐藏该分类按钮
- 仅剩1层空间时正常提示