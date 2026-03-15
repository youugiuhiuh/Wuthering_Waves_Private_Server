# SNI 测试工具使用说明

这是一个高性能的并发 SNI 测试工具，使用 Go 编写，专门为 Wuthering Waves Private Server Emulator 的 SNI 资源管理设计。

## 📍 位置

项目目录下的 `sni_tester/`

## 🚀 编译

在当前目录下运行：

```bash
go build -o sni_tester main.go
```

## 📝 使用方法

```bash
./sni_tester -f <输入文件路径> [-dns <DNS服务器>] [-doh <DoH地址>] [-w <固定并发数>] [-shutdown] [其他参数]
```

### 参数说明

- **`-f` (必填)**: 包含待测试域名的 TXT/CSV 文件路径 (支持 `#` 或 `//` 注释)。
- **`-dns` (可选)**: 指定用于解析的 DNS 服务器。**若不填，工具将自动启用内置的“国际顶级超级并发 DNS 池”（包含 Cloudflare, Google, OpenDNS 等 47 个顶级节点并发轮询）及内存 DNS 缓存引擎，极大提升解析速度并绕过国内 DNS 丢包限流。**
- **`-doh` (可选)**: 启用 DNS-over-HTTPS 解析，优先建议使用 Cloudflare DoH，例如 `-doh https://cloudflare-dns.com/dns-query`。启用后，域名解析将通过 DoH 完成，而不是 UDP DNS。
- **`-w` (可选)**: 指定固定的并发 Worker 数量（例如 `-w 2000`）。如果提供此参数，将**禁用 AIMD 自动并发调节功能**，并强制以固定并发执行测试。
- `-debug`: 开启调试模式，显示详细握手日志并允许在网络通畅时运行。
- `-p`: 下载 GeoIP 数据库时使用的代理 (支持 http/socks5)。
- `-ttl`: 失败记录的记忆天数 (默认 7 天)，在此期间内的重复失败域名将被跳过。
- `-max`: 仅处理输入文件的前 N 行。
- `-xhttp`: 开启 XHTTP 专项校验模式 (要求 TLS 1.3 + H2/H3)。
- `-reality`: 开启 Reality 专项校验模式 (要求 TLS 1.3 + X25519 + H2)。
- `-shutdown`: 任务完成后自动执行系统关机命令（Windows: `shutdown /s /t 5`，Linux/macOS: `shutdown -h now`，需要具备相应权限）。

### 🔥 增强特性

1. **智能环境检查**:
   - 程序默认要求 `google.com` **无法访问**。这确保了测试是在需要 SNI 分流的环境（如国内直连）下进行的（除非开启 `-debug`）。
2. **自动下载 GeoIP 数据库**:
   - 如果不存在 `GeoLite2-Country.mmdb`，将自动从公共镜像下载。程序结束后会自动删除临时数据库。
3. **本地极速查询**:
   - 使用 MaxMind GeoLite2 本地数据库进行 IP 归属地查询，秒级处理万级数据。
4. **智能分流 & 黑名单**:
   - 自动过滤 `CN`, `HK`, `MO`, `IR`, `RU`, `KP` 等地区的域名。
   - 自动跳过最近失败或已成功的域名。
5. **多级进度显示**:
   - 包含流式读取、测试进度以及批量写入的实时状态。
6. **自动发现目标目录**:
   - 自动寻找并将结果写入 `rust/tgbot/src/resources/sni` 及其子目录（如 `reality/`, `xhttp/`）。
7. **极致性能与智能伸缩 (AIMD)**:
   - 采用 **uTLS** 模拟 Chrome 指纹。
   - **全自动动态并发**: 工具内置了基于 AIMD（加性增/乘性减）算法的并发控制器。它会从 100 并发平稳起步，在网络良好时自动加速最高至 2000 并发；一旦遇到网络封锁或 DNS 限流，会瞬间降低并发保护网络，全程**无需用户干预**。

### 🌟 示例

```bash
./sni_tester -f domains.txt -dns 119.29.29.29 -reality
```

## ⚠️ 注意

- 该工具会自动去除域名后的端口号。
- 解析失败或归属地为黑名单/未知的域名将被记录在对应的 `failed_history*.db` (LevelDB) 中以供后续调优。
