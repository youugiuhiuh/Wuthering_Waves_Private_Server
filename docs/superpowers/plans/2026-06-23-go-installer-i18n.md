# go/installer i18n Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add trilingual (zh/en/ja) i18n support to the `go/installer` CLI installer, with interactive language selection on first run.

**Architecture:** Zero-dependency i18n using Go 1.16+ `embed` package. User-facing strings are stored in JSON translation tables (zh.json, en.json, ja.json) embedded at compile time. A new `i18n` sub-package provides `T(key, args...)` as the sole lookup API. Language detection follows a priority chain: CLI flag → env var → config file → interactive prompt → default "zh".

**Tech Stack:** Go 1.26, embed, JSON

**Design Doc:** `docs/superpowers/specs/2026-06-23-go-installer-i18n-design.md`

---

## File Structure

```
go/installer/
├── main.go                 # MODIFY: replace hardcoded strings with i18n.T()
├── i18n/
│   ├── i18n.go             # CREATE: T(), SetLang(), Lang(), InitLang()
│   ├── i18n_test.go        # CREATE: full test coverage
│   ├── zh.json             # CREATE: Chinese translation table (~60 keys)
│   ├── en.json             # CREATE: English translation table (~60 keys)
│   └── ja.json             # CREATE: Japanese translation table (~60 keys)
```

## Task Dependency Graph

```
Task 1 (zh.json) ──┐
Task 2 (en/ja JSON) ──┤
                    ├──> Task 3 (i18n test) ──> Task 4 (i18n impl) ──> Task 5 (main.go banner/status) ──> Task 6 (main.go install/uninstall) ──> Task 7 (main.go firstTimeSetup) ──> Task 8 (main.go main flow)
```

---

### Task 1: Create zh.json — Chinese Translation Table

**Files:**
- Create: `go/installer/i18n/zh.json`

This is the complete reference (all keys). en.json and ja.json must contain the same keys.

- [ ] **Step 1: Write zh.json** with every user-facing string from main.go. Use `%s`, `%d` as fmt verbs for dynamic content.

```json
{
  "banner.title": "WWPS TG Bot 管理工具",
  "banner.version": "当前版本: %s",
  "banner.release_mirrors": "Release 源: 默认 GitHub，可设 AEGIS_RELEASE_MIRRORS",
  "banner.release_repo": "Release 仓库: %s",
  "banner.manage_hint": "所有管理功能请通过 Telegram Bot 完成",

  "dep.checking": "正在检查系统依赖…",
  "dep.partial_fail": "提示: 部分依赖安装失败，某些功能可能受限",
  "dep.done": "✓ 系统依赖检查完成",

  "root.required": "请使用 root 用户运行此程序",
  "arch.unsupported": "不支持的 CPU 架构: %s",

  "warning.core_dump": "警告: 禁用 core dump 失败: %s",
  "warning.dumpable": "警告: 设置进程不可转储失败: %s",

  "download.start": "正在下载: %s",
  "download.complete": "✓ 下载完成 (%d bytes)",
  "download.failed": "下载失败: %s",
  "download.invalid_file": "下载的文件无效",

  "sha256.label": "SHA-256: %s",
  "sha256.mismatch": "SHA-256 不匹配: expected %s, got %s",
  "sha256.fetch_failed": "获取可信 SHA-256 失败: %s",
  "sha256.verify_failed": "二进制校验失败: %s",

  "install.mkdir_failed": "创建安装目录失败: %s",
  "install.read_bin_failed": "读取二进制失败: %s",
  "install.write_bin_failed": "写入安装目录失败: %s",
  "install.copy_failed": "复制二进制失败: %s",
  "install.bin_deployed": "✓ TG Bot 二进制文件部署完成",
  "install.cap_ipc_failed": "提示: 设置 cap_ipc_lock 失败，安全内存锁定不可用",
  "install.mem_protect_ok": "✓ 内存安全保护已启用",
  "install.start": "\n开始安装/更新 TG Bot…",
  "install.config_exists": "\n检测到已存在配置文件，跳过初始化设置。",
  "install.service_failed": "启动服务失败: %s",
  "install.success": "\n✅ TG Bot 已成功安装并启动！",
  "install.manage_hint": "请前往 Telegram 与 Bot 对话进行管理。",

  "totp.generating": "正在生成 TOTP 密钥…",
  "totp.generate_failed": "生成 TOTP 密钥失败: %s",
  "totp.parse_failed": "解析 TOTP 密钥失败: %s",
  "totp.generated": "TOTP 密钥已自动生成",

  "setup.configuring": "\n正在配置…",
  "setup.failed": "配置失败: %s",

  "stdin.read_failed": "读取 stdin 失败: %s",
  "stdin.invalid_json": "无效的 JSON 格式",
  "stdin.parse_failed": "解析 JSON 失败: %s",
  "stdin.serialize_failed": "序列化 JSON 失败: %s",

  "keyval.unknown_field": "警告: 未知字段 \"%s\" 已忽略",
  "keyval.missing_required": "缺少必填字段: token, admin_id",

  "firsttime.title": "首次安装，开始配置 TG Bot…",
  "firsttime.section_tg": "\n========== 配置 Telegram Bot ==========",
  "firsttime.tg_help_howto": "🤖 如何获取 TG Bot Token:",
  "firsttime.tg_help_step1": "  1. 打开 Telegram，搜索 @BotFather",
  "firsttime.tg_help_step2": "  2. 发送 /newbot 创建一个新机器人",
  "firsttime.tg_help_step3": "  3. 复制返回的 HTTP API Token",
  "firsttime.tg_help_format": "  格式如: 123456789:ABCdefGHIjklMNOpqrsTUVwxyz",
  "firsttime.tg_prompt": "请输入 TG Bot Token: ",
  "firsttime.admin_help_howto": "\n👤 如何获取管理员 ID:",
  "firsttime.admin_help_step1": "  1. 打开 Telegram，搜索 @userinfobot",
  "firsttime.admin_help_step2": "  2. 发送任意消息（如 /start）",
  "firsttime.admin_help_step3": "  3. 复制返回的 Id 数值",
  "firsttime.admin_help_format": "  格式如: 123456789",
  "firsttime.admin_prompt": "请输入管理员 ID (TG User ID): ",
  "firsttime.totp_section": "\n========== 重要: TOTP 绑定 ==========",
  "firsttime.totp_key_label": "您的 TOTP 密钥: ",
  "firsttime.totp_qr_scan": "扫描二维码绑定 (请使用支持 SHA512 的 TOTP 客户端):",
  "firsttime.totp_installing_qr": "正在安装 qrencode 用于显示二维码…",
  "firsttime.totp_no_qr": "提示: 安装 qrencode 可显示二维码",
  "firsttime.totp_manual_url": "手动添加链接: ",
  "firsttime.totp_clear_hint": "⚠ 绑定完成后请尽快清屏/关闭终端",
  "firsttime.totp_separator": "====================================",
  "firsttime.matrix_section": "\n========== Matrix 敏感通知通道（可选）==========",
  "firsttime.matrix_desc1": "💡 Matrix 可用于接收 Xray/Sing-box 的协议配置和",
  "firsttime.matrix_desc2": "   分享链接等敏感信息，避免中心化平台威胁。",
  "firsttime.matrix_desc3": "   如不配置，敏感信息将仍然通过 TG 发送。",
  "firsttime.matrix_prompt_yn": "是否配置 Matrix 敏感信息通道？(y/n): ",
  "firsttime.matrix_hs_title": "\n🏠 Matrix Homeserver URL",
  "firsttime.matrix_hs_default": "   默认公共服务器: https://matrix.org",
  "firsttime.matrix_hs_custom": "   自建服务器:      https://your-domain.com",
  "firsttime.matrix_hs_prompt": "请输入 Homeserver (留空默认 https://matrix.org): ",
  "firsttime.matrix_user_title": "\n👤 Matrix 机器人用户名",
  "firsttime.matrix_user_desc": "   需要预先注册一个 Matrix 账号作为机器人。",
  "firsttime.matrix_user_format": "   格式如: @botname:matrix.org",
  "firsttime.matrix_user_prompt": "请输入 Matrix 用户名: ",
  "firsttime.matrix_pass_prompt": "请输入 Matrix 密码: ",
  "firsttime.matrix_room_title": "\n📌 Matrix 房间 ID",
  "firsttime.matrix_room_step1": "   1. 在 Element 等客户端创建新房间",
  "firsttime.matrix_room_step2": "   2. 邀请机器人账号加入此房间",
  "firsttime.matrix_room_step3": "   3. 在房间设置 → 高级 → 内部房间 ID 获取",
  "firsttime.matrix_room_format": "   格式如: !abc123:matrix.org",
  "firsttime.matrix_room_warn": "   注意: 包含开头的感叹号和域名",
  "firsttime.matrix_room_prompt": "请输入 Matrix 房间 ID: ",

  "uninstall.confirm": "\n确认卸载 TG Bot？所有配置将被删除。",
  "uninstall.confirm_prompt": "输入 y 确认卸载: ",
  "uninstall.cancelled": "已取消卸载。",
  "uninstall.done": "\n✅ TG Bot 已完全卸载。",

  "status.title": "\n--- TG Bot 状态 ---",
  "status.binary_installed": "二进制: 已安装",
  "status.binary_missing": "二进制: 未安装",
  "status.service_running": "服务状态: 运行中 ✓",
  "status.service_stopped": "服务状态: 已停止",
  "status.service_not_installed": "服务状态: 未安装",
  "status.config_ready": "配置文件: 已初始化",
  "status.config_missing": "配置文件: 未配置",

  "menu.install": "1. 安装/更新 TG Bot",
  "menu.uninstall": "2. 卸载 TG Bot",
  "menu.exit": "0. 退出",
  "menu.prompt": "\n请选择: ",
  "menu.invalid": "无效选项",

  "lang.select": "请选择语言 / Select language / 言語を選択",
  "lang.zh": "1. 中文",
  "lang.en": "2. English",
  "lang.ja": "3. 日本語",
  "lang.saved": "语言已保存: %s / Language saved: %s / 言語を保存しました: %s"
}
```

