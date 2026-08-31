# Bot 自更新加固设计（Spec）

> 状态：已批准（方向 C 骨架 + A 快速收益）
> 日期：2026-08-31

## 背景

`aegis`（Rust Telegram/Discord/Matrix 管理 bot）通过聊天回调 `a_upgrade` 远程自更新自身二进制。对比 Go/installer 的事务性部署（temp 目录下载 → minisign/sha256 强制校验 → `systemctl stop/restart`），当前 Rust 自更新存在 6 个缺陷：

1. **信任链降级（安全）**：`download_minisign(...).await.ok().flatten()` 把签名下载失败静默吞掉 → 降级为仅 SHA256（SHA256 与二进制同源，可被伪造）。同库 `core_upgrade.rs` 的语义是「asset 存在 → 强制验证，失败即中止」。
2. **下载超时过短**：reqwest total timeout 60s（含 body 流），aegis 约 53MB，慢网必失败；Go 用 10min。
3. **固定临时文件名**：`current_exe.with_extension("update")` 固定路径 → 并发触发两次升级互相覆盖。
4. **无版本比较**：即使已是最新也会重新下载替换。
5. **无并发锁**：重复点击 `a_upgrade` 可并发执行两个下载替换任务。
6. **部署不可测、无回滚点**：`self_replace` + `std::process::exit(0)` 不可注入、不可测试；替换前无冒烟验证，失败无 `.bak` 回滚点；非 systemd 环境（docker/manual）裸 exit 后无人拉起。

## 目标

- minisign 语义与 `core_upgrade.rs` 对齐：asset 存在 → 下载/验证失败即中止；asset 缺失 → 跳过（sha256 仍强制）。
- 下载超时 60s → 10min total + 10s connect。
- 下载到同文件系统唯一临时文件，校验通过后原子替换。
- 升级前版本比较（同版本跳过）。
- 升级并发锁（O_EXCL + stale pid 检测）。
- 部署流程可注入 `DeployStrategy` trait → 可单测/集成测试；生产实现含：冒烟测试（`--version`）、`.bak` 备份、`fs::rename` 原子替换、systemd/reexec 双策略。

## 明确不做（YAGNI）

- 断点续传、下载镜像多源（仅 API base 已支持 mirrors，下载仍单 URL）。
- 替换后看门狗失败告警（pre-flight + systemd Restart + `.bak` 人工恢复点已覆盖主要场景，残余风险写入本 spec）。
- 有效 minisign 签名密钥的生成/测试夹具（无私钥；集成测试仅覆盖「asset 缺失跳过」与「asset 存在但下载失败中止」，有效签名路径由真实 release 人工验证）。

## 设计

### 1. 信任链（minisign 强制）

`upgrade.rs::fetch_latest_release_from_repo` 中：

```rust
// 现状
let minisig = self.download_minisig(&release.assets, &asset.name).await.ok().flatten();
// 目标
let minisig = self.download_minisig(&release.assets, &asset.name).await?;
```

`download_minisig` 保持「无 asset → `Ok(None)`」；asset 存在但下载/解析失败 → `Err` 向上传播 → 升级中止。

### 2. 下载超时

`new()` 构建 client 时：

```rust
reqwest::Client::builder()
    .connect_timeout(Duration::from_secs(10))
    .timeout(Duration::from_secs(600))
    .build()?
```

常量：`CONNECT_TIMEOUT_SECS = 10`、`REQUEST_TIMEOUT_SECS = 600`。

### 3. 唯一临时文件

`download_with_progress` 不再写 `current_exe.with_extension("update")`，改为在当前 exe 同目录（保证与最终目标同文件系统、`fs::rename` 原子）：

```rust
fn unique_update_path(exe_dir: &Path) -> PathBuf {
    exe_dir.join(format!(".aegis-update-{}-{:x}.tmp", std::process::id(), now_nanos()))
}
```

下载完成后原路径保留给 `finalize_install` 使用；任何校验失败时 `fs::remove_file` 清理。

### 4. 版本比较

```rust
fn is_current_version(tag: &str) -> bool {
    tag.trim().trim_start_matches('v') == env!("CARGO_PKG_VERSION")
}
```

`run()` 在 fetch 成功后、下载前调用：同版本 → 发送「已是最新」并返回 `Ok(())`。

### 5. 并发锁

`UPGRADE_LOCK_FILE`（默认 `/etc/wwps/aegis/upgrade.lock`，env `AEGIS_UPGRADE_LOCK_FILE` 可覆盖）：

