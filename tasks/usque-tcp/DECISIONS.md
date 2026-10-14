# Spec 2 已确认决策（全部有实证支撑）

| # | 决策点 | 选择 | 依据 |
|---|---|---|---|
| Q1 | 与现有 WireGuard WARP 的关系 | **并存，按探测结果自动选** | `ProbeConclusion` 是现成判据 |
| Q2 | SOCKS 模式 | **`socks --http2`** | 实测 `l4-socks` 不支持 `--http2`，本机房必死 |
| Q3 | 二进制供应链 | **pin 版本 + pin sha256** | 无签名，只有同仓库 checksums.txt |
| Q4 | 断连兜底 | **主动健康检查 + 强杀重启** | 上游有卡死死锁报告（issue #49 / PR #103） |
| Q5 | 用户流量接入 | **复用 WarpScope（Domain/Global）** | 与 Spec 1 无缝衔接，Global 终于真生效 |
| Q6 | TG 菜单 | **不做新菜单** | 由 installer 自动部署，状态并入现有 `a_warp_status` |
| Q7 | 部署方式 | **独立 systemd unit** | 与 wwps-core / wwps-box 结构一致 |
| OQ1 | 出口选择判据来源 | **(a) 探测缓存，TTL 10 分钟** | 避免在 `reload_core()` 路径上引入 3s 延迟与新失败面 |
| OQ2 | 探测缓存位置 | **(a) 内存** | 探测很轻，重启重探代价可接受；不引入状态一致性问题 |
| OQ3 | `FullyBlocked` 时 | **(a) 保持 direct 出口 + bot 告警** | 不启用 WARP，避免黑洞 |
| OQ4 | 纳入 `self_destruct` | **(a) 加入 `DESTRUCT_SERVICES`** | 否则残留 unit 可能被 systemd 拉起 |
| OQ5 | `WARP_SCOPE_ENFORCE` | **(b) 移除环境变量**，改为「出口引擎可用时 Global 即生效」 | 护栏的存在理由是「出口不可用」；现出口可用，用运行时可用性取代更准确 |
| **NEW** | **卸载行为** | **卸载时同时检查并卸载 WARP(WireGuard) 与 usque** | 用户明确要求（见下） |

---

## 新增需求：卸载必须同时清理两套实现

**用户原话**：「我希望那个卸载 WARP 功能可以同时检查 WARP 和 usque 并卸载」

### 现状（Spec 1 的 `WarpInstaller::uninstall()`，`core/xray/installer.rs`）

```rust
pub async fn uninstall() -> Result<()> {
    // Just remove the account config and the routing config
    let _ = fs::remove_file(warp_paths::ACCOUNT_FILE).await;   // warp_account.json
    let _ = fs::remove_file(warp_paths::ROUTING_FILE).await;   // 10_warp_routing.json
    MaintenanceManager::reload_core().await?;
    Ok(())
}
```

只清 WireGuard 的两个文件。Spec 2 落地后会多出 usque 的一整套资产，**只清一半会留下"幽灵状态"**：

- `wwps-usque.service` 仍在、仍被 systemd 拉起
- `/etc/wwps/usque/` 目录与已注册的 WARP 账号仍在
- Xray 配置里指向 `127.0.0.1:1080` 的 socks outbound 可能仍存在（若 outbound 生成独立于账号文件）

### 要求的行为

卸载时必须：

1. **探测两套实现的实际安装状态**（不能靠"我以为装了什么"）
2. **停用并移除 systemd unit**（先 `stop` 再 `disable`，最后删 unit 文件 + `daemon-reload`）
3. **删除各自的数据文件**
   - WireGuard：`warp_account.json`、`10_warp_routing.json`
   - usque：`/etc/wwps/usque/config.json`、（可选）二进制
