# Implementation Plan: REALITY fallback 流量兜底（生成期 + 启动自愈）

Source spec: `docs/specs/SPEC-reality-fallback-limits.md`（V1，已按用户决议定稿：不分档 / afterBytes=1 MiB / 自愈在 `ensure_base_config` 内 / `bytesPerSec:0` 视为已配置 / `warn` 一次）

## Overview

给 aegis 生成的每一个 REALITY 入站补上 `limitFallbackUpload` / `limitFallbackDownload`，并在启动时幂等自愈存量 `*_inbounds.json`，使未认证 fallback 中继从“无限速”变为“前 1 MiB 全速、之后下载 ≤512 KiB/s”。已认证 VLESS 用户路径不经过该连接器，行为不变。

## Architecture Decisions

1. **双通道**：新节点由 `build_reality_vless_inbound`（`config.rs:455`）注入；存量由 `ensure_base_config()`（`config.rs:1119`，被 `main.rs:168` / `installer.rs:584` / `maintenance.rs:127` 调用）自愈。只改生成器会漏掉所有已部署节点。
2. **纯函数 + 薄 IO**：`patch_reality_fallback_limits(&mut Value) -> bool` 不碰 IO，全部语义用单测锁定；IO 循环单独一层，接收 `conf_dir: &Path`，便于用临时目录测试而不新增 `tempfile` 依赖。
3. **对象级存在检查**：只要 `limitFallbackUpload` / `limitFallbackDownload` 键存在就整块跳过，不做 key 级补全。避免与运维手工调参打架，也让 `{"bytesPerSec": 0}`（显式关闭）语义稳定。
4. **单次 reload**：所有文件改写完成后调用一次 `MaintenanceManager::reload_core()`；启动早期核心未起时 reload 失败只记 warn。
5. **不新增依赖、不改 schema、不动 sni_probe**（spec §9）。
6. **单文件改动 → 不可并行**：T1–T4 全部落在 `rust/aegis/src/core/xray/config.rs`，按 AGENTS.md 的并发铁律必须单代理串行，不派发并行子代理。

## Task List

### Phase 1：Foundation

- [x] **Task 1 — 常量 + 纯函数补丁（TDD 红→绿）**
  - 先写失败用例：生成器字段存在且值正确（2 个）、补丁幂等（1 个）、尊重手工值含 `bytesPerSec:0`（1 个）、非 REALITY 入站不动（1 个）、畸形输入不 panic（1 个）。
  - 新增 `FALLBACK_UPLOAD_LIMIT` / `FALLBACK_DOWNLOAD_LIMIT` 常量与 `patch_reality_fallback_limits`。
  - Acceptance：6 个新用例全绿；`cargo nextest run -E 'test(reality_fallback)'` 通过。
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：无 ｜ Scope：S

- [x] **Task 2 — 生成器注入**
  - 在 `build_reality_vless_inbound` 的 `realitySettings` 中写入两个对象；确认 XHTTP 路径（`xhttp.rs:55` 复用同函数）同样命中。
  - Acceptance：Vision 与 XHTTP 生成的 JSON 均含两字段且值等于常量；既有生成器测试不回归。
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：T1 ｜ Scope：S

### Checkpoint 1
- [x] `cargo nextest run -E 'test(reality_fallback)'` 全绿
- [x] `git diff` 确认仅新增字段，未改其它 realitySettings 键

### Phase 2：Integration

- [x] **Task 3 — 自愈接入**
  - 抽出 `list_inbound_files_in(dir)`；实现 `ensure_reality_fallback_limits()`：逐个 `*_inbounds.json` 读 → 解析 → 补丁 → 原子写（临时文件 + rename）；有改动才累计，最后 reload 一次，并打一条 `warn` 汇总（文件数 / 入站数）。
  - 容错：目录不存在、文件不可读、JSON 畸形、只读文件 → 各自 `warn` 并继续，绝不阻断启动。
  - 原子写前按原文件 mode 设置临时文件权限（配置含 `privateKey`，不得放宽）。
  - 在 `ensure_base_config()` 中调用（放在 geodata 之后、`00_base.json` 分支之前），**不 reload**：`reload_core()` 会回调本函数，会递归。
  - Acceptance：二次调用返回 false 且文件逐字节不变；畸形/缺失场景不 panic。
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：T1 ｜ Scope：S

- [x] **Task 4 — 自愈层测试（临时目录）**
  - 用 `std::env::temp_dir()` + 进程内唯一子目录，覆盖：无字段文件被补齐、已有字段文件不变、畸形 JSON 跳过、纯 `00_`/非 `*_inbounds.json` 不处理。
  - Acceptance：新增 4 个用例全绿，且测试结束后自行清理临时目录。
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：T3 ｜ Scope：S

### Checkpoint 2
- [x] `cargo nextest run` 全量绿
- [ ] 真实节点启动路径跑一遍（并入 T6）

### Phase 3：Verification

- [x] **Task 5 — 质量门禁**
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo +nightly nextest run --cargo-profile fast-test`
  - `CARGO_PROFILE_RELEASE_LTO=false cargo +nightly build --release`
  - Acceptance：四项全通过，clippy 零告警。
  - Scope：XS

- [ ] **Task 6 — 手动验收（需真实节点，不进 CI）**
  - `xray -test -config <file>_inbounds.json` 通过；
  - 未认证 fallback 连接：前 1 MiB 全速，之后 ≤512 KiB/s；
  - 已认证 VLESS 用户同文件下载 A/B 差异 ≤10%；
  - 记录实测数值进 spec §11。
  - Acceptance：四项均有记录。

### Checkpoint: Complete
- [ ] spec §验收清单全部勾选
- [ ] Diff ≤2 文件 / ≤200 行（不含 spec/plan）
- [ ] `code-review-and-quality` + `doubt-driven-development` 覆核通过

## Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| 旧版 Xray 不认识字段 | 低（`json.Unmarshal` 忽略未知字段，不阻断启动） | 下发无条件；文档注明“仅新核心生效” |
| 自愈写坏配置 | 高 | 解析成功才写；临时文件 + `rename` 原子替换；写失败只 warn |
| 覆盖运维手工值 | 中 | 对象级存在检查 + 专门用例（含 `bytesPerSec:0`） |
| 启动早期 reload 失败 | 低 | 捕获并 warn，不向上传播 |
| 扫描目录耗时 | 低 | 只匹配 `*_inbounds.json` 且排除 `00_`，文件数量级为个位数 |

## Open Questions

无（spec §10 已全部决议）。
