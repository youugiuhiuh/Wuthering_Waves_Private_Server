# SNI Tester — Go 引擎

高性能并发 SNI 测试与后量子（Post-Quantum）伪装域名管理引擎，为 Wuthering Waves Private Server 设计。

## 核心特性

- **后量子（PQ）双卡牌池**：精准识别 TLS 1.3 后量子混合密钥算法（RFC 10024 `X25519MLKEM768`、`X25519Kyber768Draft00`、`SecP256r1MLKEM768`），按国家生成主卡牌池（PQ）与手牌备用池（Classic）。
- **极速存量就地升级模式**：直接读取现有的 172 个国家 `.pb` 文件，跳过耗时的 GeoIP/ASN 重复检验，极速并发复测密钥算法并清洗失效死链。
- **多网卡流量隔离绑定（`-iface`）**：支持将所有测试流量（TLS + DNS）通过 `SO_BINDTODEVICE` 与源 IP 强行绑定到指定无线网卡（如 `wlp6s0`），彻底隔离主宽带连接，日常上网与 SSH 零卡顿。
- **极速并发 DNS 解析**：内置高速无状态 UDP 53 竞速解析器与内存 LRU 缓存，单次解析仅需 10~30ms，解除高并发限流瓶颈。

---

## 运行模式

- **CLI** — 命令行直接运行测试或批量升级
- **Web API** — HTTP API 服务 (端口 18080)
- **ADB 部署** — 推送到 Android 设备作为独立服务

---

## 快速开始

### 1. 存量资源快速升级（推荐）
直接对已有 172 个国家的 `.pb` 进行后量子复测打标，并绑定无线网卡：

```bash
# 批量升级所有国别 .pb 资源文件（开 100 并发，2s 快速熔断）
./sni_tester -upgrade-dir rust/aegis/src/resources/sni -iface wlp6s0 -w 100 -timeout 2

# 仅测试升级单个国家的 .pb 文件
./sni_tester -upgrade-pb rust/aegis/src/resources/sni/US.pb -iface wlp6s0 -w 30
```

### 2. 原始域名列表全量探测
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester -f domains.txt -iface wlp6s0 -w 50
```

### 3. Web API 模式 (Android 设备)
```bash
make phone-deploy
adb forward tcp:18080 tcp:18080
curl http://localhost:18080/api/health
```

---

## Flutter App

GUI 客户端在独立项目 [`sni_tester_app/`](../sni_tester_app/)，通过 FFI 加载本引擎编译的 `libsni_web.so`。

```bash
cd ../sni_tester_app
make android-deploy
```

---

## CLI 参数速查

| 参数 | 说明 | 示例 |
|------|------|------|
| `-upgrade-dir` | 直接批量升级目录下的所有 `.pb` 文件至后量子双池格式 | `-upgrade-dir rust/aegis/src/resources/sni` |
| `-upgrade-pb` | 直接复测并升级单个 `.pb` 文件 | `-upgrade-pb rust/aegis/src/resources/sni/US.pb` |
| `-iface` | 强行将出站测试流量绑定至指定网卡或 IP（隔离宽带） | `-iface wlp6s0` 或 `-iface 192.168.2.143` |
| `-timeout` | TLS 握手与 DNS 查询超时秒数（默认 2s，快速淘汰死链） | `-timeout 2` |
| `-keep-dead` | 升级时是否保留握手失败的旧域名（默认 false，自动清洗死链） | `-keep-dead` |
| `-out-dir` | 升级产物输出目录（默认原地覆盖更新） | `-out-dir /tmp/upgraded_pb` |
| `-w` | 并发 Worker 线程数（升级模式默认 30，支持 100+） | `-w 100` |
| `-f` | (常规探测模式) 输入的 TXT/CSV 原始域名文件 | `-f domains.txt` |
| `-dns` | 自定义 DNS 服务器地址 | `-dns 1.1.1.1` |
| `-p` | GeoIP 数据库下载代理 | `-p http://127.0.0.1:7890` |
| `-ttl` | 失败记录记忆天数（默认 7 天） | `-ttl 7` |
| `-force` | 强制重测跳过的域名 | `-force` |
| `-reset` | 清除历史记录并全量重测 | `-reset` |
| `-debug` | 开启调试日志 | `-debug` |
| `-shutdown` | 测试完成后自动关机 | `-shutdown` |

---

## 验证与后量子判定标准

1. **协议要求**：
   - 必须支持 TLS 1.3
   - 必须支持 HTTP/2 或 HTTP/3 至少一种应用层协议
2. **密钥交换与后量子（PQ）分流**：
   - **PQ 主卡牌池 (`pq_domains`)**：服务端在 `ServerHello` 中选定了后量子混合密钥组（RFC 10024 `X25519MLKEM768` [0x11EC]、`X25519Kyber768Draft00` [0x6399]、`SecP256r1MLKEM768` [0x11ED]）；
   - **经典手牌备用池 (`classic_domains`)**：服务端降级协商至经典 `X25519` (0x001D)；
   - **全集兼容池 (`domains`)**：包含去重后的所有可用域名，保证旧版 Rust `build.rs` 与客户端完全向后兼容。

---

## 输出 Protobuf 结构

生成 `sni/CC.pb`（CC 为国家代码，如 US, JP, SG 等），对应 Protobuf 定义：

```protobuf
message DomainList {
    repeated string domains = 1;          // 全集域名（向后兼容）
    repeated string pq_domains = 2;       // 后量子优先主卡池
    repeated string classic_domains = 3;  // 传统 X25519 备用手牌池
}
```

---

## 性能优化与网络分流建议

1. **防止宽带卡死**：
   并发扫描会产生大量短连接和 NAT 状态表开销。推荐使用无线网卡并传入 `-iface <无线网卡名>`（如 `wlp6s0`），使流量完全从 Wi-Fi 发出，有线宽带日常使用 0 影响。
2. **高速探测建议**：
   使用 `-w 100 -timeout 2`。极速并发 UDP 解析器单次响应仅 10~30ms，2 秒超时可快速淘汰无效死链，大幅缩短测试周期。