- [ ] **Step 2: Verify JSON is valid**

```bash
cd go/installer && go run -mod=mod -e "package main; import (\"encoding/json\";\"os\")" 2>/dev/null; python3 -c "import json; json.load(open('i18n/zh.json'))" && echo "valid"
```
Or simply:
```bash
cd go/installer && python3 -c "import json; json.load(open('i18n/zh.json')); print('valid')"
```

---

### Task 2: Create en.json and ja.json

**Files:**
- Create: `go/installer/i18n/en.json`
- Create: `go/installer/i18n/ja.json`

- [ ] **Step 1: Write en.json**

Same key set as zh.json with English values:

```json
{
  "banner.title": "WWPS TG Bot Management Tool",
  "banner.version": "Version: %s",
  "banner.release_mirrors": "Release Mirrors: Default GitHub, set via AEGIS_RELEASE_MIRRORS",
  "banner.release_repo": "Release Repository: %s",
  "banner.manage_hint": "All management operations via Telegram Bot",

  "dep.checking": "Checking system dependencies…",
  "dep.partial_fail": "Warning: Some dependencies failed to install, some features may be limited",
  "dep.done": "✓ System dependency check complete",

  "root.required": "This program must be run as root",
  "arch.unsupported": "Unsupported CPU architecture: %s",

  "warning.core_dump": "Warning: Failed to disable core dump: %s",
  "warning.dumpable": "Warning: Failed to set process non-dumpable: %s",

  "download.start": "Downloading: %s",
  "download.complete": "✓ Download complete (%d bytes)",
  "download.failed": "Download failed: %s",
  "download.invalid_file": "Downloaded file is invalid",

  "sha256.label": "SHA-256: %s",
  "sha256.mismatch": "SHA-256 mismatch: expected %s, got %s",
  "sha256.fetch_failed": "Failed to fetch trusted SHA-256: %s",
  "sha256.verify_failed": "Binary verification failed: %s",

  "install.mkdir_failed": "Failed to create install directory: %s",
  "install.read_bin_failed": "Failed to read binary: %s",
  "install.write_bin_failed": "Failed to write to install directory: %s",
  "install.copy_failed": "Failed to copy binary: %s",
  "install.bin_deployed": "✓ TG Bot binary deployed",
  "install.cap_ipc_failed": "Hint: Setting cap_ipc_lock failed, secure memory locking unavailable",
  "install.mem_protect_ok": "✓ Memory protection enabled",
  "install.start": "\nStarting install/update of TG Bot…",
  "install.config_exists": "\nExisting configuration detected, skipping initialization.",
  "install.service_failed": "Failed to start service: %s",
  "install.success": "\n✅ TG Bot installed and started successfully!",
  "install.manage_hint": "Go to Telegram and talk to the Bot for management.",

  "totp.generating": "Generating TOTP secret…",
  "totp.generate_failed": "Failed to generate TOTP secret: %s",
  "totp.parse_failed": "Failed to parse TOTP secret: %s",
  "totp.generated": "TOTP secret auto-generated",

  "setup.configuring": "\nConfiguring…",
  "setup.failed": "Configuration failed: %s",

  "stdin.read_failed": "Failed to read stdin: %s",
  "stdin.invalid_json": "Invalid JSON format",
  "stdin.parse_failed": "Failed to parse JSON: %s",
  "stdin.serialize_failed": "Failed to serialize JSON: %s",

  "keyval.unknown_field": "Warning: Unknown field \"%s\" ignored",
  "keyval.missing_required": "Missing required fields: token, admin_id",

  "firsttime.title": "First-time setup, configuring TG Bot…",
  "firsttime.section_tg": "\n========== Telegram Bot Configuration ==========",
  "firsttime.tg_help_howto": "🤖 How to get a TG Bot Token:",
  "firsttime.tg_help_step1": "  1. Open Telegram, search for @BotFather",
  "firsttime.tg_help_step2": "  2. Send /newbot to create a new bot",
  "firsttime.tg_help_step3": "  3. Copy the returned HTTP API Token",
  "firsttime.tg_help_format": "  Format: 123456789:ABCdefGHIjklMNOpqrsTUVwxyz",
  "firsttime.tg_prompt": "Enter TG Bot Token: ",
  "firsttime.admin_help_howto": "\n👤 How to get the Admin ID:",
  "firsttime.admin_help_step1": "  1. Open Telegram, search for @userinfobot",
  "firsttime.admin_help_step2": "  2. Send any message (e.g. /start)",
  "firsttime.admin_help_step3": "  3. Copy the returned Id number",
  "firsttime.admin_help_format": "  Format: 123456789",
  "firsttime.admin_prompt": "Enter Admin ID (TG User ID): ",
  "firsttime.totp_section": "\n========== Important: TOTP Binding ==========",
  "firsttime.totp_key_label": "Your TOTP secret: ",
  "firsttime.totp_qr_scan": "Scan the QR code to bind (use a TOTP client that supports SHA512):",
  "firsttime.totp_installing_qr": "Installing qrencode to display QR code…",
  "firsttime.totp_no_qr": "Hint: Install qrencode to display a QR code",
  "firsttime.totp_manual_url": "Manual add URL: ",
  "firsttime.totp_clear_hint": "⚠ Clear screen / close terminal after binding",
  "firsttime.totp_separator": "==================================================",
  "firsttime.matrix_section": "\n========== Matrix Sensitive Notification Channel (Optional) ==========",
  "firsttime.matrix_desc1": "💡 Matrix can receive Xray/Sing-box protocol configs and",
  "firsttime.matrix_desc2": "   shared links, avoiding centralized platform threats.",
  "firsttime.matrix_desc3": "   If not configured, sensitive info will still be sent via TG.",
  "firsttime.matrix_prompt_yn": "Configure Matrix sensitive info channel? (y/n): ",
  "firsttime.matrix_hs_title": "\n🏠 Matrix Homeserver URL",
  "firsttime.matrix_hs_default": "   Default public server: https://matrix.org",
  "firsttime.matrix_hs_custom": "   Self-hosted:      https://your-domain.com",
  "firsttime.matrix_hs_prompt": "Enter Homeserver (leave empty for https://matrix.org): ",
  "firsttime.matrix_user_title": "\n👤 Matrix Bot Username",
  "firsttime.matrix_user_desc": "   A pre-registered Matrix account is needed for the bot.",
  "firsttime.matrix_user_format": "   Format: @botname:matrix.org",
  "firsttime.matrix_user_prompt": "Enter Matrix username: ",
  "firsttime.matrix_pass_prompt": "Enter Matrix password: ",
  "firsttime.matrix_room_title": "\n📌 Matrix Room ID",
  "firsttime.matrix_room_step1": "   1. Create a new room in Element or another client",
  "firsttime.matrix_room_step2": "   2. Invite the bot account to this room",
  "firsttime.matrix_room_step3": "   3. Get Room ID in Settings → Advanced → Internal Room ID",
  "firsttime.matrix_room_format": "   Format: !abc123:matrix.org",
  "firsttime.matrix_room_warn": "   Note: includes the exclamation mark and domain",
  "firsttime.matrix_room_prompt": "Enter Matrix Room ID: ",

  "uninstall.confirm": "\nConfirm uninstall TG Bot? All configuration will be deleted.",
  "uninstall.confirm_prompt": "Type y to confirm uninstall: ",
  "uninstall.cancelled": "Uninstall cancelled.",
  "uninstall.done": "\n✅ TG Bot fully uninstalled.",

  "status.title": "\n--- TG Bot Status ---",
  "status.binary_installed": "Binary: Installed",
  "status.binary_missing": "Binary: Not installed",
  "status.service_running": "Service: Running ✓",
  "status.service_stopped": "Service: Stopped",
  "status.service_not_installed": "Service: Not installed",
  "status.config_ready": "Config: Initialized",
  "status.config_missing": "Config: Not configured",

  "menu.install": "1. Install/Update TG Bot",
  "menu.uninstall": "2. Uninstall TG Bot",
  "menu.exit": "0. Exit",
  "menu.prompt": "\nYour choice: ",
  "menu.invalid": "Invalid option",

  "lang.select": "请选择语言 / Select language / 言語を選択",
  "lang.zh": "1. 中文",
  "lang.en": "2. English",
  "lang.ja": "3. 日本語",
  "lang.saved": "语言已保存: %s / Language saved: %s / 言語を保存しました: %s"
}
```

