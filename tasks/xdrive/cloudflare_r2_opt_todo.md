# Todo: XDRIVE Cloudflare R2 极速性能调优与中俄伊全通增强

- [x] Task 1: Core S3 构造器与参数调优 (`rust/aegis/src/core/xray/xdrive.rs`)
  - [x] 1.1: 编写 `test_cloudflare_r2_optimized_parameters` 和 `test_cloudflare_r2_with_custom_endpoint` 测试 (TDD Red)
  - [x] 1.2: 更新 `new_cloudflare_r2` 为 256KB 分片、64 并发、10ms 轮询等激进参数
  - [x] 1.3: 实现 `new_cloudflare_r2_with_endpoint` 并支持自定义域名
  - [x] 1.4: 运行 `cargo test core::xray::xdrive` 验证 (TDD Green)
- [x] Task 2: Telegram 向导与多语言提示更新
  - [x] 2.1: 更新 `zh.yml`, `en.yml`, `ja.yml` 的 Cloudflare 向导文案
  - [x] 2.2: 在 `message.rs` 中支持自定义 Endpoint/Domain 识别
  - [x] 2.3: 编写并运行向导单测
- [ ] Task 3: aegis_v2rayN 预设优化与单测 (`~/Dark/aegis_v2rayN`)
  - [ ] 3.1: 在 `AddServerViewModel.cs` 中为 `Cloudflare R2` 预设注入优化参数
  - [ ] 3.2: 编写 `AddServerViewModelTests.cs` 单测并运行 `dotnet test`
  - [ ] 3.3: 提交并推送至 `youugiuhiuh/aegis_v2rayN:master`
- [x] Task 4: 质量审查与发布
  - [x] 4.1: `cargo fmt --check`
  - [x] 4.2: `cargo clippy --all-targets -- -D warnings`
  - [x] 4.3: `cargo nextest run` 全量回归
  - [ ] 4.4: 提交 PR，合入 main，清理 worktree
