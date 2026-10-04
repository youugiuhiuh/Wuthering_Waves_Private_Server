# Plan：核心升级版本检查（对齐 Bot 自更新行为）

关联 SPEC：`SPEC.md`（已收窄：不含回滚）

## 行为约定
- `本地版本 == 目标 tag` → 回复「已是最新版本」并结束，**不下载、不替换、不重启**
- 不一致 → 走原有升级流程不变
- 本地版本读取失败（无二进制 / 输出无法解析 / 执行失败 / 超时）→ **放行升级**（fail-open），行为退化为现状

## 任务

### T1 版本比较纯函数（`src/core/utils.rs`）
- 文件：`rust/aegis/src/core/utils.rs`
- 新增：`pub fn normalize_version_tag(tag: &str) -> &str`（trim + 去前导 `v`）、`pub fn is_same_version(local: &str, remote_tag: &str) -> bool`
- 测试：表驱动，`v26.9.30`/`26.9.30` 同；`26.9.30` vs `26.9.31` 不同；`1.8.4` vs `1.8.4-rc1` 不同；空串不同；大小写/空白容忍
- 验收：`cargo nextest run core::utils` 全绿

### T2 Xray 版本解析 + 读取（`src/core/system/core_upgrade.rs`）
- 新增：`pub fn parse_xray_version_from_output(out: &str) -> Option<String>`（首个 `Xray ` 行后第一个 token）
- 新增：`pub async fn current_version(&self) -> Option<String>`（执行 `{install_dir}/wwps-core version`，超时 10s）
- 测试：用真实输出样本
  ```
  Xray 26.9.30 (Xray, Penetrates Everything.) b26a91d (go1.27.1 linux/amd64)
  A unified platform for anti-censorship.
  ```
  空输出 / 无 `Xray ` 前缀 / `Xray ` 后无 token → `None`
- 验收：`cargo nextest run core_upgrade` 全绿

### T3 Xray 升级短路
- `run_upgrade`：`fetch_release` 之后、`download_release` 之前比较；相同 → `edit_message` 为 `upgrade.core_already_latest`（含本地版本与 tag）并 `return Ok(())`
- 验收：手工/代码走查确认下载调用不可达

### T4 Sing-box 升级短路
- `singbox/upgrade.rs::run_upgrade`：`fetch_release` 之后复用既有 `current_version()` + `parse_version_from_output` 短路，键 `menu.singbox_upgrade_already_latest`
- 验收：同上

### T5 i18n
- `src/resources/i18n/{zh,en,ja}.yml` 新增 `upgrade.core_already_latest`、`menu.singbox_upgrade_already_latest`
- 验收：三语言键齐全，`cargo nextest run` 中 i18n 相关测试通过

### T6 质量门禁
- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo nextest run`
- 全绿方可交付

## 不做
- 回滚机制（用户明确暂缓）
- `semver` crate、大小比较/降级拦截、备份裁剪、菜单只读版本展示