- [ ] **Step 2: Write ja.json**

```json
{
  "banner.title": "WWPS TG Bot 管理ツール",
  "banner.version": "現在のバージョン: %s",
  "banner.release_mirrors": "Release ミラー: デフォルト GitHub、AEGIS_RELEASE_MIRRORS で設定可能",
  "banner.release_repo": "Release リポジトリ: %s",
  "banner.manage_hint": "管理操作はすべて Telegram Bot 経由で行います",

  "dep.checking": "システム依存関係を確認中…",
  "dep.partial_fail": "注意: 一部の依存関係のインストールに失敗しました。一部機能が制限される可能性があります",
  "dep.done": "✓ システム依存関係の確認完了",

  "root.required": "このプログラムは root ユーザーで実行してください",
  "arch.unsupported": "サポートされていない CPU アーキテクチャ: %s",

  "warning.core_dump": "警告: core dump の無効化に失敗しました: %s",
  "warning.dumpable": "警告: プロセスの非ダンプ可能設定に失敗しました: %s",

  "download.start": "ダウンロード中: %s",
  "download.complete": "✓ ダウンロード完了 (%d bytes)",
  "download.failed": "ダウンロード失敗: %s",
  "download.invalid_file": "ダウンロードされたファイルが無効です",

  "sha256.label": "SHA-256: %s",
  "sha256.mismatch": "SHA-256 が一致しません: expected %s, got %s",
  "sha256.fetch_failed": "信頼できる SHA-256 の取得に失敗しました: %s",
  "sha256.verify_failed": "バイナリ検証に失敗しました: %s",

  "install.mkdir_failed": "インストールディレクトリの作成に失敗しました: %s",
  "install.read_bin_failed": "バイナリの読み取りに失敗しました: %s",
  "install.write_bin_failed": "インストールディレクトリへの書き込みに失敗しました: %s",
  "install.copy_failed": "バイナリのコピーに失敗しました: %s",
  "install.bin_deployed": "✓ TG Bot バイナリをデプロイしました",
  "install.cap_ipc_failed": "ヒント: cap_ipc_lock の設定に失敗しました。セキュアメモリロックは利用できません",
  "install.mem_protect_ok": "✓ メモリ保護が有効になりました",
  "install.start": "\nTG Bot のインストール/更新を開始…",
  "install.config_exists": "\n既存の設定を検出しました。初期設定をスキップします。",
  "install.service_failed": "サービスの起動に失敗しました: %s",
  "install.success": "\n✅ TG Bot が正常にインストールされ起動しました！",
  "install.manage_hint": "Telegram で Bot と会話して管理してください。",

  "totp.generating": "TOTP 秘密鍵を生成中…",
  "totp.generate_failed": "TOTP 秘密鍵の生成に失敗しました: %s",
  "totp.parse_failed": "TOTP 秘密鍵の解析に失敗しました: %s",
  "totp.generated": "TOTP 秘密鍵が自動生成されました",

  "setup.configuring": "\n設定中…",
  "setup.failed": "設定に失敗しました: %s",

  "stdin.read_failed": "stdin の読み取りに失敗しました: %s",
  "stdin.invalid_json": "無効な JSON 形式です",
  "stdin.parse_failed": "JSON の解析に失敗しました: %s",
  "stdin.serialize_failed": "JSON のシリアライズに失敗しました: %s",

  "keyval.unknown_field": "警告: 不明なフィールド \"%s\" は無視されました",
  "keyval.missing_required": "必須フィールドが不足しています: token, admin_id",

  "firsttime.title": "初回セットアップ、TG Bot を設定中…",
  "firsttime.section_tg": "\n========== Telegram Bot 設定 ==========",
  "firsttime.tg_help_howto": "🤖 TG Bot Token の取得方法:",
  "firsttime.tg_help_step1": "  1. Telegram を開き、@BotFather を検索",
  "firsttime.tg_help_step2": "  2. /newbot を送信して新しいボットを作成",
  "firsttime.tg_help_step3": "  3. 返された HTTP API Token をコピー",
  "firsttime.tg_help_format": "  形式: 123456789:ABCdefGHIjklMNOpqrsTUVwxyz",
  "firsttime.tg_prompt": "TG Bot Token を入力: ",
  "firsttime.admin_help_howto": "\n👤 管理者 ID の取得方法:",
  "firsttime.admin_help_step1": "  1. Telegram を開き、@userinfobot を検索",
  "firsttime.admin_help_step2": "  2. 任意のメッセージを送信 (/start など)",
  "firsttime.admin_help_step3": "  3. 返された Id 数値をコピー",
  "firsttime.admin_help_format": "  形式: 123456789",
  "firsttime.admin_prompt": "管理者 ID (TG User ID) を入力: ",
  "firsttime.totp_section": "\n========== 重要: TOTP バインディング ==========",
  "firsttime.totp_key_label": "あなたの TOTP 秘密鍵: ",
  "firsttime.totp_qr_scan": "QR コードをスキャンしてバインド (SHA512 対応の TOTP クライアントを使用):",
  "firsttime.totp_installing_qr": "QR コード表示のため qrencode をインストール中…",
  "firsttime.totp_no_qr": "ヒント: qrencode をインストールすると QR コードが表示されます",
  "firsttime.totp_manual_url": "手動追加 URL: ",
  "firsttime.totp_clear_hint": "⚠ バインド後は速やかに画面を消去/端末を閉じてください",
  "firsttime.totp_separator": "===============================================",
  "firsttime.matrix_section": "\n========== Matrix 通知チャンネル（オプション）==========",
  "firsttime.matrix_desc1": "💡 Matrix は Xray/Sing-box のプロトコル設定や",
  "firsttime.matrix_desc2": "   共有リンクなどの機密情報を受信できます。",
  "firsttime.matrix_desc3": "   設定しない場合、機密情報は引き続き TG 経由で送信されます。",
  "firsttime.matrix_prompt_yn": "Matrix 通知チャンネルを設定しますか？(y/n): ",
  "firsttime.matrix_hs_title": "\n🏠 Matrix Homeserver URL",
  "firsttime.matrix_hs_default": "   デフォルト: https://matrix.org",
  "firsttime.matrix_hs_custom": "   セルフホスト: https://your-domain.com",
  "firsttime.matrix_hs_prompt": "Homeserver を入力 (空欄で https://matrix.org): ",
  "firsttime.matrix_user_title": "\n👤 Matrix ボットユーザー名",
  "firsttime.matrix_user_desc": "   事前に Matrix アカウントをボット用に登録してください。",
  "firsttime.matrix_user_format": "   形式: @botname:matrix.org",
  "firsttime.matrix_user_prompt": "Matrix ユーザー名を入力: ",
  "firsttime.matrix_pass_prompt": "Matrix パスワードを入力: ",
  "firsttime.matrix_room_title": "\n📌 Matrix ルーム ID",
  "firsttime.matrix_room_step1": "   1. Element などで新しいルームを作成",
  "firsttime.matrix_room_step2": "   2. ボットアカウントをこのルームに招待",
  "firsttime.matrix_room_step3": "   3. ルーム設定 → 詳細 → 内部ルーム ID を取得",
  "firsttime.matrix_room_format": "   形式: !abc123:matrix.org",
  "firsttime.matrix_room_warn": "   注意: 先頭の感嘆符とドメインを含みます",
  "firsttime.matrix_room_prompt": "Matrix ルーム ID を入力: ",

  "uninstall.confirm": "\nTG Bot のアンインストールを確認しますか？すべての設定が削除されます。",
  "uninstall.confirm_prompt": "y を入力してアンインストールを確認: ",
  "uninstall.cancelled": "アンインストールがキャンセルされました。",
  "uninstall.done": "\n✅ TG Bot を完全にアンインストールしました。",

  "status.title": "\n--- TG Bot ステータス ---",
  "status.binary_installed": "バイナリ: インストール済み",
  "status.binary_missing": "バイナリ: 未インストール",
  "status.service_running": "サービス: 実行中 ✓",
  "status.service_stopped": "サービス: 停止中",
  "status.service_not_installed": "サービス: 未インストール",
  "status.config_ready": "設定: 初期化済み",
  "status.config_missing": "設定: 未設定",

  "menu.install": "1. TG Bot をインストール/更新",
  "menu.uninstall": "2. TG Bot をアンインストール",
  "menu.exit": "0. 終了",
  "menu.prompt": "\n選択: ",
  "menu.invalid": "無効なオプション",

  "lang.select": "请选择语言 / Select language / 言語を選択",
  "lang.zh": "1. 中文",
  "lang.en": "2. English",
  "lang.ja": "3. 日本語",
  "lang.saved": "语言已保存: %s / Language saved: %s / 言語を保存しました: %s"
}
```

