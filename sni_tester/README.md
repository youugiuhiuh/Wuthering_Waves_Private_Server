# SNI 测试工具使用说明

这是一个高性能的并发 SNI 测试工具，使用 Go 编写。

## 📍 位置
`/home/ub/WorkCodes/wwps/sni_tester/`

## 🚀 编译
如果尚未编译，请运行：
```bash
cd /home/ub/WorkCodes/wwps/sni_tester
go build -o sni_tester main.go
```

## 📝 使用方法

```bash
./sni_tester -f <输入文件路径>
```

### 参数说明
- `-f`: 包含待测试域名的 TXT 文件路径 (一行一个，支持 `#` 注释)。
- `-debug`: (可选) 开启调试模式，显示详细握手日志 (进度条将自动关闭)。

### 🔥 增强特性
1. **自动下载 GeoIP 数据库**:
   - 程序启动时会检查当前目录下是否存在 `GeoLite2-Country.mmdb`。
   - 如果不存在，将自动从公共镜像下载 (约 4MB)，并显示下载进度条。
   - **自动清理**: 如果通过自动下载获取的数据库，程序结束后会自动删除，保持目录整洁。
2. **本地极速查询**:
   - 使用 MaxMind GeoLite2 本地数据库进行 IP 归属地查询，无需等待 API 响应，速度极快且无限制。
3. **双重进度条**:
   - 包含"Testing SNIs" (测试中) 和 "Writing Files" (写入中) 两个阶段的独立进度显示。
4. **智能分流**:
   - `CN` (中国) 域名自动丢弃。
   - 其他国家域名自动归类追加。
5. **极致性能 & 真实模拟**:
   - 采用 **uTLS** 库模拟 Chrome 浏览器指纹，与 Xray Core 行为一致。
   - 默认 **1000 并发**，测试速度提升 50 倍以上。

### 🌟 示例

1. **准备测试文件**:
   创建一个名为 `test_domains.txt` 的文件。

2. **运行测试**:
   ```bash
   ./sni_tester -f test_domains.txt
   ```

3. **预期输出**:
   ```text
   loaded 5 domains from test_domains.txt
   [PASS] www.google.com (IP: 142.250.1.1, Country: US)
   [PASS] www.bing.com (IP: 110.242.68.3, Country: US)
   Successfully appended 1 domains to US.txt
   Successfully appended 1 domains to US.txt
   ```

## ⚠️ 注意
- 该工具会自动去除域名后的端口号 (如 `:443`)。
- 请确保系统路径中已安装 `xray`，或者通过 `-x` 指定准确路径。
