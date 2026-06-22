# SNI Tester 手机端执行方案

**日期:** 2026-06-22
**模式:** strict
**状态:** approved

## 目标

将 `sni_tester` 部署到 Android 手机运行，通过 Web UI 控制，测试完成后通过 adb/USB 取回 `.pb` 结果文件到 PC 开发机。

功能与 PC 端 CLI 完全一致（GeoIP/ASN/DNS池/BadgerDB/多国.pb输出），只增加 Web 控制层和简化部署流程。

## 架构

```
sni_tester/
├── cmd/
│   ├── sni_tester/main.go    # CLI 入口（重构为调用 pkg/，行为不变）
│   └── sni_web/main.go       # Web 服务入口（新增）
├── pkg/
│   ├── config.go             # Config 结构体、常量、DNS池、uTLS指纹池
│   ├── dns.go                # DNS 解析、故障转移、限流、健康追踪、缓存
│   ├── tls.go                # TLS 握手、uTLS 指纹、ALPN、H2/H3 验证
│   ├── geo.go                # GeoIP 国家/ASN 查询、缓存、黑名单
│   ├── storage.go            # BadgerDB 操作（成功/失败/阻止历史、ASN黑名单）
│   ├── protobuf.go           # .pb 文件读写（DomainList marshal/unmarshal）
│   ├── engine.go             # 核心编排：Worker池、AIMD并发控制、结果收集、进度回调
│   └── types.go              # 共享类型定义
├── proto/
│   └── sni.proto             # 不变
├── Makefile                  # 新增 phone-deploy / phone-pull target
└── go.mod / go.sum           # 依赖不变
```

### CLI 入口 (`cmd/sni_tester/main.go`)

- 解析命令行参数 → 填充 `pkg.Config`
- 注册终端进度回调（progressbar）
- 调用 `pkg.Engine.Run(ctx, config, callback)`
- 输出到 `rust/aegis/src/resources/sni/`

### Web 入口 (`cmd/sni_web/main.go`)

- 嵌入式 HTML 单页（`embed.FS`）
- API 路由（全部标准库 `net/http`）：

| 路由 | 方法 | 说明 |
|------|------|------|
| `/` | GET | Web UI 页面（单文件 HTML，内嵌 CSS/JS） |
| `/api/upload` | POST | 上传输入文件（TXT/CSV），存入临时目录 |
| `/api/start` | POST | 启动测试，body: `{workers,ttl,force,reset,debug,dns,max}` |
| `/api/progress` | GET | SSE 流，推送实时进度和结果 |
| `/api/stop` | POST | 停止测试 |
| `/api/download` | GET | 下载全部 .pb 文件（zip） |
| `/api/status` | GET | 当前状态 `{state: "idle|running", progress, stats}` |

### 核心引擎 (`pkg/engine.go`)

```go
type ProgressCallback func(event ProgressEvent)

type ProgressEvent struct {
    Type      string   // "dns_resolved", "tls_handshake", "validated", "skipped", "failed", "done"
    Domain    string
    Success   bool
    Country   string
    IP        string
    Info      string
    Progress  float64  // 0.0-1.0
    Stats     Stats    // 累计统计
}

type Stats struct {
    Total, Success, Failed, Skipped int
    RatePerSec                     float64
}

type Engine struct{ cfg Config }

func (e *Engine) Run(ctx context.Context, input io.Reader, cb ProgressCallback) (*Result, error)
```

### Config 结构体（统一 CLI 和 Web 入口）

```go
type Config struct {
    DNSAddr      string
    FixedWorkers int
    Debug        bool
    ForceRetry   bool
    ResetAll     bool
    TTLDays      int
    MaxLines     int
    AutoShutdown bool
    GeoDBFile    string
    GeoASNFile   string
    BadgerDBDir  string
    OutputDir    string
    UseBuiltinDNS bool
}
```

## 部署流程

### phone-deploy（一键部署）

```makefile
phone-deploy:
    # 1. 下载 GeoIP 数据库（如本地缺失）
    # 2. GOOS=android GOARCH=arm64 go build -o sni_web ./cmd/sni_web
    # 3. adb push sni_web /data/local/tmp/sni_web
    # 4. adb push GeoLite2-Country.mmdb /data/local/tmp/
    # 5. adb push GeoLite2-ASN.mmdb /data/local/tmp/
    # 6. adb shell "cd /data/local/tmp && chmod +x sni_web && ./sni_web"
    # 7. adb forward tcp:8080 tcp:8080
    # 8. echo "打开 http://localhost:8080"
```

### phone-pull（一键取回）

```makefile
phone-pull:
    # adb pull /data/local/tmp/sni_output/ rust/aegis/src/resources/sni/
```

### 使用流程

```
1. PC: make phone-deploy
2. 浏览器: 打开 localhost:8080（或手机浏览器 localhost:8080）
3. 浏览器: 上传域名文件 → 设置参数 → 点开始
4. 浏览器: 观察实时进度，等待完成
5. PC: make phone-pull
6. PC: cargo build（.pb 已就位）
```

## 数据流

```
手机:
  域名输入 (TXT/CSV) → Engine.Run()
    ├─ DNS 解析 (多池 + 缓存 + 限流)
    ├─ GeoIP 国家/ASN 查询
    ├─ TLS 握手 (uTLS 随机指纹)
    ├─ 验证 (TLS 1.3 + X25519 + H2/H3)
    ├─ BadgerDB 历史记录
    └─ 输出 .pb 文件到 sni_output/
           ↓ SSE → Web UI (实时进度)
           ↓ adb pull
PC:
  rust/aegis/src/resources/sni/*.pb
    → cargo build → SniAssets (rust_embed)
```

## 错误处理

- **手机掉电/中断**：BadgerDB 记录已测结果，重启后跳过已成功域名
- **DNS 全部失败**：报告网络错误，不 crash
- **GeoIP DB 缺失**：首次启动自动下载；下载失败时国家记为 UNKNOWN，继续测试
- **磁盘满**：BadgerDB 写入失败时告警但不中断，结果仍输出 .pb
- **ADB 断开**：Web UI 独立运行不受影响，取回时重新连接即可

## 测试策略

1. **pkg/ 单元测试**：每个包独立可测
   - `dns_test.go`：DNS 解析、故障转移、缓存命中
   - `tls_test.go`：TLS 握手结果验证、H3 检测
   - `geo_test.go`：GeoIP 缓存、ASN 黑名单
   - `storage_test.go`：BadgerDB CRUD、TTL 过期
   - `engine_test.go`：Worker池行为、进度回调、ctx 取消
2. **集成测试**：CLI 入口对照重构前后输出一致性
3. **端到端**：phone-deploy → Web UI 操作 → phone-pull → 验证 .pb 文件

## 不变项

- `sni_tester/main.go` CLI 行为完全不变
- `proto/sni.proto` 不变
- Rust 端 `SniAssets` / `SNISelector` 零改动
- 现有 `go.mod` 依赖不增不减（Web 用标准库）
