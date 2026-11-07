# Todo List: XDRIVE 国际版 AWS S3 预设与纯国际服务商体系

- [x] Task 1: AWS S3 数据模型与预设构建器 (XdriveConfig AWS S3 Integration)
  - [x] 编写失败测试用例 (TDD RED)
  - [x] 实现 `new_aws_s3` 与智能解析提取 (TDD GREEN)
  - [x] 确认测试通过并提交代码
- [ ] Task 2: 国际服务商菜单与文案完善 (i18n & Menu Restructuring)
  - [ ] 更新 `zh.yml`, `en.yml`, `ja.yml` 多语言文案
  - [ ] 在 `xray.rs` 中加入 `u_xdrive_p:aws` 按钮并路由
  - [ ] 确认测试通过并提交代码
- [ ] Task 3: 消息监听器集成 AWS S3 问答向导 (Message Handler Integration)
  - [ ] 在 `message.rs` 中接通 AWS 分步问答与全文提取
  - [ ] 编写测试用例验证完整流转
  - [ ] 确认测试通过并提交代码
- [ ] Task 4: 全流程门禁与回归验证 (Quality Gates)
  - [ ] `cargo fmt --check`
  - [ ] `cargo clippy --all-targets --all-features -- -D warnings`
  - [ ] `cargo nextest run --cargo-profile fast-test`
  - [ ] `cargo test --doc`
