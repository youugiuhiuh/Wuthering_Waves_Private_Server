# SNI Tester — Go 引擎

高性能并发 SNI 测试引擎，为 Wuthering Waves Private Server 设计。

## 运行模式

- **CLI** — 命令行直接运行
- **Web API** — HTTP API 服务 (端口 18080)
- **ADB 部署** — 推送到 Android 设备作为独立服务

## 快速开始

```bash
# CLI 模式
go build -o sni_tester ./cmd/sni_tester/
./sni_tester -f domains.txt

# Web API 模式 (Android 设备)
make phone-deploy
adb forward tcp:18080 tcp:18080
curl http://localhost:18080/api/health
```

## Flutter App

GUI 客户端在独立项目 [`sni_tester_app/`](../sni_tester_app/)，通过 FFI 加载本引擎编译的 `libsni_web.so`。

```bash
cd ../sni_tester_app
make android-deploy
```

## Makefile

| 目标 | 说明 |
|------|------|
| `make build` | 构建 Android arm64 二进制 |
| `make phone-deploy` | 推送 + 在 Android 设备运行 |
| `make phone-pull` | 拉取测试结果 |
| `make clean` | 清理构建产物 |

## API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/health` | 健康检查 |
| GET | `/api/status` | 当前状态与统计 |
| POST | `/api/start` | 开始测试 |
| POST | `/api/stop` | 停止测试 |
| GET | `/api/progress` | SSE 实时进度 |
| GET | `/api/download` | 下载结果 |
| POST | `/api/upload` | 上传域名文件 |
| GET | `/api/files` | 文件列表 |
| DELETE | `/api/files` | 删除文件 |

## CLI 参数

| 参数 | 说明 |
|------|------|
| `-f` | (必填) 域名文件 |
| `-dns` | DNS 服务器 |
| `-w` | 并发 Worker 数 |
| `-debug` | 调试模式 |
| `-p` | GeoIP 下载代理 |
| `-ttl` | 失败记录记忆天数 |
| `-max` | 仅处理前 N 行 |
| `-force` | 强制重测 |
| `-reset` | 清除历史 |
| `-shutdown` | 测试后退出 |

## 项目结构

```
sni_tester/
├── cmd/
│   ├── sni_tester/main.go    # CLI 入口
│   └── sni_web/              # API 服务器 + CGO 导出
│       ├── main.go
│       ├── handlers.go
│       └── export.go         # FFI 导出 (StartServer/StopServer)
├── pkg/
│   ├── config.go / types.go / dns.go / tls.go
│   ├── geo.go / storage.go / protobuf.go / engine.go
├── go.mod / go.sum
├── Makefile
└── README.md
```

## DNS 引擎

内置智能 DNS 优先级：**DoH → DoT → UDP**，支持 ~35 个国内外 DNS 服务器，动态健康权重 + 智能限流。

## 验证标准

- TLS 1.3 必需
- X25519 系列密钥交换必需
- H2 或 H3 至少支持一种

## 输出格式

`sni/CC.pb` (Protobuf)，CC 为国家代码 (US, JP 等)。
