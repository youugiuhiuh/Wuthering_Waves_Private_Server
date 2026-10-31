# SPEC: needrestart 配置改为追加式 override_rc

- 状态：待批准
- 模式：strict（涉及安全逻辑 / 升级链路正确性）
- 影响文件：1 个源码文件，1 个测试文件（同文件内 `mod tests`）

## 1. 问题

`AutoUpdateConfigurator::needrestart_config()` 生成的配置用**整体赋值**覆盖了
`$nrconf{override_rc}`：

```perl
$nrconf{restart} = 'a';
$nrconf{override_rc} = {
  qr(^unattended-upgrades\.service$) => 0,
};
```

`$nrconf{override_rc} = {...}` 在 Perl 中是整体替换，会**抹掉发行版自带的 50 条默认排除名单**。
`needrestart` 的判定逻辑（`/usr/sbin/needrestart:1105-1129`）按 hash 键遍历匹配，
被抹掉的键不再产生保护。

### 实测证据（Debian 13, needrestart 3.11-1，真机复刻源码判定逻辑）

| 服务 | 发行版默认 | 当前 aegis 配置 | 后果 |
| --- | --- | --- | --- |
| `dbus.service` | skip | **RESTART** | 触发 `SIGTERM received, will stop` |
| `dbus-broker.service` | skip | **RESTART** | 同上 |
| `apt-daily.service` | skip | **RESTART** | 升级驱动 oneshot 被重启 |
| `apt-daily-upgrade.service` | skip | **RESTART** | 升级驱动 oneshot 被重启 |
| `cron-daily.service` | skip | **RESTART** | systemd-cron oneshot 被重启 |
| `NetworkManager.service` | skip | **RESTART** | 网络抖动风险 |
| `override_rc` 条目数 | 50 | **1** | 丢失 49 条保护 |

`apt-daily.service` / `apt-daily-upgrade.service` 被重启是本缺陷的**最严重后果**：
它们是 `unattended-upgrades` 的实际驱动 oneshot，收到 SIGTERM 会中断升级链条。

### 与 2026-10-10 事故的关系

- `5b5e99c Init` 引入 `restart='a'`（无 `override_rc`）
- 2026-10-10 两台生产机积压 49/57 个安全更新
- `9b58241` 加入 `override_rc` 保护哨兵 `unattended-upgrades.service`

`9b58241` 的修复**有效但不完整**：它保住了哨兵，但整体覆盖的写法同时把
`apt-daily` / `apt-daily-upgrade` 两个驱动 oneshot 从 skip 翻成了 RESTART。

## 2. 目标

保留 `9b58241` 的保护意图，同时恢复发行版默认的 50 条排除名单。

**非目标**（明确不做）：

- 不修改 `$nrconf{restart} = 'a'`（保留 `9b58241` 作者的选择；改成 `'i'` 属独立决策）
- 不改 `configure_needrestart()` 的写入路径（仍写 `/etc/needrestart/needrestart.conf`）
- 不迁移到 `conf.d/` 片段机制
- 不处理 `dpkg -V` 显示的 conffile 漂移（属独立问题）

## 3. 设计

### 3.1 配置变更

```perl
# 改前
$nrconf{restart} = 'a';
$nrconf{override_rc} = {
  qr(^unattended-upgrades\.service$) => 0,
};

# 改后
$nrconf{restart} = 'a';
$nrconf{override_rc}->{qr(^unattended-upgrades\.service$)} = 0;
```

`->{...} = 0` 是对**已存在 hash 的单键赋值**，不替换整个 hash。
发行版 50 条默认保留，`unattended-upgrades.service` 仍在保护名单内
（发行版默认本就含该键，显式赋值幂等）。

### 3.2 代码变更点

`rust/aegis/src/core/system/operations.rs`

```rust
pub(crate) fn needrestart_config() -> String {
    r#"$nrconf{restart} = 'a';
$nrconf{override_rc}->{qr(^unattended-upgrades\.service$)} = 0;
"#
    .to_string()
}
```

## 4. 测试设计（TDD）

RED 先行：先改测试使其失败，再改实现使其通过。

### 4.1 改写既有测试 `test_needrestart_config_defers_unattended_upgrades_service`

保留原有两条断言（`override_rc` 存在、哨兵在名单内），新增：

| 断言 | 目的 |
| --- | --- |
| 配置含 `$nrconf{override_rc}->{` | 必须是追加语法 |
| 配置**不含** `$nrconf{override_rc} = {` | 禁止整体覆盖 |
| `test_needrestart_config` 保留 `restart = 'a'` 断言 | 防止误改 restart |

关键：断言 `->{` 与 `= {` 的互斥，能精确锁住回归。

### 4.2 新增语义测试（可选加分项）

不引入 Perl 依赖的前提下，用字符串断言覆盖不了「发行版 50 条是否保留」。
该行为已由真机 Perl 实验验证，属**部署验收**而非单元测试范围，记录在 §6。

## 5. 验收标准

### 单元测试

```bash
cd rust/aegis && cargo nextest run --cargo-profile fast-test needrestart
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### 真机验收（两台服务器）

部署后执行，预期 `entries=50` 且 `apt-daily*` 均为 skip：

```bash
perl -e '
  our %nrconf; open my $f,"<","/etc/needrestart/needrestart.conf"; local $/;
  eval <$f>;
  print "entries=", scalar keys %{$nrconf{override_rc}}, "\n";
'
```

## 6. 风险与回滚

| 风险 | 缓解 |
| --- | --- |
| Perl 语法错误导致 needrestart 崩溃 | 真机 Perl 实验已验证语法（`entries=50` 成功加载） |
| 恢复默认名单后某些服务不再重启 | 这是意图内行为；发行版默认即此 |
| `dpkg -V` 仍显示 conffile 漂移 | 已知未解决，独立 issue |
| 回滚 | `git revert` 单提交，配置由 `perform_maintenance` 重写即恢复 |

## 7. 已知未解决问题（不在本 spec 范围）

1. **conffile 漂移**：`dpkg -V needrestart` 显示 `??5?????? c`，aegis 直写
   `/etc/needrestart/needrestart.conf` 覆盖了 dpkg 管理的文件，未来升级会冲突。
   正确做法是写 `conf.d/` 片段。
2. **`restart='a'` 与 Debian 默认 `'i'` 的偏离**：`9b58241` 有意选择 `'a'`，
   但其副作用依赖 `override_rc` 名单完整性。本 spec 修复了名单，未改变 `'a'`。
3. **`installArchives() failed` 误报**：needrestart post-invoke hook 退出码污染
   apt 返回码，导致 `unattended-upgrade` 误判失败。本 spec 降低其触发概率
   （`dbus` 恢复 skip），未根治。
