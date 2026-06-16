# Aegis I18N — 设计文档

**日期**: 2026-06-16
**状态**: 已确认
**版本**: aegis v2.9.1

## 目标

为 `rust/aegis` 添加中/英/日三语国际化支持，在主菜单提供语言切换按钮，语言偏好持久化至加密配置文件。

## 技术选型

使用 **`i18n` crate**（crates.io 上的 `i18n`），而非自定义模块。

- `t!("key")` 宏编译时检查，key 不存在则编译失败
- YAML 翻译文件，`i18n!` 宏编译时嵌入
- 生态成熟，避免手写翻译逻辑导致 bug

## 架构

```
rust/aegis/src/
├── core/
│   └── i18n.rs              # NEW — Lang enum + 全局语言状态
├── resources/
│   └── i18n/
│       ├── zh.yml           # NEW
│       ├── en.yml           # NEW
│       └── ja.yml           # NEW
├── app/
│   └── state.rs             # MOD — lang 字段 + getter/setter
├── bootstrap.rs             # MOD — EncryptedConfig.lang
├── main.rs                  # MOD — i18n 初始化, 欢迎消息翻译
├── adapters/telegram/handlers/
│   ├── menu.rs              # MOD — 所有文本 + 语言切换按钮
│   ├── callback.rs          # MOD — lang:* dispatch
│   ├── ops.rs               # MOD — 文本替换
│   ├── schedule.rs          # MOD — 文本替换
│   ├── singbox.rs           # MOD — 文本替换
│   ├── warp.rs              # MOD — 文本替换
│   ├── xray.rs              # MOD — 文本替换
│   └── message.rs           # MOD — 文本替换
├── adapters/matrix/
│   └── handlers.rs          # MOD — 文本替换
└── app/
    ├── auth.rs              # MOD — 文本替换
    └── destruct_flow.rs     # MOD — 文本替换
```

## 依赖

```toml
[dependencies]
i18n = "0.6"
```

## Lang 枚举与全局状态

```rust
// src/core/i18n.rs
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

i18n::i18n!("src/resources/i18n");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    Zh,
    En,
    Ja,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
            Lang::Ja => "ja",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "en" => Lang::En,
            "ja" => Lang::Ja,
            _ => Lang::Zh,
        }
    }
}

static CURRENT_LANG: RwLock<Lang> = RwLock::new(Lang::Zh);

pub fn set_lang(lang: Lang) {
    *CURRENT_LANG.write().unwrap() = lang;
}

pub fn lang() -> Lang {
    *CURRENT_LANG.read().unwrap()
}
```

## 翻译 Key 设计

每个 YAML 文件按模块/页面层级组织。三个语言文件必须包含完全相同的 key 集合。

```yaml
# zh.yml (示例)
menu:
  title: "🏠 <b>主菜单</b>"
  prompt: "请选择操作类目:"
  monitor: "📊 系统状态"
  users: "👥 用户管理"
  ops: "🛠 运维中心 (Ops)"
  settings: "⚙️ 系统设置"
  back_main: "⬅️ 返回主菜单"
  network_opt: "🌩 网络优化"
  security: "🛡 安全防护"
  sys_cmd: "💻 系统指令"
  log_audit: "📄 日志审计"
  back: "⬅️ 返回"
  refresh: "🔄 刷新"

auth:
  expired: "🚫 会话已过期，请发送 6 位 TOTP 验证码重新认证"
  required: "🔐 请先发送 6 位 TOTP 验证码进行认证（或 /auth <验证码>）。"
  invalid_user: "⚠️ 无法识别用户身份，请访问管理员检查权限"

lang:
  switch: "🌐 Language"
  zh: "中文"
  en: "English"
  ja: "日本語"
  switched: "✅ 语言已切换为 "
```

## 持久化

`EncryptedConfig` 新增字段：

```rust
pub lang: Option<String>,  // "zh" | "en" | "ja" | None
```

- 启动时读取 `config.enc` → 解密 → `lang` → `set_lang()`
- 切换时 `set_lang()` → 加密写回 `config.enc`
- `lang: None`（旧配置）默认中文

## 语言切换 UI

仅在主菜单底部新增一行切换按钮：

```
[📊 系统状态] [👥 用户管理]
[🛠 运维中心]
[⚙️ 系统设置]
[🌐 中文 | English | 日本語]    ← NEW
```

回调: `lang:zh` / `lang:en` / `lang:ja` → 更新状态 → 加密持久化 → 重新渲染主菜单。

## 改动文件汇总（18 文件）

**新增 (4)**: `Cargo.toml`(dep), `zh.yml`, `en.yml`, `ja.yml`, `i18n.rs`  
**修改 (15)**: `Cargo.toml`, `mod.rs`(core), `state.rs`, `bootstrap.rs`, `main.rs`, `menu.rs`, `callback.rs`, `ops.rs`, `schedule.rs`, `singbox.rs`, `warp.rs`, `xray.rs`, `message.rs`, `log.rs`, `matrix/handlers.rs`, `auth.rs`, `destruct_flow.rs`

## 测试

- 现有 `cargo test` 全部通过
- 新增 i18n 单元测试：
  - 默认语言为 `Zh`
  - `set_lang()` 后 `lang()` 正确
  - `Lang::from_str()` 解析正确
  - YAML 三文件 key 集合一致

## 风险

- `i18n` crate 与 `serde` 版本兼容 — 通过 `cargo update` 或 pin 版本解决
- YAML key 不一致 → 编译时由 `i18n!` 宏捕获，不会运行时报错
