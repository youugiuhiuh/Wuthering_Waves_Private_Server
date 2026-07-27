# Aegis: Xray-core 预发行版 + 一键部署 mKCP

**Date:** 2026-07-27
**Status:** Design (approved)

---

## 变更1: Xray-core 默认使用预发行版

### 当前行为
`fetch_release(None)` 请求 `/releases/latest`，始终获取最新稳定版。

### 目标行为
`fetch_release(None)` 改为请求 `/releases?per_page=20`，筛选 `prerelease: true` 的第一条。

### 涉及文件

| 文件 | 变更 |
|---|---|
| `src/core/network/release_api.rs` | `ReleaseResponse` 结构体添加 `prerelease: bool` 字段 |
| `src/core/system/core_upgrade.rs` | `fetch_release(None)` 改为请求 tag 列表 + 筛选 prerelease |

### release_api.rs 变更

`ReleaseResponse` 结构体加重命字段：
```rust
#[derive(Debug, Deserialize)]
pub struct ReleaseResponse {
    pub tag_name: String,
    pub prerelease: bool,         // 新增
    pub assets: Vec<ReleaseAsset>,
    pub body: Option<String>,
}
```

### core_upgrade.rs 变更

`fetch_release(None)` 分支：
- 当前：`GET /repos/{owner}/{repo}/releases/latest`
- 改为：`GET /repos/{owner}/{repo}/releases?per_page=20`
- 从响应中取第一条 `prerelease == true` 的 release
- 若无预发行版则 fallback 到第一条（或保持报错）

---

## 变更2: 一键部署新增 mKCP 步骤

### 当前流程 (8步)
1. VPS调优
2. Xray-core安装
3. PQ密钥
4. XHTTPx20
5. Visionx20
6. Sing-box
7. Hysteria2x3
8. 安全加固

### 目标流程 (10步)
1. VPS调优
2. Xray-core安装（预发行版）
3. PQ密钥
4. XHTTPx20
5. Visionx20
6. Sing-box
7. Hysteria2x3
8. **mKCP + DNS伪装 x5**
9. **mKCP + 微信伪装 x5**
10. 安全加固

### 涉及文件

| 文件 | 变更 |
|---|---|
| `src/shared/handlers/ops.rs` | `handle_one_click()` 在步骤7后插入步骤8、9 |
| `src/shared/i18n.rs` 或 i18n 配置 | 新增步骤描述字符串 |

### ops.rs 变更

在 `handle_one_click()` 中，Hysteria2x3 步骤完成后、安全加固前，插入：

```rust
// Step 8: mKCP DNS伪装 x5 — 调用 batch_create_kcp(5, IpVersion::V4, &["mld"])
// Step 9: mKCP 微信伪装 x5 — 调用 batch_create_kcp(5, IpVersion::V4, &["mlw"])
```

步骤总数常量从 8 改为 10，安全加固步骤号变为 10。

### i18n 字符串

新增：
- `ops.deploy_step_mkcp_dns` = "Creating 5 mKCP + DNS camouflage inbounds"
- `ops.deploy_step_mkcp_wechat` = "Creating 5 mKCP + WeChat camouflage inbounds"
- `ops.deploy_total_steps` = "10" (从 "8" 更新)

### 不变文件

- `kcp_mask.rs` — mld / mlw 已存在
- `kcp.rs` — `batch_create_kcp()` 已支持任意 mask_codes
- `installer.rs` — 跟随 `core_upgrade.rs` 自动生效，无需改动

---

## 验证策略

1. `cargo check` — 编译通过
2. 单元测试 — `cargo test`
3. 无运行时回归 — 一键部署现有步骤不受影响
4. mKCP 步骤 — 确认 DNS / WeChat 配置 JSON 生成正确，端口分配不冲突
