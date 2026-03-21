# SNI Tester Mobile

基于 Fyne 框架的跨平台 SNI 测试应用，支持 Android/iOS/桌面端。

## 功能

- 文件选择 (TXT/CSV)
- Reality / XHTTP 模式切换
- 实时进度显示
- 日志输出
- 导出 JSON 结果
- 系统通知

## 项目结构

```
sni_tester_mobile/
├── main.go           # Fyne GUI 应用入口
├── go.mod            # Go 模块定义
└── fyne.toml         # Fyne 配置
```

## 依赖安装

```bash
cd sni_tester_mobile
go mod tidy
```

## 运行

```bash
go run .
```

## 打包 Android APK

```bash
# 需要 Android SDK 和 NDK
go install fyne.io/fyne/v2/cmd/fyne@latest
fyne package -os android -app-id com.wwps.snitester
```

## 打包 iOS

```bash
# 需要 macOS 和 Xcode
fyne package -os ios -app-id com.wwps.snitester
```

## 与 sni_tester 配合使用

1. 在手机上运行 `sni_tester_mobile`，完成测试后导出 JSON 文件
2. 将 JSON 文件传到电脑
3. 使用 sni_tester 的 -import 参数导入：

```bash
cd sni_tester
go build -o sni_tester main.go
./sni_tester -import result.json
```

## JSON 格式

导出的 JSON 文件格式：

```json
{
  "version": "1.0",
  "mode": "reality",
  "timestamp": "2026-03-21T10:30:00Z",
  "results": [
    {
      "domain": "example.com",
      "success": true,
      "ip": "1.2.3.4",
      "country": "US",
      "info": "Validated"
    }
  ]
}
```

## 注意事项

- 移动端测试在中国大陆网络环境下运行
- 测试结果会自动按国家分组写入 `rust/tgbot/src/resources/sni/` 目录
- 导入时会自动去重，不会重复添加已存在的域名
