# KCP FinalMask: 对齐上游 XTLS/Xray-core 设计文档

**Status:** Draft
**Date:** 2026-06-16
**Scope:** 修改 2 个 Rust 源文件 + 更新 5 个测试

## 问题

当前 Rust 实现 (`kcp.rs`, `kcp_mask.rs`) 生成的 KCP FinalMask JSON 配置与上游 XTLS/Xray-core 不兼容。三个结构差异导致 Xray-core 拒绝或错误解析生成的配置。

## 上游参考

| 参考来源 | 关键发现 |
|----------|---------|
| `transport/internet/config.proto` (XTLS/Xray-core) | `udpmasks` 是 StreamConfig 第 10 个字段, JSON tag `"udpmasks,omitempty"` — 直接在 streamSettings 层级 |
| XTLS/Xray-core discussion #716 (VMessAEAD 链接标准 §4.3.20) | `fm` 分享链接参数携带原始 JSON 数组 — 无 `{"udp": [...]}` 包装 |
| `transport/internet/finalmask/salamander/config.pb.go` | Config 结构体有 `Password string json:"password,omitempty"` — 字段在 TypedMessage 层级，不在 `settings` 内 |
| `common/serial/typed_message.proto` | TypedMessage = `type` (完整 protobuf 类型名) + `value` (bytes) |

## 三个差异（当前 → 修复后）

### 1. JSON 键路径: `finalmask.udp` → `udpmasks`

```json
// 当前（错误）
{"streamSettings": {"finalmask": {"udp": [...]}}}

// 修复后（正确，按 config.proto）
{"streamSettings": {"udpmasks": [...]}}
```

位置: `build_kcp_inbound()` in `kcp.rs`

### 2. 类型名称: 短名 → 完整 protobuf 路径

```json
// 当前（错误）
{"type": "salamander"}

// 修复后（正确，按 TypedMessage proto）
{"type": "xray.transport.internet.finalmask.salamander.Config"}
```

位置: `as_json()` in `kcp_mask.rs`

类型名映射:

| 短名 | 完整类型路径 |
|------|------------|
| `salamander` | `xray.transport.internet.finalmask.salamander.Config` |
| `noise` | `xray.transport.internet.finalmask.noise.Config` |
| `sudoku` | `xray.transport.internet.finalmask.sudoku.Config` |
| `xdns` | `xray.transport.internet.finalmask.xdns.Config` |
| `xicmp` | `xray.transport.internet.finalmask.xicmp.Config` |
| `realm` | `xray.transport.internet.finalmask.realm.Config` |
| `mkcp-legacy` | `xray.transport.internet.finalmask.mkcp.Header` |

> **注意:** 确切的类型字符串需在实现时对照 Xray-core TypeRegistry 验证。以上格式遵循 salamander 的 `config.pb.go` 中的惯例 (`xray.transport.internet.finalmask.salamander.Config`)。

### 3. 字段扁平化: 移除 `settings` 包装

```json
// 当前（错误 — 多余嵌套）
{"type": "salamander", "settings": {"password": "sekret"}}

// 修复后（正确 — 扁平，按 salamander config.pb.go）
{"type": "xray.transport.internet.finalmask.salamander.Config", "password": "sekret"}
```

位置: `as_json()` in `kcp_mask.rs`。每个变体的字段上移一层。

### 客户端链接格式

```json
// 当前（错误）
&fm={"udp":[{"type":"salamander","settings":{...}}]}

// 修复后（正确，按 discussion #716 §4.3.20）
&fm=[{"type":"xray.transport.internet.finalmask.salamander.Config","password":"sekret"}]
```

位置: `generate_kcp_client_link()` in `kcp.rs`

## 修改文件列表

| 文件 | 修改内容 |
|------|---------|
| `rust/aegis/src/core/xray/kcp_mask.rs` | `as_json()` — 扁平化字段, 完整类型路径。更新 5 个测试的期望值。 |
| `rust/aegis/src/core/xray/kcp.rs` | `build_kcp_inbound()` — `finalmask.udp` → `udpmasks`。`generate_kcp_client_link()` — 移除 `{"udp": []}` 包装。 |

## 测试更新

`kcp_mask.rs` 中 5 个测试函数需要更新期望 JSON 字符串:

- `test_mkcp_legacy_as_json`
- `test_salamander_with_packet_size`
- `test_xdns_new_format`
- `test_xicmp_new_format`
- `test_realm_as_json`

每个测试的 `expected` 字符串改为新扁平格式和完整类型路径。

## 验证

```bash
cd rust/aegis
cargo test                    # 422 tests 基线, 预期: 全部通过
cargo clippy                  # 预期: 0 警告
cargo fmt                     # 预期: 干净
```

## 影响评估

- 2 个源文件 ~30 行变化
- 5 个测试期望值更新
- 无 API 变更，纯 JSON 格式修正
- 对当前代码生成的现有 KCP 配置不向后兼容
