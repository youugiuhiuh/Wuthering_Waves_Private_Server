# SPEC：核心升级版本检查 + 预检 + 健康检查 + 自动回滚（Xray-core / Sing-box）

状态：**已实现并经真机验证** ｜ 提交：`c678871` `dd3e4d5` `678aa07` `734f4a0` `b6f4c7a`

## 目标与结论

补齐 `wwps-core`（= Xray-core）与 `wwps-box`（= Sing-box）升级流程的三个缺口：

| 原缺口 | 现状 |
|---|---|
| 无版本检查，已是最新仍完整走「下载 → 备份 → 替换 → 重启」 | 已加短路，一致则零副作用返回 |
| 备份只写不读，全代码库无 restore 实现 | 已实现自动回滚，备份有了消费方 |
| 备份目录无限堆积 | 保留最近 3 份，与升级成败无关 |

## 已确认事实

| 项 | 事实 |
|---|---|
| 上游来源 | Xray-core 默认 `XTLS/Xray-core`（`core/paths.rs:68-69` → `core_upgrade.rs:26-27`）；Sing-box `SagerNet/sing-box` 走 `fetch_prerelease` |
| 本机二进制 | `/etc/wwps/wwps-core/wwps-core`、`/etc/wwps/wwps-box/wwps-box`，即上游二进制改名，其 `version` 输出即上游版本 |
| `xray version` | `Xray 26.9.30 (Xray, Penetrates Everything.) b26a91d (go1.27.1 linux/amd64)` + `A unified platform for anti-censorship.` |
| `sing-box version` | 首行 `sing-box version 1.15.0-alpha.10`，其后为 Environment/Tags/Revision/CGO 多行 |
| unit 模板 | 两核心均 `Type=simple` + `Restart=always` + `RestartSec=5` |
| 预检命令 | Xray `run -test -confdir <dir>`（官方文档确认）、Sing-box `check -C <dir>` |

## 实现

### 1. 版本短路（`core/utils.rs`）
`normalize_version_tag` / `is_same_version`——**只判相等**，不做大小比较：预发行通道的版本语义由上游 tag 决定，bot 不应自行判定「本地更新」而拒绝管理员显式指定的升级。
- Xray：新增 `parse_xray_version_from_output` + `WwpsCoreUpgradeManager::current_version`
- Sing-box：复用既有 `current_version` / `parse_version_from_output`
- 两侧版本读取均加 10s 硬超时 + `kill_on_drop(true)`；**版本未知一律 fail-open 放行**

### 2. 防线一：替换前预检（`core/system/core_health.rs::run_config_preflight`）
用新二进制验证现网配置，失败则**完全不碰现网二进制**（此时备份未产生、服务未重启）。
- `is_preflight_unsupported` 只扫 **stderr**：Xray 把 warning 与 `Configuration OK.` 打到 stdout，若把 stdout 纳入判定，一份含 `usage:` 字样的配置就能让真实配置错误被误放行
- flag 被上游移除 → `Unsupported` → **fail-open 继续**，只靠防线二兜底

### 3. 防线二：跨崩溃周期健康检查（`classify_samples` / `wait_for_health`）
`is-active` 全程 active **且** `NRestarts` 无增量才算 `Healthy`；窗口 `3s × 5 = 15s`，必须大于 `RestartSec=5`，否则在崩溃重启发生前就判健康。
`Unknown`（探测工具故障）**不触发回滚**——探测坏了不等于核心坏了。

### 4. 自动回滚（`restore_backup`）
回滚目标固定为**本次升级刚产生的** `backup_path`（不得重新扫目录取 latest，否则并发/残留会回滚到错误版本）；沿用「暂存 `.new` → `chmod 0755` → 原子 rename」避开 ETXTBSY；恢复 geo 数据失败仅记日志；**恢复后重跑完整健康窗口复验**，不健康即如实报失败并保留现场。
Sing-box 侧此前完全没有备份，现补上 `/etc/wwps/wwps-box/backup/` 与同构两道防线。

### 5. 备份裁剪
`select_backups_to_delete` 只认「prefix + 可解析时间戳」，外来文件一律不碰，最旧优先删除。
裁剪紧跟 `backup_current_core` / `backup_binary`，**不挂在成功分支末尾**——否则回滚路径提前 return 时永远裁不到。

### 6. 消息网关
预检通过 / 预检不可用 / 预检失败（含提炼根因）/ 健康检查中 / 无法完成 / 正在回滚 / 回滚成功（含判定）/ 回滚失败需人工介入 / 已清理 N 份，zh+en+ja 共 16 键。

## 真机验证结论（`23.165.248.200`）

| 路径 | 结果 |
|---|---|
| 版本短路 | ✅ 两次，sha256 与 `NRestarts` 均未变，零副作用 |
| 正常升级 | ✅ 26.9.9 → 26.9.30，sha256 精确回到官方二进制 |
| 自动回滚 | ✅ 崩溃循环 → `Inactive` → 回滚到本次备份 → 复验通过；回滚后旧核心成功 bind 并持续监听 |
| 备份裁剪 | ✅ 6 份 → 3 份，287M → 187M |
| Sing-box 侧 | 代码同构已实现，见下方「验证状态」 |

验证手段：`Type=simple` 一次性 dummy unit 实测（crash-loop 时 `is-active=activating`、`NRestarts` 0→2）、健康服务 15s 采样 5/5 active 无误伤、沙箱注入非法配置确认 `exit 23` 与 stderr 无误标、端口占用看门狗制造「bind 失败但预检通过」。

## 过程中发现并修复的三个缺陷

均**只有真机验证才暴露**，单元测试全绿时它们都还活着：

1. **前缀不匹配**：`DEFAULT_BACKUP_PREFIX = "wwps-core-backup"` 无尾部横杠，剥前缀后剩 `-2026...` 被当非法时间戳过滤 → 一份都不删。单元测试用着手写带横杠前缀，与生产常量不一致。
2. **裁剪位置错误**：挂在成功分支末尾，回滚路径提前 return 走不到 → 回滚后备份反而变多。与 1 叠加，只修 1 会误以为修好了。
3. **单次 `is-active` 挡路**：`verify_service_active().await?` 在健康检查之前，失败即 `return Err`，**回滚代码永不执行**，坏二进制留原地。已改为只记日志。

## 已知边界

- **预检覆盖不到「字段被移除但上游未加校验」的情况**：实测注入未知字段 `bogusRemovedField` 后 Xray 仍 `Configuration OK.`（Go `encoding/json` 静默忽略未知字段）。上游显式加了报错的移除（如 `legacy reverse has been removed`）能拦到；纯改名/移除则拦不到。
- **「进程健康但功能已坏」拦不到**：REALITY `minClientVer` 默认值变更这类问题核心能起、流量不通，两道防线都无感知。
- Sing-box 走预发行通道，升级目标可能是 alpha。

## 验证状态

- Xray-core：四条路径全部真机验证通过
- Sing-box：代码与 Xray 侧同构；`check -C` 预检命令已在真机确认可用（`1.14.2` 对现网配置 exit 0），完整升级链路待最后一轮确认

## 未纳入本轮

- 手动回滚菜单入口（备份列表 + 指定回滚）
- 断电 / `kill -9` 后的升级事务日志恢复（`.upgrade-journal`）
- `conf/` 配置目录备份（升级不触碰 conf，刻意不扩大范围）
- Sing-box 备份目录的常规定时清理（现由每次升级触发）
