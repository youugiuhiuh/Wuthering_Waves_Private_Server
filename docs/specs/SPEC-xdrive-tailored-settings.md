# Spec: XDRIVE 各存储服务专属流控参数科学定制

## 1. 目标与背景 (Objective & Background)

### 1.1 背景
XDRIVE 是基于云对象存储或网盘作为信箱中转的传输协议。由于服务端并不监听常规入站端口，必须通过周期性 `ListObjects` 轮询云存储目录来感知客户端上行数据。
目前 `rust/aegis` 中内置的各存储构造器存在以下流控参数痛点：
- **Cloudflare R2**：原 `poll: 30ms / maxPoll: 400ms` 过度激进，空转时每秒高达 2.5 次请求，挂机 5~6 天即可将整月 100 万次免费 Class A 配额耗尽；
- **Backblaze B2**：虽然已做过退避调整，但仍需在配额保护（每天仅 2,500 次）与 TCP/TLS 首包超时风险之间取得更精准的平衡；
- **AWS S3 / Oracle S3 / 标准 S3**：缺失 `flush_interval_ms` 与 `eager_window_ms` 参数，`maxPoll: 800ms` 退避上限偏低，容易造成无序突发或高频空转。

### 1.2 目标
为每一种支持的存储服务注入量身定制的专属 `xdriveSettings` 预设，兼顾“防刷爆免费额度”与“交互低延迟”：

| 存储提供商 | 分片大小 (`segmentBytes`) | 轮询基础 (`pollIntervalMs`) | 最大退避 (`maxPollIntervalMs`) | 刷出缓冲 (`flushIntervalMs`) | 积极窗口 (`eagerWindowMs`) | 并发槽数 (`concurrency`) | 设计意图 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Cloudflare R2** | 512KB | 50ms | 3000ms (3s) | 30ms | 3000ms (3s) | 16 | 3秒深度退避，全月稳用 <80% 免费额度，唤醒迅速 |
| **Backblaze B2** | 512KB | 200ms | 5000ms (5s) | 100ms | 3000ms (3s) | 4 | 5秒退避控制空转调用，同时避免浏览器 5秒握手超时 |
| **AWS S3** | 1024KB | 50ms | 2000ms (2s) | 50ms | 2000ms (2s) | 16 | 企业级高吞吐，补齐积极窗口与刷出缓冲 |
| **Oracle S3** | 1024KB | 50ms | 2000ms (2s) | 50ms | 2000ms (2s) | 8 | 兼顾 20GB 免费空间与甲骨文出网特性 |
| **标准/自定义 S3** | 1024KB | 50ms | 2000ms (2s) | 50ms | 2000ms (2s) | 8 | 通用稳健基线，避免空刷 |
| **Microsoft OneDrive** | 64KB | 100ms | 1000ms (1s) | 30ms | 2000ms (2s) | 4 | *(保持现役最佳实践)* 微软 Graph API 防 429 平衡线 |
| **Google Drive** | 512KB | 50ms | 200ms (0.2s) | 10ms | 10000ms (10s) | 64 | *(保持现役最佳实践)* 官方调优低延迟直连 |

---

## 2. 技术栈与命令 (Tech Stack & Commands)

- **语言**: Rust (2021 edition)
- **格式检查**: `cargo fmt --check`
- **代码分析**: `cargo clippy --workspace --all-targets -- -D warnings`
- **测试**: `cargo nextest run --workspace`

---

## 3. 涉及文件与模块结构 (Project Structure & Modified Files)

```text
rust/aegis/
├── src/
│   └── core/
│       └── xray/
│           ├── xdrive.rs       # 构造器参数升级及相关预设测试用例更新
│           └── config.rs       # 测试断言同步更新
```

---

## 4. 测试策略 (Testing Strategy)

1. **单元测试 (`xdrive.rs` & `config.rs`)**：
   - 验证 `new_cloudflare_r2`、`new_cloudflare_r2_with_endpoint` 生成的参数符合规范；
   - 验证 `new_backblaze_b2` 生成的参数符合规范；
   - 验证 `new_aws_s3`、`new_oracle_cloud_s3`、`new_s3` 生成的参数完整包含 `flush_interval_ms` 和 `eager_window_ms`；
   - 验证向导文本解析输出符合新参数规范。
2. **全量门禁保障**：
   - `cargo fmt --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo nextest run` 100% 绿灯。

---

## 5. 边界规则 (Boundaries)

- **Always do (必须做)**:
  - 严格保持 `XdriveConfig` 字段类型（`Option<u32>` / `Option<u64>`）不变；
  - 同步更新测试套件中断言旧参数数值的用例，杜绝测试断言过期；
  - 保证构建出的 Xray inbound JSON 中各字段名完全与 Xray 内核对齐。
- **Never do (严禁做)**:
  - 破坏现有构造器签名或公共 API；
  - 将 `maxPollIntervalMs` 设为过长数值（如 >10s）导致系统 TCP 握手超时。

---

## 6. 验收标准 (Success Criteria)

- [ ] `new_cloudflare_r2` 与 `new_cloudflare_r2_with_endpoint` 使用 `poll: 50`, `maxPoll: 3000`, `flush: 30`, `eager: 3000`, `concurrency: 16`；
- [ ] `new_backblaze_b2` 使用 `poll: 200`, `maxPoll: 5000`, `flush: 100`, `eager: 3000`, `concurrency: 4`；
- [ ] `new_aws_s3`、`new_oracle_cloud_s3`、`new_s3` 补齐 `flush: 50`, `eager: 2000`, `maxPoll: 2000`；
- [ ] 所有相关单元测试断言同步更新并全部通过；
- [ ] 门禁检查（`fmt`, `clippy`, `nextest`）全绿。

---

## 7. 待确认问题 (Open Questions)

- 无。