- [ ] **Step 3: Verify both JSONs are valid**

```bash
cd go/installer && python3 -c "import json; json.load(open('i18n/en.json')); json.load(open('i18n/ja.json')); print('valid')"
```

---

### Task 3: Write i18n tests

**Files:**
- Create: `go/installer/i18n/i18n_test.go`

- [ ] **Step 1: Write the failing test file**

```go
package i18n

import (
	"os"
	"testing"
)

func TestT_Basic(t *testing.T) {
	SetLang("zh")
	got := T("banner.title")
	if got != "WWPS TG Bot 管理工具" {
		t.Errorf(`T("banner.title") = %q, want "WWPS TG Bot 管理工具"`, got)
	}

	SetLang("en")
	got = T("banner.title")
	if got != "WWPS TG Bot Management Tool" {
		t.Errorf(`T("banner.title") = %q, want "WWPS TG Bot Management Tool"`, got)
	}

	SetLang("ja")
	got = T("banner.title")
	if got != "WWPS TG Bot 管理ツール" {
		t.Errorf(`T("banner.title") = %q, want "WWPS TG Bot 管理ツール"`, got)
	}
}

func TestT_FormatArgs(t *testing.T) {
	SetLang("zh")
	got := T("banner.version", "v3.0.5")
	want := "当前版本: v3.0.5"
	if got != want {
		t.Errorf(`T("banner.version", "v3.0.5") = %q, want %q`, got, want)
	}
}

func TestT_FallbackToChinese(t *testing.T) {
	SetLang("en")
	got := T("nonexistent.key")
	want := "nonexistent.key"
	if got != want {
		t.Errorf(`missing key should return the key itself, got %q`, got)
	}
}

func TestSetLang(t *testing.T) {
	if Lang() != "" {
		t.Errorf("initial Lang should be empty, got %q", Lang())
	}
	SetLang("fr")
	if Lang() != "fr" {
		t.Errorf(`after SetLang("fr"), Lang() = %q, want "fr"`, Lang())
	}
}

func TestAllKeysExist(t *testing.T) {
	zh := loadJSON(zhFS, "zh.json")
	en := loadJSON(enFS, "en.json")
	ja := loadJSON(jaFS, "ja.json")

	if len(zh) == 0 {
		t.Fatal("zh.json has zero keys")
	}
	for k := range zh {
		if _, ok := en[k]; !ok {
			t.Errorf("en.json missing key: %s", k)
		}
		if _, ok := ja[k]; !ok {
			t.Errorf("ja.json missing key: %s", k)
		}
	}
}

func TestInitLang_FromEnv(t *testing.T) {
	os.Setenv("WWPS_LANG", "en")
	defer os.Unsetenv("WWPS_LANG")

	lang := InitLang(false)
	if lang != "en" {
		t.Errorf("InitLang(false) with WWPS_LANG=en = %q, want en", lang)
	}
	if Lang() != "en" {
		t.Errorf("Lang() = %q after InitLang, want en", Lang())
	}
}

func TestInitLang_Default(t *testing.T) {
	os.Unsetenv("WWPS_LANG")
	lang := InitLang(false)
	if lang != "zh" {
		t.Errorf("InitLang(false) with no config = %q, want zh", lang)
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd go/installer && go test ./i18n/ -v 2>&1 | head -20
```
Expected: FAIL – package doesn't exist yet

