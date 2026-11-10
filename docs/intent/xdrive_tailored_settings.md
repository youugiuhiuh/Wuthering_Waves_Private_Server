# 意向书：XDRIVE 各存储服务专属流控参数科学定制

- **目标成果 (Outcome):** 针对 Rust 服务端（`rust/aegis`）内置的 XDRIVE 云存储预设构建器进行流控参数（`xdriveSettings`）调优，按 Cloudflare R2、Backblaze B2、AWS S3、Oracle S3、标准 S3 等各存储的配额计费模式与延迟特性注入定制化默认流控参数。
- **受益人群 (User):** XDRIVE 节点部署者与终端用户（避免长期空跑刷爆免费 API 配额，杜绝意外扣费，同时保障首包低延迟与高吞吐）。
- **执行背景 (Why now):** 当前部分存储（如 R2 的 `maxPoll: 400ms`、S3 缺失 `flush/eager`）参数过于激进或缺乏退避机制，空转时高频发送 LIST 请求，极易在几天内耗尽 R2 的 100 万次月度免费额度或 B2 的每日 2,500 次配额。
- **成功标准 (Success):**
  1. **Cloudflare R2** 预设：`segment: 512KB`, `poll: 50ms`, `maxPoll: 3000ms`, `flush: 30ms`, `eager: 3000ms`, `concurrency: 16`；
  2. **Backblaze B2** 预设：`segment: 512KB`, `poll: 200ms`, `maxPoll: 5000ms`, `flush: 100ms`, `eager: 3000ms`, `concurrency: 4`；
  3. **AWS S3 / Oracle S3 / 标准 S3** 预设：`maxPoll: 2000ms`, `flush: 50ms`, `eager: 2000ms`；
  4. **OneDrive / Google Drive**：保持各自成熟的防 429 专属流控参数；
  5. 单元测试更新并全部通过，`cargo fmt`、`cargo clippy`、`cargo nextest` 全绿。
- **限制条件 (Constraint):** 不改变参数数据结构与字段类型，严格保持配置生成与向导逻辑向下兼容。
- **排除范围 (Out of scope):** 本轮不修改 C# 客户端代码，不增加用户在交互界面的选择复杂度。
