# Aegis Signal Presage 整合設計

## 目標

新增 Signal 平台支援，透過 Presage 函式庫連結為第二設備，實現訊息收發、檔案下載，並整合至現有統一事件分發層 (`BotEvent` → `dispatch_event()`)。

## 架構

```
SignalManager (Arc)
  ├─ store: SqliteStore          ← 持久化 session/keys
  ├─ mutex: Mutex<Manager>       ← &self adapter 方法用
  └─ receive_task: tokio task    ← 獨立 clone 接收訊息

SignalAdapter : BotAdapter       ← Arc<SignalManager>
  ├─ send_message(&self)         ← mutex.lock() → manager.send_message()
  ├─ download_file(&self)        ← manager.get_attachment()
  ├─ edit/delete/callback        ← no-op / 文字替代
  └─ platform() → Signal

runtime.rs
  ├─ CLI: --link-signal          ← 一次性 QR 連結
  ├─ 啟動: load_registered()     ← 自動恢復 session
  └─ 背景: receive_loop()        ← Stream<Received> → BotEvent → dispatch_event()
```

## 訊息映射

| Presage `Received` | BotEvent | 說明 |
|---|---|---|
| `DataMessage { body, attachments:[] }` | `BotEvent::Command` (若 / 開頭) 或 `BotEvent::Message` | 文字 |
| `DataMessage { attachments:[a,..] }` | `BotEvent::Message` + `file_id` | 帶檔案附件 |
| `QueueEmpty` / 其他 | 忽略 | |

## user_id 型別變更 (i64 → String)

全域性變更：所有事件、auth、rate-limiting 中的 `user_id` 從 `i64` 改為 `String`。

| 影響範圍 | 變更 |
|---------|------|
| `MessageEvent.user_id`, `CommandEvent.user_id`, `CallbackEvent.user_id` | `i64` / `String` → 統一 `String` |
| `BotEvent::user_id()` 回傳值 | `i64` → `&str` |
| `AppState::is_authorized`, `is_admin_user`, `record_auth_success` | 參數 `i64` → `&str` |
| 內部 HashMap key (`sessions`, `failed_attempts`) | `HashMap<i64,..>` → `HashMap<String,..>` |
| `admin_id` config (struct + file) | `i64` → `String` |
| Telegram `msg.from.id.0 as i64` | 改為 `.to_string()` |
| Matrix `parse_user_id()` 回傳值 | `i64` → `String` |

## BotAdapter 實作

| trait 方法 | Signal 實作 |
|-----------|------------|
| `send_message(target, content)` | Mutex lock → `manager.send_message(recipient, body, now)` |
| `edit_message(target, msg_id, content)` | 發送新訊息 ("✏️ " + text) — Signal 不支援編輯 |
| `delete_message(...)` | no-op |
| `download_file(file_id)` | `manager.get_attachment(&AttachmentPointer)` — file_id 存 JSON pointer |
| `answer_callback(...)` | no-op（Signal 無 callback query） |
| `platform()` | `Platform::Signal` |
| `capabilities()` | `has_inline_keyboard: false`（與 Matrix 一致） |

## 新增 / 修改檔案

| 檔案 | 動作 | 內容 |
|------|------|------|
| `src/adapters/signal/mod.rs` | 新建 | 模組宣告 |
| `src/adapters/signal/adapter.rs` | 新建 | `SignalAdapter` — BotAdapter 實作 |
| `src/adapters/signal/manager.rs` | 新建 | `SignalManager` — 連結/載入/發送/接收迴圈 |
| `src/main/signal.rs` | 新建 | CLI `--link-signal` + `connect_signal()` |
| `src/main.rs` | 修改 | CLI arg parsing、auto-detect |
| `src/main/runtime.rs` | 修改 | Signal 背景接收任務 |
| `src/main/adapter.rs` | 修改 | `build_adapter()` Signal 分支 |
| `src/adapters/common/trait.rs` | 修改 | `Platform::Signal` variant |
| `src/shared/types.rs` | 修改 | `user_id: String` (所有事件) |
| `src/shared/dispatch.rs` | 修改 | `user_id` 字串比對 |
| `src/shared/commands.rs` | 修改 | `user_id` 字串比對 |
| `src/shared/destruct.rs` | 修改 | `user_id` 字串比對 |
| `src/app/state.rs` | 修改 | `HashMap<String,..>` + `is_authorized(&str)` |
| `src/app/auth.rs` | 修改 | `user_id: &str` |
| `src/bootstrap.rs` | 修改 | `admin_id: String` in config |
| `src/main/config.rs` | 修改 | `admin_id` 型別 |
| `src/main/matrix.rs` | 修改 | Matrix `parse_user_id` 回傳型別 |
| `Cargo.toml` | 修改 | 新增 presage 依賴 |

## 依賴

```toml
[dependencies]
presage = { git = "https://github.com/whisperfish/presage" }
presage-store-sqlite = { git = "https://github.com/whisperfish/presage" }

[patch.crates-io]
curve25519-dalek = { git = 'https://github.com/signalapp/curve25519-dalek', tag = 'signal-curve25519-4.1.3' }
```

## 生命週期

```
main()
├─ --link-signal  → cli::link_signal()     (一次性 QR 連結 → 退出)
│  └─ Manager::link_secondary_device(store, Production, name, tx)
│     └─ 輸出 QR URL → 手機 Signal 掃描 → 儲存 session → 退出
│
└─ 正常啟動        → runtime::run()
   ├─ load_and_validate()
   │  ├─ config.admin_id: String
   │  └─ 讀取 encrypted_config (admin_id 現在是字串)
   ├─ connect_signal()
   │  ├─ SqliteStore::open(path)
   │  ├─ Manager::load_registered(store)
   │  └─ SignalAdapter::new(manager)
   ├─ adapter = build_adapter(telegram, matrix, signal)
   └─ runtime::run(all adapters)
      └─ Signal receive loop (tokio::spawn)
         └─ for msg in manager.receive_messages().await?
            → BotEvent::Message / Command → dispatch_event()
```

## 已知限制

1. Signal 無 callback/inline keyboard — markup 以文字列表渲染（與 Matrix 相同）
2. 無法編輯/刪除已發送訊息
3. `&self` adapter 方法內部短暫持有 mutex
4. `curve25519-dalek` patch 可能與其他依賴衝突（需建置測試）
5. 僅支援第二設備連結模式（不支援手機號碼註冊）
