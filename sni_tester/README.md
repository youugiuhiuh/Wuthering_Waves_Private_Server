# SNI 测试工具使用说明

高性能并发 SNI 测试工具，为 Wuthering Waves Private Server 的 SNI 资源管理设计。

## 编译

```bash
cd sni_tester

# CLI 模式
go build -o sni_tester ./cmd/sni_tester/

# Web UI 模式 (手机部署用)
go build -o sni_web ./cmd/sni_web/
```

## 快速开始

### CLI 模式 (在 PC 上运行)

```bash
./sni_tester -f domains.txt
```

### Web UI 模式 (在手机上运行)

```bash
# 1. 交叉编译 Android arm64 二进制
GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o sni_web ./cmd/sni_web/

# 2. 推送到手机
adb push sni_web /data/local/tmp/
adb push GeoLite2-Country.mmdb /data/local/tmp/ 2>/dev/null || true
adb push GeoLite2-ASN.mmdb /data/local/tmp/ 2>/dev/null || true

# 3. 在手机上运行
adb shell "cd /data/local/tmp && SNI_OUTPUT_DIR=/data/local/tmp/sni_output chmod +x sni_web && ./sni_web"

# 4. 端口转发
adb forward tcp:8080 tcp:8080

# 5. 在浏览器中打开 http://localhost:8080
```

### Makefile 一键操作

```bash
# 编译 + 推送到手机 + 启动 Web UI
make phone-deploy

# 从手机拉取测试结果
make phone-pull
```

## 参数说明 (CLI 模式)

| 参数 | 说明 |
|------|------|
| `-f` | (必填) 包含待测试域名的 TXT/CSV 文件路径 |
| `-dns` | 指定 DNS 服务器 (不填则使用内置 DNS 池 DoH→DoT→UDP) |
| `-w` | 固定并发 Worker 数 (默认自动 AIMD 调节) |
| `-debug` | 调试模式，跳过网络隔离检查 |
| `-p` | 下载 GeoIP 数据库时使用的代理 |
| `-ttl` | 失败记录记忆天数 (默认 7) |
| `-max` | 仅处理输入文件前 N 行 |
| `-force` | 强制重新测试之前跳过的域名 |
| `-reset` | 清除所有历史记录 |
| `-shutdown` | 测试完成后自动关机 |

## Web UI 模式

启动后通过浏览器访问 `http://localhost:8080`，功能：

1. **上传域名文件** — 拖拽或点击上传 TXT/CSV
2. **配置参数** — 并发数、DNS、TTL、强制重测等
3. **开始/停止** — 控制测试任务
4. **实时进度** — SSE 推送进度条、统计、结果表
5. **下载结果** — 测试完成后打包下载 .pb 文件

## DNS 解析特性

### DNS 优先级: DoH → DoT → UDP

内置智能 DNS 解析引擎，自动切换协议。

### 内置 DNS 服务器池

- **DoH**: 腾讯 `doh.pub`、阿里 `dns.alidns.com`、360 `dns.360.cn`、Cloudflare、Google、Quad9 等
- **DoT**: 腾讯 `dot.pub:853`、阿里 `dns.alidns.com:853`、360 `dns.360.cn:853` 等
- **UDP**: ~35 个国内外 DNS 服务器

### DNS 健康权重系统

动态权重调整，故障衰减，恢复提升，加权选择。

### 智能限流

| 服务商 | QPS 限制 |
|--------|---------|
| 阿里云 DoH/DoT | 15 QPS |
| 阿里云 UDP | 80 QPS |
| 腾讯 DNSPod | 50 QPS |
| 其他国内 | 50 QPS |
| 国外 DNS | 500 QPS |
| 全局上限 | 300 QPS |

## 验证标准

所有域名均通过统一验证:
- **TLS 1.3** 必需
- **X25519 系列密钥交换** 必需
- **H2 或 H3** 至少支持一种

## 输出格式

- 输出文件: `sni/CC.pb` (Protobuf 格式)
- CC 为国家代码 (US, JP 等)

## 项目结构

```
sni_tester/
├── cmd/
│   ├── sni_tester/main.go    # CLI 入口
│   └── sni_web/              # Web UI 服务器
│       ├── main.go
│       ├── handlers.go
│       └── static/index.html
├── pkg/                      # 核心库
│   ├── types.go              # 共享类型
│   ├── config.go             # 配置与常量
│   ├── dns.go                # DNS 解析
│   ├── tls.go                # TLS 握手
│   ├── geo.go                # GeoIP 查询
│   ├── storage.go            # BadgerDB 持久化
│   ├── protobuf.go           # Protobuf I/O
│   └── engine.go             # 核心引擎
├── proto/sni.proto
├── Makefile
└── go.mod
```
