# Minisign 签名硬化 — 设计文档

**日期**：2026-09-13
**状态**：已批准
**模式**：strict（安全逻辑、跨 Go/Rust、影响 5+ 文件）

## 1. 威胁模型

| 项 | 内容 |
|----|------|
| **防护目标** | 中间人替换二进制（下载链路被劫持、镜像站被篡改） |
| **不防** | 上游发布账号被盗（攻击者能重签）；开发者本地机器沦陷 |
| **核心前提** | 签名缺失 = 拒绝（硬校验）。否则攻击者删掉 `.minisig` 即绕过，签名形同虚设 |

## 2. 现状（已实测核验）

- **GitHub**：27 个 release，**26 个有签名**，仅 `v1.5.0` 缺
- **Codefloe**：releases 存在（如 v1.6.0），**全部无签名**（dispatch 守卫限 GitHub）
- **三端均为软校验**（签名缺失只打印警告后继续）
- 当前公钥：`RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf`，过期 `2027-07-02`
- `rotate-minisign-key.sh` 会**删除** 90 天内到期的旧公钥
- **trusted comment 实测格式**：`v1.5.3:aegis`（从 v1.5.3 的 aegis.minisig 实际下载核验）

## 3. 已批准的决策

| # | 决策 | 选择 |
|---|------|------|
| 1 | Codefloe 是否签名 | ✅ **签**（修 sudo + 架构 + 改 dispatch 守卫） |
| 2 | 旧公钥处理 | ✅ 从活跃列表**移入「历史密钥」列表**供验证 |
| 3 | v1.5.0 无签名 | ✅ 方案 (a) 接受失败（1 小时窗口过渡版本） |
| 4 | 引导悖论 | ✅ 接受「防护从下一个 release 起生效」 |
| 5 | Codefloe 私钥暴露面 | ✅ 接受（仓库访问可控） |
| 6 | 版本匹配 | ✅ `HasPrefix`/`contains` → **精确相等** `==` |

## 4. 关键发现：版本匹配漏洞

三端版本校验**全都不一致且都宽松**：

| 端 | 代码位置 | 当前语义 | 绕过向量 |
|----|---------|---------|---------|
| Go installer | `go/installer/main.go:884` | `strings.HasPrefix(gotVersion, expectedVersion)` | `v1.5.3-evil` 通过 |
| Rust upgrade | `rust/aegis/src/core/system/upgrade.rs:416` | `got_version.contains(&artifact.tag_name)` | `xv1.5.3`、`v1.5.3-evil` 通过 |
| Rust core_upgrade | `rust/aegis/src/core/system/core_upgrade.rs:375` | `got_version.contains(&release.tag_name)` | 同上 |

`gotVersion` 来自签名内容（攻击者若有私钥可任意构造），`expectedVersion` 来自 release tag。
子串/前缀匹配允许攻击者构造 `xv1.5.3` 或 `v1.5.3-evil` 通过校验。

**修复为精确相等不会破坏兼容性**：实测 trusted comment 为干净的 `v1.5.3`，
且 Go 侧 `ver`（`release.TagName`）与 Rust 侧 `tag_name`（API 字段）均含 `v` 前缀，格式一致。

## 5. 架构设计

### 5.1 密钥列表一分为二

```rust
// rust/aegis/src/core/crypto/minisign.rs
pub struct MinisignKeyEntry {
    pub public_key: &'static str,
    pub expires_at: &'static str,
    pub retired_at: &'static str,   // 新增："" = 仍活跃
}

// 活跃密钥：验证新版本，检查过期
pub const MINISIGN_ACTIVE_KEYS: &[MinisignKeyEntry] = &[...];

// 历史密钥：验证用旧钥签的历史版本，永不因过期被剔除
pub const MINISIGN_HISTORICAL_KEYS: &[MinisignKeyEntry] = &[...];
```

**验证顺序**：先试活跃钥（检查过期）→ 再试历史钥（**不检查过期**，
否则历史版本永远无法验证）。

Go 侧同构改造。

### 5.2 硬校验三处

| 文件 | 当前 | 改为 |
|------|------|------|
| `go/installer/main.go:897` | `if !minisigPassed { printYellow("skipped") }` | 拒绝并 `return ""` |
| `rust/.../upgrade.rs:399` | `download_minisig(...) -> Result<Option<Vec<u8>>>` | 缺签名返回 `Err` |
| `rust/.../core_upgrade.rs:355` | `if let Some(sig_url) = ...` | `let sig_url = ...ok_or_else(...)?` |

附带：`ReleaseArtifact.minisig: Option<Vec<u8>>` 在使用点强制 `ok_or_else`。

### 5.3 rotation 脚本改造

```
生成新密钥对
  ↓
旧「活跃」密钥 → 移入「历史」区块（不删除）
  ↓
新密钥 → 写入「活跃」区块
```

同时**移除** `cat "$TEMP_DIR/minisign.key"`（私钥不进终端/history），
改为仅打印文件路径。

### 5.4 Codefloe 签名

| 改动 | 文件 |
|------|------|
| `sudo install` → `install`（root 容器无需 sudo，且常无 sudo） | `.forgejo/workflows/sign-release.yml` |
| 加 `uname -m` 架构分支（x86_64/aarch64） | 同上 |
| `Dispatch Signing Workflow` 守卫 `github` → 两平台均跑 | `.forgejo/workflows/public-release.yml` |
| 私钥用 `mktemp -d` + `trap` 清理 | `.forgejo/workflows/sign-release.yml` |

**依据**：
- Forgejo 官方文档：「the runner will execute all the steps, **as root**」
- minisign 0.12 linux tarball 实测含 `minisign-linux/{x86_64,aarch64}/minisign`
- Codefloe runner 清单：`ubuntu-latest` 同时注册在 amd64 与 arm64 机器上

## 6. 破坏面分析

| 场景 | 影响 |
|------|------|
| 从 GitHub 升级（≥v1.2.6） | ✅ 26/27 正常 |
| 安装/升级 `v1.5.0` | ❌ 失败（已接受） |
| 从 Codefloe 下载（修复前） | ❌ 全部失败 |
| 从 Codefloe 下载（修复后，新版本） | ✅ 正常 |

## 7. 已知限制（引导悖论）

用户机器上**现有的旧 `aegis`/`installer`** 是软校验代码。
硬校验只对「用新版本安装/升级」生效。**防护从下一个 release 起。**
（已接受：旧二进制无法被追溯修改）

## 8. 验收标准

1. Rust 单测覆盖：活跃钥验证、历史钥验证、过期活跃钥拒绝、历史钥忽略过期、精确版本匹配
2. Go 单测同构覆盖
3. 三端版本匹配均为精确相等
4. 签名缺失时三端均拒绝
5. Codefloe 与 GitHub 均产出 `.minisig`
6. rotation 脚本保留历史公钥且不打印私钥
7. `minisign-key-check.yml` 同步扫描历史区块
