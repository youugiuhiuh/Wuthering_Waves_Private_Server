# sni_probe — 三阶段 SNI 探测引擎（Rust 热路径实现）

`./sni_tester`（Go）中三条热路径的 Rust 重写版。Go 版保留原样做对照，两者输入输出格式完全一致，可以混用。

## 为什么是 Rust

| 关注点 | Go 版 (`utls`) | Rust 版 (`boring`) |
| :--- | :--- | :--- |
| ClientHello 指纹 | 需按 profile 逐个模拟，profile 更新滞后 | 直接用 BoringSSL，与 Chrome 同一 wire format |
| PQ 混合 kex | `utls` 手写扩展 | `boring` `pq-experimental` feature，`X25519MLKEM768` |
| 版权头解析 | 完整 HTTP 客户端（含连接池/Keep-Alive 补丁） | 最小 h2 客户端，只取首跳响应头 |
| 运行时依赖 | 需 Go 工具链 + `go.mod` 供应链 | 单静态二进制，无 CGO（BoringSSL 静态编入） |

### 实测对比（同机、同目标、同参数）

| 指标 | Go | Rust | 差异 |
| :--- | ---: | ---: | ---: |
| 二进制体积 | 18.0 MB | **4.2 MB** | −77% |
| 非测试代码行数 | 5,842 | **4,820** | −18% |
| `resolve-map` 2000 域名 w=200 | 40.8s | **31.5s** | **快 23%** |
| `probe-survival` 80 域名 w=80 | 3.02s | 3.04s | 持平 |
| `build-pools` 500 条 w=100 | 28.6s | **10.5s** | **快 2.7 倍** |
| 内存峰值 | 22–33 MB | 39–45 MB | +60%（换吞吐） |

**为什么 `probe-survival` 持平而 `build-pools` 快 2.7 倍**——单次握手的实际开销：

```text
handshake_only x20: wall 636.8ms/次 | cpu 4.04ms/次 | 网络占比 99.4%
full probe     x20: wall 954.9ms/次 | cpu 4.31ms/次 | 网络占比 99.5%
```

**CPU 只占 4ms，99.4% 的时间在等网络 RTT。** 所以：
- 阶段二是纯握手，没有任何可优化的 CPU 部分 → 语言换不来提升
- 阶段三快 2.7 倍不是 Rust 快，而是**把 HTTP 头校验合进了同一条 TLS 连接**
  （Go 版 `ValidateDomain` 里 `CheckH3Support` + `ValidateHTTP` 是两次额外握手）

**结论：继续在语言层面优化是无效努力，真正的杠杆是减少往返次数与提高并发。**

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

## CLI 参数表

### 全局参数（写在子命令之前）

| 参数 | 说明 | 默认 |
| :--- | :--- | :--- |
| `--debug` | 打印逐域名明细（ASN、证书链、失败原因） | 关 |
| `--shutdown` | 全部阶段完成后关机 | 关 |
| `--history <FILE>` | 历史库路径 | `sni_probe_history.redb` |
| `--ttl <DAYS>` | 失败记录保留天数 | `7` |
| `--force` | 无视历史记录重测 | 关 |
| `--reset` | 清空全部历史后退出 | 关 |
| `--no-guard` | 跳过启动并发自检 | 关 |

> ⚠ **全局参数必须写在子命令前面**：`sni_probe --history h.redb build-pools ...` ✅
> `sni_probe build-pools --history h.redb ...` ❌ 会报 `unexpected argument`

### 各子命令

| 子命令 | 关键参数 | 默认 `-w` |
| :--- | :--- | ---: |
| `resolve-map` | `-f` 输入 / `-o` 输出 / `--max` | 300 |
| `probe-survival` | `-f` / `-o` / `--timeout` / `-p`* / `--max` | 100 |
| `build-pools` | `-f` / `--out-dir` / `--geo` / `--timeout` / `--max` | 100 |
| `run` | `-f` / `--state` / `--out-dir` / `--geo` / `--timeout` / `--max` | 100 |
| `upgrade-dir` | `-d` 源目录 / `--out-dir` / `--legacy` / `--keep-dead` / `--geo` | 30 |
| `upgrade-pb` | `-f` 单文件 / `--out` / `--legacy` / `--keep-dead` / `--geo` | 30 |
| `merge` | `-d` 源目录 / `-o` 输出 / `--legacy` | — |

