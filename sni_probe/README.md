# sni_probe — 三阶段 SNI 探测引擎（Rust 热路径实现）

`./sni_tester`（Go）中三条热路径的 Rust 重写版。Go 版保留原样做对照，两者输入输出格式完全一致，可以混用。

## 为什么是 Rust

| 关注点 | Go 版 (`utls`) | Rust 版 (`boring`) |
| :--- | :--- | :--- |
| ClientHello 指纹 | 需按 profile 逐个模拟，profile 更新滞后 | 直接用 BoringSSL，与 Chrome 同一 wire format |
| PQ 混合 kex | `utls` 手写扩展 | `boring` `pq-experimental` feature，`X25519MLKEM768` |
| 版权头解析 | 完整 HTTP 客户端（含连接池/Keep-Alive 补丁） | 最小 h2 客户端，只取首跳响应头 |
| 运行时依赖 | 需 Go 工具链 + `go.mod` 供应链 | 单静态二进制，无 CGO（BoringSSL 静态编入） |

## 三阶段

```text
resolve-map    海外 VPS        raw 域名       -> domain,ip 映射表      （纯海外 Anycast DNS）
probe-survival 审查境内/代理    domain,ip      -> 存活名单              （零 DNS、零 HTTP、150ms 防 RST）
build-pools    海外 VPS        domain,ip 存活 -> 双池 .pb              （TLS1.3/PQ/证链/管辖国过滤）
```

## 一键串联：`run` 子命令

三段不再需要手搓 CSV —— `run` 用一个 `state.bin` 串起全部阶段，并在每次网络切换时
用 Google 连通性检测把关、等用户回车确认后才继续：

```bash
./sni_probe run -f tranco_raw.csv --state sni_probe_state.bin \
    --out-dir rust/aegis/src/resources/sni --geo ipinfo_lite.mmdb \
    -w 100 --timeout 5
```

```text
=== sni_probe run：一键三阶段 ===

[阶段一] 海外预解析 —— 需要【海外网络】
  → 可连通（www.google.com → 11.27.97.214，TLS 1.3 握手成功）
  ⠹ resolve-map [========>          ] 1204/3000 (40%) 250/s ETA 7s

[阶段二] 境内存活探测 —— 需要【境内网络】，本阶段不发任何 DNS 查询
  → 不可访问：www.google.com → 142.250.x.x TLS 握手失败（connection reset by peer）
  ⠹ probe-survival [=====>         ] 612/1204 (51%) 180/s ETA 3s

[阶段三] 海外深度质检 —— 需要【海外网络】
  → 可连通（www.google.com → 11.27.97.214，TLS 1.3 握手成功）
  ⠹ build-pools [=============>    ] 480/612 (78%) 60/s ETA 2s

=== 全部完成 ===
```

**网络闸门**：每次阶段启动前都会检测 `www.google.com:443` 的完整 TLS 1.3 握手
（不是单看 TCP 连通 —— 审查系统常表现为 TCP 通、ClientHello 后被 RST），
不满足要求会提示切换网络并等待回车复检。

**断点续跑**：`state.bin` 里记录已完成阶段的位掩码，中断后重跑会自动跳过已完成阶段。

**GeoDB 自动下载**：`--geo` 指向的文件不存在时，会自动从 11 个地址按序尝试
（10 个 GitHub 加速镜像 + 官方地址），下载到临时文件再原子 rename，中断不会留下半个库。
响应小于 1 MB 视为错误页直接丢弃：

```text
[GeoDB] 尝试 1/11: https://gh.h233.eu.org/...
[GeoDB] 失败：HTTP 403 Forbidden
[GeoDB] 尝试 4/11: https://gh-proxy.org/...
[GeoDB] 下载完成：ipinfo_lite.mmdb（24.6 MB）
```

下载失败会**直接中止**而不是静默放行——否则管辖国过滤会因国家码全为 `UNKNOWN` 而失效。

**状态文件**（`state.bin`，proto3，`src/state.rs`）：

```proto
message State {
    repeated Entry phase1 = 1;  // 预解析结果 domain,ip
    repeated Entry phase2 = 2;  // 存活候选 domain,ip
    repeated Entry phase3 = 3;  // 深度质检通过 domain
    uint32 completed = 4;       // 已完成阶段位掩码
}
```

> `shortcut:` 自动化测试可用 `SNI_PROBE_SKIP_GATES=1` 绕过网络闸门（仅供 CI/冒烟测试）。

## 编译

需要 `cmake`（BoringSSL 构建期依赖）：

```bash
cargo build --release
# 产物：target/release/sni_probe（约 4 MB）
```

## 用法

```bash
# 阶段一：海外预解析
./sni_probe resolve-map -f tranco_raw.csv -o resolved_targets.csv -w 300

# 阶段二：境内存活探测（输入必须含 IP，本阶段不发任何 DNS 查询）
./sni_probe probe-survival -f resolved_targets.csv -o survived_candidates.csv -w 100 --timeout 3

# 阶段三：深度质检 + 双池 .pb（与磁盘已有内容增量合并）
./sni_probe build-pools -f survived_candidates.csv \
    --out-dir rust/aegis/src/resources/sni \
    --geo ipinfo_lite.mmdb -w 100 --timeout 5
```

## 判定标准（REALITY 伪装目标）

* TLS 1.3，密钥交换 `X25519` / `X25519MLKEM768`（PQ 池）
* ALPN 协商为 `h2`
* 证书链深度 ≤ 3，证书总字节 ≤ 5500（Xray 内部 8192 字节缓冲）
* HTTP 响应头不含 `cf-ray` / `cf-cache-status`，`server` 不含 `cloudflare` / `hcdn`
* 国家码不属于 CN / HK / MO / IR / RU / KP

## 模块

| 文件 | 职责 |
| :--- | :--- |
| `src/target.rs` | 输入行解析（纯域名 / Tranco / `domain,ip`） |
| `src/dns.rs` | 纯海外 Anycast DNS 解析器 |
| `src/cloudflare.rs` | Cloudflare CIDR / ASN 排除 |
| `src/tls.rs` | BoringSSL TLS 1.3 探测、PQ 判定、证链限制、存活探测 |
| `src/h2.rs` | 最小 HTTP/2 客户端（只取响应头） |
| `src/geo.rs` | IPinfo Lite / MaxMind MMDB 国家与 ASN、管辖国过滤 |
| `src/pb.rs` | `DomainList` protobuf 手写编解码 |
| `src/pipeline.rs` | 三阶段并发执行器与增量 `.pb` 写入 |
| `src/state.rs` | `state.bin` 编解码、原子写入、阶段位掩码 |
| `src/connectivity.rs` | Google 连通性检测（DNS + TLS 1.3 握手） |
| `src/run.rs` | 一键编排：网络闸门 + 阶段串联 |
| `src/progress.rs` | 进度条（无 TTY 自动静默） |
| `src/geodl.rs` | IPinfo Lite MMDB 自动下载（镜像轮询 + 原子替换） |

## 已知限制（`shortcut:`）

* `probe-survival` / `build-pools` 的 `-p` SOCKS5 参数在 Rust 版尚未接线（Go 版已支持）；
  需要走代理时请用 Go 版，或在本地透明代理层面处理。
* `h2.rs` 只解析 HPACK 静态表覆盖的响应头；目标若完全用 Huffman 编码首跳头，`server` 判定会退化为"无 CDN 头"。