4. **无论哪一套缺失都不得失败**（幂等）：只装了 usque 时应能正常卸载；两套都没装时应报告"无 WARP 实现"而非报错
5. **清理后重载 Xray 核心**，使路由配置回到无 WARP 状态
6. **返回可读的结果**：本次实际卸载了哪些（用于 bot 文案）

### 判定语义（待实现时确认细节）

| 状态 | 期望行为 |
|---|---|
| 两套都装 | 全部卸载，报告两条 |
| 只装 WireGuard | 卸载 WireGuard，报告一条，**不报错** |
| 只装 usque | 卸载 usque，报告一条，**不报错** |
| 都没装 | 报告「未检测到 WARP 实现」，**返回成功而非错误**（幂等） |
| unit 存在但进程未运行 | 仍应移除 unit 文件 |

### 与 Q6「不做新菜单」的关系

Q6 决定不加新菜单。卸载按钮沿用现有 `a_warp_uninstall`，
但**其语义从「卸载 WireGuard WARP」扩大为「卸载所有 WARP 实现」**。
i18n 文案需要相应调整（现有键：`warp.uninstall_service` / `warp.uninstall_confirm` /
`warp.uninstalling` / `warp.uninstall_success` / `warp.uninstall_fail`）。

---

## 硬约束（不可违反）

1. **必须用 `socks --http2`，禁止用 `l4-socks`**
   原因：`l4-socks` 无 `--http2`，是 QUIC-only，在本机房（出站 UDP 被封）下吞吐实测 ~1 KB/s。
   上游 README 的「l4 更快」建议**在本部署下不成立**——必须在代码注释里写明，
   否则未来会被"优化"成 l4-socks 并把节点搞挂。

2. **健康检查必须能对付「卡死但不退出」**
   实测上游行为：空闲 `Tunnel idle. Waiting for outbound activity before reconnecting...`
   上游 Known Issues：空闲会被远端以 `H3_NO_ERROR` 断开。
   上游 issue #49：用户报告「用一阵子卡死后必须手动 kill 重启」。
   上游 PR #103：修过一个 tunnel 死锁。
   → systemd `Restart=always` 只在进程**退出**时生效，对卡死无效。

3. **`register` 必须处理交互式 ToS 确认**
   实测：不写入 `y` 会得到 `failed to read user input: EOF`。

4. **usque 配置格式与现有 WireGuard 完全不兼容**
   ```
   warp_account.json (WireGuard): private_key / public_key / reserved / client_id
   usque config.json  (MASQUE)  : private_key / access_token / id / endpoint_h2_v4 / ...
   ```
   不能共用文件语义；`reserved` 是 WireGuard 专有概念。

5. **pin 的版本与 sha256 必须硬编码**
   不能用 latest。已实测校验可行：
   ```
   v4.2.1  usque_4.2.1_linux_amd64.zip
   sha256 = 4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313
   ```

6. **Xray `socks` outbound 的形状是扁平的，不是 `servers[]`**
   ```json
   { "protocol": "socks", "settings": { "address": "127.0.0.1", "port": 1080 } }
   ```
   与 `vless`/`vmess`/`trojan`/`wireguard` 的 `settings.servers[]` **不同**。
   写错会静默失效（Xray 对未知字段宽容）。必须为此写断言测试。

---

## 已知实测数据（供设计与测试断言使用）

| 项 | 值 |
|---|---|
| 版本 | v4.2.1 |
| 资产名 | `usque_4.2.1_linux_amd64.zip` |
| sha256 | `4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313` |
| 二进制大小（解压后） | 12,906,658 B |
| 隧道延迟 | ≈ 39 ms |
| 隧道吞吐 | 9.2 – 11.8 MB/s |
| 隧道时 CPU / RSS | 22.4% / 19.1 MB |
| 默认监听 | `127.0.0.1:1080`（可用 `-p` 改） |
| 配置文件 | `config.json`（可用 `-c` 改） |
| 健康判据 | 走隧道请求 `cdn-cgi/trace` 且含 `warp=on` |
