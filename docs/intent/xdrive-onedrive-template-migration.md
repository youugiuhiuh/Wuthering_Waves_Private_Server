# Statement of Intent: XDRIVE OneDrive Template 原生集成与旧版 Azure App 废除

## 1. 意图概述 (Outcome)
在 `rust/aegis` 服务端管理体系中彻底废除/移除原有的 Azure App 手动配置逻辑，重构 `XdriveService::OneDrive`，底层全面对接经过实测验证的 Xray-core `service: "template"` 引擎。
用户无需申请企业级 Azure App 或手动配置 Client ID / Client Secret，只需授权常规个人版微软账号获取 `refresh_token`（例如通过 `rclone authorize "onedrive"` 输出的 JSON），即可自动完成配置渲染与无缝代理互通。

## 2. 目标用户 (User)
- Aegis 服务端管理运维者（通过 CLI 或交互式机器人配置节点）；
- 终端代理客户端用户（导入 VLESS XDRIVE 链接或由客户端协同驱动）。

## 3. 背景与动因 (Why Now)
- Xray-core 原生并未实现官方内置的 `service: "onedrive"`，原有引导用户前往 Azure Entra ID 注册 App 的逻辑不仅门槛极高，且在 Xray-core 内运行时因缺少对应驱动而直接失败；
- 我们已通过沙盒与真实公网流量双向验证：基于 Xray-core 原生 `template` 引擎、注入公共 Client 凭证、开启 `"flatten": true` 并配置均衡流控防 429 参数，可在个人免费版 OneDrive 上稳定实现全双工隧道代理。

## 4. 成功指标 (Success Criteria)
1. 移除 `rust/aegis` 中所有引导用户手动配置 Azure App / Client Secret 的废弃逻辑与文案；
2. 向导智能解析：输入包含 `refresh_token` 的 JSON 或纯 Token 时，自动注入内置公共 Client ID 与 Secret；
3. 配置生成输出：生成的 Xray 服务端与客户端 JSON 中，自动渲染为结构合法的 `service: "template"` 对象，包含完整的 auth/put/get/delete/list 及平衡流控参数；
4. 质量与回归门禁：现有测试与新增的 OneDrive Template 测试用例 100% 通过（`cargo test` 绿灯），无 `cargo clippy` 告警。

## 5. 约束限制 (Constraints)
- 纯 Rust 原生实现，不引入额外重量级外部依赖；
- 对底层 Xray-core 协议与结构保持 100% 兼容；
- 遵循项目 `strict` 模式工程铁律。

## 6. 范围外事项 (Out of Scope)
- 本期不在 Aegis 内置启动本地 OAuth2 HTTP 重定向监听服务。
