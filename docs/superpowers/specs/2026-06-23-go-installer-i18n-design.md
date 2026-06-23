# go/installer 多语言 (i18n) 支持

**日期**: 2026-06-23
**状态**: 已批准
**范围**: `go/installer/`

---

## 1. 目标

为 `go/installer` CLI 安装器添加中/英/日三语支持，替换所有硬编码中文字符串。

## 2. 支持语言

| 代码 | 语言 | 角色 |
|------|------|------|
| `zh` | 中文 | 默认 / fallback |
| `en` | English | 备选 |
| `ja` | 日本語 | 备选 |

## 3. 文件结构

```
go/installer/
├── main.go              # 现有，print*() 调用改为 T(key, args...)
├── main_test.go         # 现有
├── i18n/
│   ├── i18n.go          # T() API、语言检测、配置读写、init()
│   ├── i18n_test.go     # 单元测试
│   ├── zh.json          # 中文翻译表
│   ├── en.json          # English 翻译表
│   └── ja.json          # 日本語翻訳表
├── go.mod
└── go.sum
```

## 4. 翻译表格式 (JSON)

Key 命名规范：`模块.描述`，允许 `%s` / `%d` 等 `fmt` 占位符用于动态内容。

```json
{
  "banner.title": "WWPS TG Bot 管理工具",
  "banner.version": "当前版本: %s",
  "install.start": "开始安装/更新 TG Bot...",
  "install.downloading": "正在下载: %s",
  "error.download_failed": "下载失败: %s",
  "status.binary": "二进制: %s",
  "prompt.select_lang": "请选择语言 / Select language / 言語を選択"
}
```

规则:
- `zh.json` 约 60 条，每条都必须有
- `en.json` / `ja.json` 含相同 key 集，值翻译为对应语言
- fallback: 若某 key 在目标语言缺失，fallback 到 `zh.json`

## 5. 核心 API

```go
package i18n

// SetLang 设置当前语言 ("zh" | "en" | "ja")
func SetLang(lang string)

// Lang 返回当前语言
func Lang() string

// T 返回 key 在当前语言下的翻译，可选 fmt 参数
func T(key string, args ...interface{}) string

// InitLang 完整初始化流程：检测配置 → 必要时交互询问 → 保存
// 返回选定的语言代码
func InitLang(interactive bool) string
```

## 6. 语言检测优先级

```
--lang 参数 > WWPS_LANG 环境变量 > /etc/wwps/aegis/.lang 文件 > 交互询问(首次) > 默认 "zh"
```

## 7. 运行流程

**交互模式** (`./installer` 无参数):
1. `checkRoot()`, `checkArch()` — 不变
2. `InitLang(true)`:
   - 读 `/etc/wwps/aegis/.lang` → 若存在则直接用
   - 若不存在 → 显示语言选择菜单，读用户输入，写入 `.lang`
3. 后续一切用 `T()` 替换硬编码字符串

**非交互模式** (`--setup-stdin`, `--setup-keyval`):
- `InitLang(false)`:
  - 检查 `--lang` / `WWPS_LANG` → 有则用
  - 读 `.lang` 文件 → 有则用
  - 都没有 → 默认 `zh`（不交互询问）

## 8. 语言配置文件

- 路径: `/etc/wwps/aegis/.lang`
- 内容: 纯文本 `zh` 或 `en` 或 `ja`
- 创建: `i18n.InitLang(true)` 时 `os.MkdirAll(installDir)` 确保目录存在后写入

## 9. 修改点摘要

**main.go:**
- 增加 `import ".../i18n"`
- 所有 `printRed(...)` / `printYellow(...)` / `printGreen(...)` / `printSkyBlue(...)` 中用户可见的字符串用 `i18n.T("key", args...)` 替换
- 所有 `fmt.Print(...)` / `fmt.Sprintf(...)` 中的用户可见字符串同理
- `main()` 中在 banner 前插入 `i18n.InitLang(true)` 调用
- error 返回值字符串不翻译（如 `fmt.Errorf`），仅打印给用户的字符串翻译

**新增 i18n package:**
- `i18n.go` (~120 行)
- `i18n_test.go` (~80 行)
- `zh.json` (~60 条)
- `en.json` (~60 条)
- `ja.json` (~60 条)

## 10. 零新依赖

使用 Go 1.16+ 标准库 `embed` 包嵌入 JSON 文件，不引入任何第三方 i18n 库。

## 11. 测试策略

| 测试 | 内容 |
|------|------|
| `TestT_Basic` | 各语言 key 返回正确翻译 |
| `TestT_Fallback` | 缺失 key fallback 到 zh |
| `TestT_FormatArgs` | `T("key", arg)` 正确插值 |
| `TestSetLang` | SetLang/Lang 往返 |
| `TestDetectLang_Env` | `WWPS_LANG` 环境变量检测 |
| `TestDetectLang_Default` | 无任何配置时返回 "zh" |
| `TestLangFile_Roundtrip` | 写 `.lang` → 读回一致性 |
| `TestAllKeysExist` | en/ja 含 zh 的全部 key |