- `OpenOptions::create_new(true)` 独占创建，写入 pid。
- 已存在 → 读 pid，`/proc/<pid>` 不存在则视为 stale，删除后重试一次；否则报「升级已在进行」。
- RAII 守卫 `UpgradeLock`（Drop 时删除锁文件）。
- 锁在 `run()` 最开头获取，全流程持有。

### 6. 可注入部署（DeployStrategy trait）

```rust
#[async_trait]
pub trait DeployStrategy: Send + Sync {
    /// 使新二进制生效；返回后新进程将被启动。
    async fn deploy(&self, update_path: &Path, current_exe: &Path) -> Result<()>;
    /// 生产实现为 true（替换后 sleep + exit(0)）；测试 mock 为 false。
    fn needs_exit(&self) -> bool { true }
}
```

`UpgradeManager` 持有 `Arc<dyn DeployStrategy>`（默认 `ProductionDeploy`），提供 `with_deploy()` 注入。

生产实现 `ProductionDeploy::deploy`（顺序关键）：

1. `smoke_test_binary(update_path)`：执行 `update_path --version`，要求 exit 0 且 stdout 含 `aegis`。
2. `fs::copy(current_exe, current_exe.bak)` 备份。
3. `fs::rename(update_path, current_exe)` 原子替换（Linux 下 rename 覆盖运行中二进制合法，运行进程持有旧 inode）。
4. 重启策略（env `AEGIS_UPGRADE_STRATEGY`：`systemd` / `reexec` / `auto`）：
   - `systemd`：什么都不做（`finalize_install` 随后 `exit(0)`，由 systemd `Restart=always` 拉起）。
   - `reexec`：`Command::new(current_exe).args(env::args().skip(1)).process_group(0).spawn()` 分离子进程再返回。
   - `auto`：`is_systemd_managed()`（`INVOCATION_ID` env 或 `/run/systemd/system` 存在）→ systemd，否则 reexec。

### 7. `finalize_install` 重构

```rust
async fn finalize_install(&self, artifact, update_path, adapter, target, progress_msg_id) -> Result<()> {
    let current_exe = std::env::current_exe()?;
    self.deploy.deploy(update_path, &current_exe).await?;
    fs::remove_file(update_path).await.ok();   // rename 后 update 路径已不存在，容错清理
    self.write_upgrade_flag(&artifact.tag_name).await?;
    adapter.send_message(target, MessageContent { text: t!("upgrade.bot_updated", ...), markup: None }).await?;
    if self.deploy.needs_exit() {
        sleep(Duration::from_secs(2)).await;
        std::process::exit(0);
    }
    Ok(())
}
```

### 8. 可配置化（测试与镜像支持）

| env | 默认 | 用途 |
|---|---|---|
| `AEGIS_RELEASE_API_BASES`（逗号分隔） | `https://api.github.com` | Release API base，wiremock 测试用 |
| `AEGIS_UPGRADE_FLAG_FILE` | `/etc/wwps/aegis/upgrade.flag` | 升级成功 flag |
| `AEGIS_UPGRADE_LOCK_FILE` | `/etc/wwps/aegis/upgrade.lock` | 并发锁 |
| `AEGIS_UPGRADE_STRATEGY` | `auto` | 重启策略 |

## 测试策略

- **单元**（`upgrade.rs` 内 `#[cfg(test)]`）：`is_current_version`、`unique_update_path` 唯一性/同目录、锁（stale/占用/释放）、`smoke_test_binary`（临时脚本成功/失败/不可执行）、超时常量。
- **集成**（`tests/upgrade_integration.rs`，wiremock + `MockBotAdapter` + `MockDeployStrategy`）：
  - happy path：release JSON + 二进制 asset（digest 匹配）→ `run()` 完整走通，deploy 被调用且收到正确路径。
  - minisig asset 存在但下载 500 → 中止（错误路径）。
  - minisig asset 缺失 → 跳过，正常完成（sha256-only）。
  - sha256 不匹配 → 中止，临时文件被清理。
- env 相关测试用 `serial_test`（已存在 dev-dep）+ 唯一临时路径；edition 2024 下 `env::set_var` 为 `unsafe`。

## 残余风险（记录）

- 新版本启动失败时无自动回滚/告警：pre-flight 冒烟 + systemd `Restart=always` 覆盖主要场景，`aegis.bak` 提供人工恢复点。
- 下载仍为单 URL，无多镜像。