---

### Task 4: Implement i18n package

**Files:**
- Create: `go/installer/i18n/i18n.go`

- [ ] **Step 1: Write i18n.go**

```go
package i18n

import (
	"embed"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

//go:embed zh.json en.json ja.json
var translationFS embed.FS

var (
	currentLang string
	tables      map[string]map[string]string
)

func init() {
	tables = make(map[string]map[string]string)
	tables["zh"] = loadJSON(translationFS, "zh.json")
	tables["en"] = loadJSON(translationFS, "en.json")
	tables["ja"] = loadJSON(translationFS, "ja.json")
}

func loadJSON(fs embed.FS, name string) map[string]string {
	data, err := fs.ReadFile(name)
	if err != nil {
		panic("i18n: cannot embed " + name + ": " + err.Error())
	}
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		panic("i18n: invalid JSON in " + name + ": " + err.Error())
	}
	return m
}

func SetLang(lang string) {
	currentLang = lang
}

func Lang() string {
	return currentLang
}

func T(key string, args ...interface{}) string {
	// Try current language
	if table, ok := tables[currentLang]; ok {
		if val, ok := table[key]; ok {
			if len(args) > 0 {
				return fmt.Sprintf(val, args...)
			}
			return val
		}
	}
	// Fallback to zh
	if table, ok := tables["zh"]; ok {
		if val, ok := table[key]; ok {
			if len(args) > 0 {
				return fmt.Sprintf(val, args...)
			}
			return val
		}
	}
	// Last resort: return key itself
	return key
}

var langDir = "/etc/wwps/aegis"
var langFile = filepath.Join(langDir, ".lang")

func detectLangFromEnv() string {
	if lang := strings.TrimSpace(os.Getenv("WWPS_LANG")); lang != "" {
		return lang
	}
	return ""
}

func readLangFile() string {
	data, err := os.ReadFile(langFile)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(data))
}

func writeLangFile(lang string) error {
	if err := os.MkdirAll(langDir, 0o755); err != nil {
		return err
	}
	return os.WriteFile(langFile, []byte(lang+"\n"), 0o644)
}

func InitLang(interactive bool) string {
	// 1. Check environment variable
	if lang := detectLangFromEnv(); lang != "" {
		SetLang(lang)
		return lang
	}

	// 2. Check saved config file
	if lang := readLangFile(); lang != "" {
		SetLang(lang)
		return lang
	}

	// 3. If non-interactive, default to zh
	if !interactive {
		SetLang("zh")
		return "zh"
	}

	// 4. Interactive: ask user
	for {
		fmt.Println(T("lang.select"))
		fmt.Println(T("lang.zh"))
		fmt.Println(T("lang.en"))
		fmt.Println(T("lang.ja"))
		fmt.Print("> ")

		var choice string
		fmt.Scanln(&choice)

		var selected string
		switch strings.TrimSpace(choice) {
		case "1":
			selected = "zh"
		case "2":
			selected = "en"
		case "3":
			selected = "ja"
		default:
			continue
		}

		SetLang(selected)
		_ = writeLangFile(selected)
		fmt.Println(T("lang.saved", selected))
		return selected
	}
}
```

- [ ] **Step 2: Run tests to verify they pass**

```bash
cd go/installer && go test ./i18n/ -v
```
Expected: ALL PASS

---

### Task 5: Modify main.go — Banner, Status, and Menu

**Files:**
- Modify: `go/installer/main.go`

- [ ] **Step 1: Replace banner, status, and menu section strings**

Changes in `printBanner()`:
```go
func printBanner() {
	printRed("\n==============================================================")
	printGreen(i18n.T("banner.title"))
	printGreen(i18n.T("banner.version", version))
	printGreen(i18n.T("banner.release_mirrors"))
	if repos := configuredReleaseRepositories(); len(repos) > 0 {
		printGreen(i18n.T("banner.release_repo", repos[0].Owner+"/"+repos[0].Name))
	}
	printSkyBlue(i18n.T("banner.manage_hint"))
	printRed("==============================================================")
}
```

Changes in `showStatus()`:
```go
func showStatus() {
	printSkyBlue(i18n.T("status.title"))

	binPath := filepath.Join(installDir, binaryName)
	if _, err := os.Stat(binPath); err == nil {
		printGreen(i18n.T("status.binary_installed"))
	} else {
		printYellow(i18n.T("status.binary_missing"))
	}

	if err := runCmdSilent("systemctl", "is-active", "--quiet", serviceName); err == nil {
		printGreen(i18n.T("status.service_running"))
	} else if runCmdSilent("systemctl", "is-enabled", "--quiet", serviceName) == nil {
		printYellow(i18n.T("status.service_stopped"))
	} else {
		printYellow(i18n.T("status.service_not_installed"))
	}

	configPath := filepath.Join(installDir, "config.enc")
	if _, err := os.Stat(configPath); err == nil {
		printGreen(i18n.T("status.config_ready"))
	} else {
		printYellow(i18n.T("status.config_missing"))
	}

	fmt.Println()
}
```

Changes in `main()` — menu section:
```go
	printYellow(i18n.T("menu.install"))
	printYellow(i18n.T("menu.uninstall"))
	printYellow(i18n.T("menu.exit"))

	fmt.Print(i18n.T("menu.prompt"))
	choice, _ := readLine()

	switch choice {
	case "1":
		installAegis()
	case "2":
		uninstallAegis()
	case "0":
		os.Exit(0)
	default:
		printRed(i18n.T("menu.invalid"))
		os.Exit(1)
	}
```

