# SNI 测试工具

高性能并发 SNI 测试工具，为 Wuthering Waves Private Server 设计。

支持三种运行模式：
- **CLI** — PC 命令行直接运行
- **Web API** — 纯 API 服务 (端口 18080)，供 Flutter 客户端或 `curl` 调用
- **Flutter** — 跨平台桌面/移动端 GUI (内嵌 Go 后端)

## 快速开始

### CLI 模式

```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester -f domains.txt
```

### Flutter Linux 桌面

```bash
make linux-run
```

### Flutter Android

```bash
make flutter-deploy     # 构建 + 安装到手机
```

### Web API 模式 (调试用)

```bash
GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o sni_web ./cmd/sni_web/
adb push sni_web /data/local/tmp/
adb shell "cd /data/local/tmp && ./sni_web" &
adb forward tcp:18080 tcp:18080
curl http://localhost:18080/api/health
```

## API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/health` | 健康检查 (status/uptime/version) |
| GET | `/api/status` | 当前状态与统计 |
| POST | `/api/start` | 开始测试 (body: servers_file, domains_file, timeout_sec, max_concurrent) |
| POST | `/api/stop` | 停止测试 |
| GET | `/api/progress` | SSE 实时进度推送 |
| GET | `/api/download` | 下载结果 (.pb 文件) |
| POST | `/api/upload` | 上传域名文件 |

## 构建

```bash
# 原生 Android arm64 二进制
make sni_web

# Flutter Android APK (自动打包 Go 二进制)
make flutter-build

# Flutter Linux 桌面版
make linux-build

# 推送到手机运行 (纯 Web API，无 Flutter)
make phone-deploy
```

## 参数说明 (CLI 模式)

| 参数 | 说明 |
|------|------|
| `-f` | (必填) 待测试域名文件路径 |
| `-dns` | DNS 服务器 (不填则自动 DoH→DoT→UDP) |
| `-w` | 固定并发 Worker 数 |
| `-debug` | 调试模式 |
| `-p` | GeoIP 下载代理 |
| `-ttl` | 失败记录记忆天数 (默认 7) |
| `-max` | 仅处理前 N 行 |
| `-force` | 强制重测跳过记录 |
| `-reset` | 清除历史 |
| `-shutdown` | 测试后自动关机 |

## Flutter 架构

```
flutter_app/
├── lib/
│   ├── data/
│   │   ├── models/         # Stats, ProgressEvent, StatusResponse, StartParams
│   │   └── services/       # ApiClient (HTTP + SSE + 自动解压 Go 二进制)
│   ├── ui/
│   │   ├── core/           # AppTheme (Material 3, 系统主题)
│   │   └── features/home/
│   │       ├── view_models/  # HomeViewModel (ChangeNotifier)
│   │       └── views/        # HomeScreen + 4 个 Widget
│   └── main.dart
├── assets/
│   └── sni_web             # Go 后端二进制 (自动打包)
└── test/
    └── models_test.dart
```

Flutter 启动时自动从 asset 提取 `sni_web` 到应用目录并执行，通过 `http://localhost:18080` 通信。

## 项目结构

```
sni_tester/
├── cmd/
│   ├── sni_tester/main.go    # CLI 入口
│   └── sni_web/              # API 服务器
│       ├── main.go           # CORS, /api/health, 端口 18080
│       └── handlers.go       # SSE, start, stop, status, download, upload
├── pkg/                      # 核心库
│   ├── types.go / config.go / dns.go / tls.go
│   ├── geo.go / storage.go / protobuf.go / engine.go
├── flutter_app/              # Flutter 跨平台 GUI
├── proto/sni.proto
├── Makefile                  # build/deploy/run 一键命令
└── go.mod
```

## DNS 引擎

内置智能 DNS 优先级：**DoH → DoT → UDP**，支持 ~35 个国内外 DNS 服务器，动态健康权重 + 智能限流。

## 验证标准

- TLS 1.3 必需
- X25519 系列密钥交换必需
- H2 或 H3 至少支持一种

## 输出格式

`sni/CC.pb` (Protobuf)，CC 为国家代码 (US, JP 等)。