\* `probe-survival -p` 已解析但**尚未接线**，见文末已知限制。

## 判定标准（REALITY 伪装目标）

* TLS 1.3，密钥交换 `X25519` / `X25519MLKEM768`（PQ 池）
* ALPN 协商为 `h2`，或响应头带 `Alt-Svc: h3`（二者任一即可）
* 证书链深度 ≤ 3，证书总字节 ≤ 5500（Xray 内部 8192 字节缓冲）
* HTTP 响应头不含 `cf-ray` / `cf-cache-status`，`server` 不含 `cloudflare` / `hcdn`
* 国家码不属于 CN / HK / MO / IR / RU / KP
* 所属 ASN 不在黑名单（25 个境内云厂商/运营商种子 + 阈值累加拉黑）

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
| `src/storage.rs` | 历史存储（redb）：失败 TTL、成功记录、ASN 黑名单 |
| `src/upgrade.rs` | 存量 `.pb` 双池升级（目录 / 单文件） |
| `src/logging.rs` | `--debug` 开关 |
| `src/shutdown.rs` | `--shutdown` 关机 |
| `src/merge.rs` | 跨文件 `.pb` 合并去重 |
| `src/resource.rs` | 启动时 ulimit / CPU 自检 |

## 与 Go 版的功能对齐

| 功能 | Go | Rust | 说明 |
| :--- | :--- | :--- | :--- |
| 三阶段探测 | ✅ | ✅ | |
| GeoDB 自动下载 | ✅ | ✅ | 10 镜像 + 官方兜底 |
| **历史存储** | badger | **redb** | 成功/失败 TTL、封禁历史、ASN 黑名单 |
| **`--upgrade-dir`** | ✅ | ✅ | 存量 `.pb` 双池升级，支持 `--legacy` |
| **`--upgrade-pb`** | ✅ | ✅ | 单文件升级 |
| **ASN 黑名单** | ✅ | ✅ | 种子 25 个境内 ASN + 阈值累加拉黑 |
| **`--debug`** | ✅ | ✅ | 逐域名 ASN/证书链/失败原因 |
| **`--shutdown`** | ✅ | ✅ | 完成后关机 |
| `--ttl` / `--force` / `--reset` | ✅ | ✅ | 历史 TTL、强制重测、清空历史 |
| **H3 检测** | ✅ | ✅ | `Alt-Svc: h3` 判定，h2 或 h3 任一即可 |
| **跨文件 `.pb` 合并去重** | ✅ | ✅ | `merge` 子命令 |
| SOCKS5 代理 | ✅ | ❌ | Rust 版 `-p` 尚未接线，需代理请用 Go 版 |
| `--iface` 网卡绑定 | ✅ | ❌ | 未实现 |
| DoH / DoT | ✅ | ❌ | 有意不实现，仅用裸 UDP 53 |
| DNS 速率限制/健康加权 | ✅ | ❌ | 有意不实现 |

### 启动时并发安全自检

每个子命令启动前会读 `ulimit -n` 与 CPU 核心数，`-w` 超限时告警并给出推荐值：

```text
⚠ 并发数 -w 500 超出本机安全范围（判定：偏高）
   实测：ulimit -n = 1024, CPU 核心 = 1
   约束：FD 上限 794，CPU 上限 20 → 受 CPU 限制，推荐 -w 20
   瓶颈是 CPU：单次握手约 4ms CPU，并发过高会互相抢占，
   耗时被拉长、超时增多——这些超时同样会被写入历史库，
   下次运行被当成「近期失败」跳过。
   [build-pools] 建议改用：-w 20
```

两条约束：

| 瓶颈 | 推导 | 超限后果 |
| :--- | :--- | :--- |
| 文件描述符 | `ulimit -n` − 128 − 10% | `Too many open files`，假失败入库 |
| CPU | 核心数 × 20 | 握手互相抢占，超时增多 |

**为什么必须是「核心数 × 20」而不是随便填大**：单次握手约 4ms CPU、
墙钟约 600ms–1s，理论比值约 150，取 20 是保守留裕量。单核机器上
`-w 500` 不是「更快」，而是「把 CPU 挤爆 → 大量超时 → 假 dead 入库」。