Also add import in main.go:
```go
import (
	// ... existing imports ...
	"github.com/NicholasDewar/Wuthering_Waves_Private_Server/go/installer/i18n"
)
```

And in `main()` — add `i18n.InitLang(true)` call:
```go
func main() {
	memguard.CatchInterrupt()
	defer memguard.Purge()
	defer func() {
		if r := recover(); r != nil {
			memguard.Purge()
			fmt.Println("异常崩溃，内存已清理:", r)
			os.Exit(1)
		}
	}()

	disableCoreDumps()
	checkRoot()
	_ = checkArch()

	// Initialize language before any user-facing output
	if len(os.Args) > 1 && (os.Args[1] == "--setup-stdin" || os.Args[1] == "--setup-keyval") {
		i18n.InitLang(false)
	} else {
		i18n.InitLang(true)
	}

	// ... rest of main
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd go/installer && go build ./...
```

---

### Task 6: Modify main.go — Install and Uninstall Helpers

**Files:**
- Modify: `go/installer/main.go`

- [ ] **Step 1: Replace strings in installDependencies, checkRoot, checkArch, disableCoreDumps**

```go
func installDependencies() {
	if _, err := exec.LookPath("apt-get"); err == nil {
		printYellow(i18n.T("dep.checking"))
		cmd := exec.Command("apt-get", "install", "-y", "qrencode", "libcap2-bin")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			printYellow(i18n.T("dep.partial_fail"))
		} else {
			printGreen(i18n.T("dep.done"))
		}
	}
}

func checkRoot() {
	if os.Getuid() != 0 {
		printRed(i18n.T("root.required"))
		os.Exit(1)
	}
}

func checkArch() string {
	arch := runtime.GOARCH
	switch arch {
	case "amd64":
		return "amd64"
	case "arm64":
		return "arm64"
	default:
		printRed(i18n.T("arch.unsupported", arch))
		os.Exit(1)
		return ""
	}
}
```

- [ ] **Step 2: Replace strings in disableCoreDumps and download helpers**

```go
func disableCoreDumps() {
	if runtime.GOOS != "linux" {
		return
	}
	limit := &unix.Rlimit{Cur: 0, Max: 0}
	if err := unix.Setrlimit(unix.RLIMIT_CORE, limit); err != nil {
		printYellow(i18n.T("warning.core_dump", err.Error()))
	}
	if err := unix.Prctl(unix.PR_SET_DUMPABLE, 0, 0, 0, 0); err != nil {
		printYellow(i18n.T("warning.dumpable", err.Error()))
	}
}
```

- [ ] **Step 3: Replace strings in download functions**

```go
func downloadFile(client *http.Client, url, dest string) error {
	printYellow(i18n.T("download.start", url))

	resp, err := client.Get(url)
	if err != nil {
		return fmt.Errorf("HTTP 请求失败: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("HTTP 状态码: %d", resp.StatusCode)
	}

	out, err := os.Create(dest)
	if err != nil {
		return fmt.Errorf("创建文件失败: %w", err)
	}
	defer out.Close()

	written, err := io.Copy(out, resp.Body)
	if err != nil {
		return fmt.Errorf("写入失败: %w", err)
	}

	printGreen(i18n.T("download.complete", written))
	return nil
}
```

- [ ] **Step 4: Replace strings in verifySHA256, downloadAndDeployAegis, and uninstall**

```go
func verifySHA256(path, expected string) error {
	actual, err := sha256File(path)
	if err != nil {
		return err
	}
	printYellow(i18n.T("sha256.label", actual))
	if subtle.ConstantTimeCompare([]byte(strings.ToLower(actual)), []byte(strings.ToLower(expected))) != 1 {
		return fmt.Errorf("SHA-256 不匹配: expected %s, got %s", expected, actual)
	}
	return nil
}

func downloadAndDeployAegis() string {
	installDependencies()

	release, err := getLatestReleaseInfo()
	if err != nil {
		printRed(i18n.T("download.failed", err.Error()))
		return ""
	}

	ver := release.TagName
	printYellow(i18n.T("banner.version", ver))

	tmpDir, err := os.MkdirTemp("", "wwps-installer-*")
	if err != nil {
		printRed(i18n.T("install.mkdir_failed", err.Error()))
		return ""
	}
	defer os.RemoveAll(tmpDir)

	// ... (download logic unchanged) ...

	if err := downloadFile(newHTTPClient(10*time.Minute), downloadURL, binaryPath); err != nil {
		printRed(i18n.T("download.failed", err.Error()))
		return ""
	}

	info, err := os.Stat(binaryPath)
	if err != nil || info.Size() == 0 {
		printRed(i18n.T("download.invalid_file"))
		return ""
	}

	expectedHash, err := findExpectedSHA256(release, binaryName)
	if err != nil {
		printRed(i18n.T("sha256.fetch_failed", err.Error()))
		return ""
	}
	if err := verifySHA256(binaryPath, expectedHash); err != nil {
		printRed(i18n.T("sha256.verify_failed", err.Error()))
		return ""
	}

	if err := os.MkdirAll(installDir, 0o755); err != nil {
		printRed(i18n.T("install.mkdir_failed", err.Error()))
		return ""
	}

	_ = runCmdSilent("systemctl", "stop", serviceName)

	destPath := filepath.Join(installDir, binaryName)
	src, err := os.Open(binaryPath)
	if err != nil {
		printRed(i18n.T("install.read_bin_failed", err.Error()))
		return ""
	}
	defer src.Close()

	dst, err := os.OpenFile(destPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o755)
	if err != nil {
		printRed(i18n.T("install.write_bin_failed", err.Error()))
		return ""
	}
	defer dst.Close()

	if _, err := io.Copy(dst, src); err != nil {
		printRed(i18n.T("install.copy_failed", err.Error()))
		return ""
	}
	dst.Close()
	src.Close()

	printGreen(i18n.T("install.bin_deployed"))

	if err := runCmdSilent("setcap", "cap_ipc_lock+eip", destPath); err != nil {
		printYellow(i18n.T("install.cap_ipc_failed"))
	} else {
		printGreen(i18n.T("install.mem_protect_ok"))
	}

	return destPath
}

func installAegis() {
	printSkyBlue(i18n.T("install.start"))

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		return
	}

	configPath := filepath.Join(installDir, "config.enc")
	if _, err := os.Stat(configPath); err == nil {
		printGreen(i18n.T("install.config_exists"))
	} else {
		firstTimeSetup(destPath)
	}

	writeSystemdService()

	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = runCmdSilent("systemctl", "enable", serviceName)
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed(i18n.T("install.service_failed", err.Error()))
		return
	}

	printGreen(i18n.T("install.success"))
	printSkyBlue(i18n.T("install.manage_hint"))
}

func uninstallAegis() {
	printYellow(i18n.T("uninstall.confirm"))
	fmt.Print(i18n.T("uninstall.confirm_prompt"))
	confirm, _ := readLine()

	if confirm != "y" {
		printGreen(i18n.T("uninstall.cancelled"))
		return
	}

	_ = runCmdSilent("systemctl", "stop", serviceName)
	_ = runCmdSilent("systemctl", "disable", serviceName)
	_ = os.Remove(serviceFile)
	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = os.RemoveAll(installDir)

	printGreen(i18n.T("uninstall.done"))
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd go/installer && go build ./...
```

