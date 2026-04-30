# Xray KCP FinalMask 排序修复 & 分类删除管理

## 背景

对比 Xray-core 上游源码发现三个问题：

1. **FinalMask.udp 数组顺序反转**：当前 `canonical_order()` 把 XICMP 排在数组第一位（视为"最外层"），但 Xray-core 处理 udp 数组的语义是"第一=最内层（最靠近原始数据），最后=最外层（最靠近网络传输）"。整个排序完全反了。
2. **streamSettings JSON 字段顺序不符上游**：`build_kcp_inbound()` 中 `kcpSettings` 出现在 `security` 和 `finalmask` 之前，而上游 `StreamConfig` 的规范顺序是 `network → security → finalmask → kcpSettings`。
3. **删除管理缺分类筛选**：所有 `*_inbounds.json` 文件混在一起，无法按 Reality / XHTTP / KCP 类型筛选或删除。

## 方案：语义重构（方案 B）

### Section 1：KCP FinalMask.udp 排序修复

**文件**: `rust/tgbot/src/logic/config.rs`

重写 `sort_priority()` 为 innermost→outermost 语义（值越小越靠近数据、排在数组前面）：

| 遮罩 | 新 priority | 语义 |
|------|------------|------|
| Sudoku | 0 | 最内层，最靠近原始数据 |
| mKCP (Original/Aes128Gcm) | 10 | 加密层紧贴数据 |
| Salamander | 20 | 混淆层 |
| Header (Dns/Wechat/Srtp/Utp/Dtls/Wireguard) | 30 | 伪装头部 |
| Noise | 40 | 噪声填充 |
| XDNS | 50 | DNS 传输替换 |
| XICMP | 60 | 最外层，ICMP 传输替换 |

`canonical_order()` 保持升序排列不变，排序结果：
```
[Sudoku, mKCP, Salamander, Headers, Noise, XDNS, XICMP]
```

**`validate_stack()` 规则更新**：
- XICMP 必须在数组末尾（最外层）
- Sudoku 只能在数组开头（最内层）
- mKCP 加密层应在数组前部（紧贴数据）
- 移除"XICMP 必须第一个添加"的错误描述

**注释修正**：所有中文注释统一为 "innermost-first" 语义。

### Section 2：streamSettings JSON 字段顺序修复

**文件**: `rust/tgbot/src/logic/config.rs`（`build_kcp_inbound()`）

将 `streamSettings` 的 JSON 字段顺序调整为与上游 `StreamConfig` 一致：

```json
{
  "network": "kcp",
  "security": "none",
  "finalmask": { "udp": [...] },
  "kcpSettings": { "mtu": 1350, "tti": 50, ... }
}
```

### Section 3：按类型筛选的删除管理

#### 后端

**文件**: `rust/tgbot/src/logic/config.rs`

新增方法：
```rust
pub async fn list_inbound_files_by_proto(proto: Proto) -> Result<Vec<String>>
```
根据文件名前缀（`batch_reality_` / `batch_xhttp_` / `batch_kcp_`）过滤 `*_inbounds.json` 文件。

#### 前端

**文件**: `rust/tgbot/src/main.rs`

`m_del_cfg` 菜单增加类型筛选行：
```
📋 全部 | 🌐 Reality | ⚡ XHTTP | 📡 KCP
🧨 删除全部配置
➗ 按数量删除配置
🎯 指定配置删除
⬅️ 返回
```

新增回调路由，callback data 携带筛选类型：
- `cfg_filter:all` / `cfg_filter:reality` / `cfg_filter:xhttp` / `cfg_filter:kcp`
- `cfg_del_select:reality` — 列出 Reality 类型文件
- `cfg_del_count:xhttp` — 按数量删除 XHTTP 类型
- `cfg_del_all_confirm:kcp` — 确认清空 KCP 类型
- `cfg_del_all_confirm:all` / `cfg_del_all_confirm:` — 清空全部

所有删除操作（全部删除、按数量、指定文件）都基于当前筛选类型。

## 不涉及

- 不修改 `build_reality_vless_inbound()`（Reality/XHTTP 的 streamSettings 字段顺序）
- 不修改 sing-box 相关功能
- 不修改 KCP 客户端链接格式