自检只告警不阻断；确知自己在做什么可加 `--no-guard` 跳过。

### 历史库与 ASN 黑名单

```bash
# 失败记录保留 7 天，第二次运行自动跳过（实测 6.9s -> 0.03s）
./sni_probe --history hist.redb --ttl 7 build-pools -f survived.csv --geo ipinfo_lite.mmdb

# 无视历史重测 / 清空历史
./sni_probe --history hist.redb --force build-pools ...
./sni_probe --history hist.redb --reset

# 逐域名明细（ASN、证书链尺寸、失败原因）
./sni_probe --debug build-pools -f survived.csv
```

同一 ASN 下累计 3 个域名失败后整家拉黑，后续同 ASN 目标零握手直接跳过。
种子名单含 25 个境内云厂商/运营商（Chinanet、China Telecom、Tencent Cloud、Alibaba、Zenlayer 等）。

### 跨文件合并去重（#13）

磁盘上散落多份不同时期的 `.pb` 时，合并成一个去重总池：

```bash
./sni_probe merge -d rust/aegis/src/resources/sni -o /tmp/MERGED.pb
# [merge] ... | PQ=3 classic=2
```

规则：
- 跨文件去重；同一域名同时出现在两池时**以 PQ 为准**（避免客户端按错池握手）
- 自动跳过 CN / HK / MO / IR / RU / KP 的国家文件
- 旧格式需加 `--legacy`

### HTTP/3 检测（#12）

判定顺序（越靠前越省一次连接）：

1. 已取回的响应头里有 `Alt-Svc: h3`（**首选**，复用同一条 TLS 连接，零额外开销）
2. 否则看 ALPN 是否为 `h2`
3. 两者皆无时才主动调 `check_h3_support` 再探一次

因此「只支持 H3 没有 h2」的目标现在也能通过质检，而 v1.9.9 的严格 h2 判定会把它丢掉。

### 存量 `.pb` 升级

```bash
# 目录级（v1.9.9 单池 -> 双池）
./sni_probe upgrade-dir -d rust/aegis/src/resources/sni --legacy --geo ipinfo_lite.mmdb -w 30

# 单文件，另存而不覆盖
./sni_probe upgrade-pb -f rust/aegis/src/resources/sni/US.pb --out /tmp/US.pb --legacy
```

> **`--legacy` 必须显式声明**：旧版 `domains = 1` 与新版的 `pq_domains = 1`
> **字段号完全相同**，字节层面无法区分。猜错的后果是把整池 classic 域名误标成 PQ，
> 客户端会按 PQ 握手失败。因此这里选择显式参数而非启发式。

## 已知限制（`shortcut:`）

| 限制 | 影响 | 规避 |
| :--- | :--- | :--- |
| `-p` SOCKS5 **参数被解析但未接线** | 传了等于没传，仍走直连 —— **境内跑阶段二会暴露真实 IP** | 用 Go 版，或在本地透明代理层处理 |
| `--iface` 网卡绑定未实现 | 无法把流量锁到 4G 网卡 | 同上 |
| DoH / DoT 未实现 | 仅裸 UDP 53；被劫持时无降级路径 | 有意为之（用户决定不做） |
| DNS 限流/健康加权未实现 | `-w` 极高时公共 DNS 可能限流 | 启动自检会先把 `-w` 压到安全值 |
| `h2.rs` 只解析 HPACK 静态表 | 目标若完全用 Huffman 编码首跳头，`server` 判定退化为「无 CDN 头」 | 这类目标仍可能因 ALPN=h2 通过，只是少了 CDN 特征判定 |
| `--no-guard` 无二次确认 | 误用会静默突破并发上限 | 只在确知机器能力时使用 |

## 环境要求与建议

| 项目 | 建议值 | 原因 |
| :--- | :--- | :--- |
| CPU 核心 | **≥ 4 核** | 单核跑 `build-pools` 时 CPU 是瓶颈，推荐 `-w` 被压到 20 |
| `ulimit -n` | **≥ 65535** | 默认 1024 会把上限压到约 794 |
| 内存 | ≥ 512 MB 可用 | 峰值约 45 MB，留裕量给 MMDB 与历史库 |

**1 核机器不适合跑 `build-pools`** —— 它的定位应是控制面（Aegis + Xray），
深度质检应放到多核探测机。