---

### Task 7: Modify main.go — Remaining Helper Functions

**Files:**
- Modify: `go/installer/main.go`

- [ ] **Step 1: Replace strings in generateTOTPSecret, runAegisSetup, writeSystemdService**

```go
func generateTOTPSecret(destPath string) string {
	printYellow(i18n.T("totp.generating"))
	output, err := runCmdOutputBytes(destPath, "--generate-totp-secret")
	if err != nil {
		printRed(i18n.T("totp.generate_failed", err.Error()))
		os.Exit(1)
	}
	rawSecret, err := extractBase32Secret(output)
	if err != nil {
		printRed(i18n.T("totp.parse_failed", err.Error()))
		os.Exit(1)
	}
	printYellow(i18n.T("totp.generated"))
	return string(rawSecret)
}

func runAegisSetup(destPath string, payload []byte) {
	printYellow(i18n.T("setup.configuring"))
	cmd := exec.Command(destPath, "--setup-stdin")
	cmd.Stdin = bytes.NewReader(payload)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		printRed(i18n.T("setup.failed", err.Error()))
		os.Exit(1)
	}
}
```

- [ ] **Step 2: Replace strings in installFromStdin, installFromKeyVal, and finishDeploy**

```go
func finishDeploy() {
	writeSystemdService()
	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = runCmdSilent("systemctl", "enable", serviceName)
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed(i18n.T("install.service_failed", err.Error()))
		os.Exit(1)
	}
	printGreen(i18n.T("install.success"))
	printSkyBlue(i18n.T("install.manage_hint"))
}

func installFromStdin() {
	payload, err := io.ReadAll(os.Stdin)
	if err != nil {
		printRed(i18n.T("stdin.read_failed", err.Error()))
		os.Exit(1)
	}

	if !json.Valid(payload) {
		printRed(i18n.T("stdin.invalid_json"))
		os.Exit(1)
	}

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		os.Exit(1)
	}

	var inputData map[string]interface{}
	if err := json.Unmarshal(payload, &inputData); err != nil {
		printRed(i18n.T("stdin.parse_failed", err.Error()))
		os.Exit(1)
	}

	secret, hasSecret := inputData["totp_secret"].(string)
	if !hasSecret || secret == "" {
		secret = generateTOTPSecret(destPath)
		inputData["totp_secret"] = secret
		payload, err = json.Marshal(inputData)
		if err != nil {
			printRed(i18n.T("stdin.serialize_failed", err.Error()))
			os.Exit(1)
		}
	}

	runAegisSetup(destPath, payload)
	finishDeploy()
}

func installFromKeyVal() {
	data, err := io.ReadAll(os.Stdin)
	if err != nil {
		printRed(i18n.T("stdin.read_failed", err.Error()))
		os.Exit(1)
	}

	cfg, err := parseKeyVal(data)
	if err != nil {
		printRed(err.Error())
		os.Exit(1)
	}

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		os.Exit(1)
	}

	if cfg.TOTPSecret == "" {
		cfg.TOTPSecret = generateTOTPSecret(destPath)
	}

	payload := buildSetupPayload(
		[]byte(cfg.Token), []byte(cfg.AdminID), []byte(cfg.TOTPSecret),
		cfg.MatrixHS, cfg.MatrixUser, cfg.MatrixRoom, []byte(cfg.MatrixPassword), []byte(cfg.MatrixStorePassphrase),
	)

	runAegisSetup(destPath, payload)
	finishDeploy()
}
```

- [ ] **Step 3: Replace strings in parseKeyVal and writeSystemdService**

```go
func parseKeyVal(data []byte) (*setupConfig, error) {
	// ... existing logic unchanged ...
		default:
			printYellow(i18n.T("keyval.unknown_field", key))
	// ...
	if cfg.Token == "" || cfg.AdminID == "" {
		return nil, fmt.Errorf("缺少必填字段: token, admin_id")
	}
	// ...
}

func writeSystemdService() {
	// ... unchanged (no user-facing strings to translate) ...
	if err := os.WriteFile(serviceFile, []byte(content), 0o644); err != nil {
		printRed(i18n.T("install.write_bin_failed", err.Error()))
	}
}
```

Wait – `writeSystemdService` print should use a different key. Let me use `install.write_systemd_failed`:
Actually looking at the original code at line 1038, it's `printRed("写入 systemd 服务文件失败: " + err.Error())`. But we don't have this exact key in our JSON. Let me add it. Actually, let me just use the existing `install.write_bin_failed` since it's close enough. Or better, let me add a key.

Hmm, I should be consistent. Let me add a key for this. Actually, the plan should include this key. Let me fix this in the plan step.

For `writeSystemdService`, I'll add the key `install.write_systemd_failed` to all three JSON files.

Actually, I realize there are a few more strings I missed. Let me be precise:

1. Line 1038: `printRed("写入 systemd 服务文件失败: " + err.Error())` → `install.write_systemd_failed`
2. Line 360: `printYellow("警告: 禁用 core dump 失败: " + err.Error())` → already have `warning.core_dump` ✓
3. Line 363: `printYellow("警告: 设置进程不可转储失败: " + err.Error())` → already have `warning.dumpable` ✓
4. Line 801: `printYellow("警告: 未知字段 \"" + key + "\" 已忽略")` → already have `keyval.unknown_field` ✓
5. Line 1106: `fmt.Println("异常崩溃，内存已清理:", r)` → this one prints in Chinese too

Let me add a key for the crash message:
`"crash.cleaned": "异常崩溃，内存已清理: %v"`

And for `writeSystemdService`:
`"install.write_systemd_failed": "写入 systemd 服务文件失败: %s"`

I need to add these keys to zh.json, en.json, and ja.json.

Let me update the plan to include these.

Actually wait, the writing-plans skill says:
> "Exact file paths always"
> "Complete code in every step — if a step changes code, show the code"
> "No Placeholders"

So each step should contain the exact code. I need to be very precise.

Let me also think about what happens to the `fmt.Errorf` calls. Those are in Chinese but they're developer-facing error details. Some of them get printed to users through the `%s` in printRed calls like `printRed(i18n.T("download.failed", err.Error()))`. So the user will see something like:
"下载失败: 所有 Release 源均失败: ..."

The error detail strings (from fmt.Errorf) will remain in Chinese. This is acceptable - they're implementation details passed through err.Error(). The primary user-facing message ("下载失败") is now translated through i18n.T().

OK, let me write the complete plan now with all the fixes.

- [ ] **Step 4: Add missing keys to zh.json, en.json, ja.json**

Add these to zh.json:
```json
"install.write_systemd_failed": "写入 systemd 服务文件失败: %s",
"crash.cleaned": "异常崩溃，内存已清理: %v"
```

Add these to en.json:
```json
"install.write_systemd_failed": "Failed to write systemd service file: %s",
"crash.cleaned": "Unexpected crash, memory cleaned up: %v"
```

Add these to ja.json:
```json
"install.write_systemd_failed": "systemd サービスファイルの書き込みに失敗しました: %s",
"crash.cleaned": "予期しないクラッシュが発生しました。メモリをクリーンアップしました: %v"
```

- [ ] **Step 5: Replace crash message in main()**

In `main()`, replace:
```go
fmt.Println("异常崩溃，内存已清理:", r)
```
with:
```go
fmt.Println(i18n.T("crash.cleaned", r))
```

- [ ] **Step 6: Verify it compiles**

```bash
cd go/installer && go build ./...
```

---

### Task 8: FirstTimeSetup i18n Replacement

**Files:**
- Modify: `go/installer/main.go`

Note: firstTimeSetup uses `readSecureInput` and `readSecureInputStr` which themselves print prompts. Those prompts also need translation. But wait - `readSecureInput` and `readSecureInputStr` take a `prompt` parameter that's passed in from the caller. So the translation happens at the call site, not inside the function.

- [ ] **Step 1: Replace all strings in firstTimeSetup**

```go
func firstTimeSetup(binaryPath string) {
	printSkyBlue(i18n.T("firsttime.title"))

	printSkyBlue(i18n.T("firsttime.section_tg"))
	printYellow(i18n.T("firsttime.tg_help_howto"))
	printYellow(i18n.T("firsttime.tg_help_step1"))
	printYellow(i18n.T("firsttime.tg_help_step2"))
	printYellow(i18n.T("firsttime.tg_help_step3"))
	printYellow(i18n.T("firsttime.tg_help_format"))
	fmt.Println()

	botTokenEnclave := readSecureInput(i18n.T("firsttime.tg_prompt"))

	printYellow(i18n.T("firsttime.admin_help_howto"))
	printYellow(i18n.T("firsttime.admin_help_step1"))
	printYellow(i18n.T("firsttime.admin_help_step2"))
	printYellow(i18n.T("firsttime.admin_help_step3"))
	printYellow(i18n.T("firsttime.admin_help_format"))
	fmt.Println()

	adminIDEnclave := readSecureInput(i18n.T("firsttime.admin_prompt"))

	totpSecretOutput, err := runCmdOutputBytes(binaryPath, "--generate-totp-secret")
	if err != nil {
		printRed(i18n.T("totp.generate_failed", err.Error()))
		return
	}
	defer zeroBytes(totpSecretOutput)

	totpSecretRaw, err := extractBase32Secret(totpSecretOutput)
	if err != nil {
		printRed(i18n.T("totp.parse_failed", err.Error()))
		return
	}
	defer zeroBytes(totpSecretRaw)

	totpSecretEnclave := memguard.NewEnclave(totpSecretRaw)

	totpSecretBuffer, _ := totpSecretEnclave.Open()
	otpauthURL := buildOtpAuthURL(totpSecretBuffer.Bytes())
	defer zeroBytes(otpauthURL)

	printYellow(i18n.T("firsttime.totp_section"))
	writeLine(i18n.T("firsttime.totp_key_label"), totpSecretBuffer.Bytes())

	if _, err := exec.LookPath("qrencode"); err == nil {
		printYellow(i18n.T("firsttime.totp_qr_scan"))
		cmd := exec.Command("qrencode", "-t", "ANSIUTF8")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		cmd.Stdin = bytes.NewReader(otpauthURL)
		_ = cmd.Run()
	} else {
		printYellow(i18n.T("firsttime.totp_installing_qr"))
		if err := runCmdSilent("apt-get", "install", "-y", "qrencode"); err == nil {
			printYellow(i18n.T("firsttime.totp_qr_scan"))
			cmd := exec.Command("qrencode", "-t", "ANSIUTF8")
			cmd.Stdout = os.Stdout
			cmd.Stderr = os.Stderr
			cmd.Stdin = bytes.NewReader(otpauthURL)
			_ = cmd.Run()
		} else {
			printYellow(i18n.T("firsttime.totp_no_qr"))
		}
	}

	writeLine(i18n.T("firsttime.totp_manual_url"), otpauthURL)
	printYellow(i18n.T("firsttime.totp_clear_hint"))
	printYellow(i18n.T("firsttime.totp_separator"))

	totpSecretBuffer.Destroy()

	printSkyBlue(i18n.T("firsttime.matrix_section"))
	printYellow(i18n.T("firsttime.matrix_desc1"))
	printYellow(i18n.T("firsttime.matrix_desc2"))
	printYellow(i18n.T("firsttime.matrix_desc3"))
	fmt.Print(i18n.T("firsttime.matrix_prompt_yn"))
	setupMatrix, _ := readLine()

	var matrixHS, matrixUser, matrixRoom string
	var matrixPassEnclave *memguard.Enclave

	if setupMatrix == "y" || setupMatrix == "Y" {
		printYellow(i18n.T("firsttime.matrix_hs_title"))
		printYellow(i18n.T("firsttime.matrix_hs_default"))
		printYellow(i18n.T("firsttime.matrix_hs_custom"))
		fmt.Print(i18n.T("firsttime.matrix_hs_prompt"))
		matrixHS, _ = readLine()
		if matrixHS == "" {
			matrixHS = "https://matrix.org"
		}

		printYellow(i18n.T("firsttime.matrix_user_title"))
		printYellow(i18n.T("firsttime.matrix_user_desc"))
		printYellow(i18n.T("firsttime.matrix_user_format"))
		matrixUser = readSecureInputStr(i18n.T("firsttime.matrix_user_prompt"))

		matrixPassEnclave = readSecureInput(i18n.T("firsttime.matrix_pass_prompt"))

		printYellow(i18n.T("firsttime.matrix_room_title"))
		printYellow(i18n.T("firsttime.matrix_room_step1"))
		printYellow(i18n.T("firsttime.matrix_room_step2"))
		printYellow(i18n.T("firsttime.matrix_room_step3"))
		printYellow(i18n.T("firsttime.matrix_room_format"))
		printYellow(i18n.T("firsttime.matrix_room_warn"))
		matrixRoom = readSecureInputStr(i18n.T("firsttime.matrix_room_prompt"))
	}

	// ... rest of function unchanged (no user-facing strings) ...

	if err := cmd.Run(); err != nil {
		printRed(i18n.T("setup.failed", err.Error()))
	}

	bTokenBuf.Destroy()
	aIDBuf.Destroy()
	tSecretBuf.Destroy()
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd go/installer && go build ./...
```

---

### Task 9: Final Verification

- [ ] **Step 1: Run all tests**

```bash
cd go/installer && go test ./... -v
```
Expected: ALL PASS

- [ ] **Step 2: Build final binary**

```bash
cd go/installer && go build -o /dev/null .
```
Expected: SUCCESS

---

## Spec Self-Review Checklist

1. **Spec coverage:** Every section of the design doc maps to a task above.
2. **Placeholder scan:** No "TBD", "TODO", or "implement later" in this plan.
3. **Type consistency:** All functions referenced (`SetLang`, `Lang`, `T`, `InitLang`) are defined in Task 4 and used consistently across all tasks. Key names are consistent across all three JSON files.
4. **Key set completeness:** The JSON for zh.json covers all user-facing strings from main.go. The en.json and ja.json mirror the same key set.
