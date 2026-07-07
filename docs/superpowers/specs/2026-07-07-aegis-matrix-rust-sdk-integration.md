# Matrix Rust SDK Integration (rust/aegis)

**Date**: 2026-07-07
**Status**: Design / Reference
**Platform**: Matrix

## 1. 目標與範圍

- 支持平台：Matrix
- 支持功能：文字命令分發、狀態查詢、Xray/SingBox/Ops/Warp 管理
- **不支持**：自毀流程、調度管理（保留未來擴展接口）
- 交互方式：純文字命令（符合 Matrix 無 inline keyboard 的平台特性）

## 2. matrix-rust-sdk 關鍵 API

官方倉庫：<https://github.com/matrix-org/matrix-rust-sdk>（v0.18.0, Apache-2.0）

| API | 用途 | 對應 adapter 方法 |
|-----|------|-------------------|
| `Client::login_username(&self, user, password, device_id, initial_device_name)` | 登入 Matrix 伺服器 | 初始連接 |
| `Client::sync_once(SyncSettings::default())` | 一次性同步 | 輪詢事件 |
| `Client::sync_stream(SyncSettings::default())` | 持續同步串流 | 事件監聽 |
| `Room::send(RoomMessageEventContent)` | 發送純文字訊息 | `send_message` |
| `Room::send_attachment(name, mime, data, config)` | 發送附件 | `send_message` (sensitive) |
| `Room::redact(OwnedEventId, reason, txn_id)` | 刪除訊息（redact） | `delete_message` |
| `RoomMessageEventContent::make_replacement(new, meta)` | 編輯訊息（replacement） | `edit_message` |

### 事件 ID 格式

Matrix 事件 ID 格式為 `$event_id:server`，與我們的 `MessageId(String)` 可直接兼容（parse 後會取得 `OwnedEventId`）。

## 3. 現有 `MatrixAdapter` 解析

文件位置：`src/adapters/matrix/adapter.rs`

### BotAdapter 接口實現

| 方法 | 實作方式 |
|------|----------|
| `send_message` | 敏感文字 → `send_attachment`（加密防護），一般文字 → `Room::send` |
| `edit_message` | 解析 `msg_id` 為 `OwnedEventId`，用 `make_replacement` 建立編輯 |
| `delete_message` | 解析 `msg_id` 為 `OwnedEventId`，呼叫 `Room::redact` |
| `platform` | 返回 `Platform::Matrix` |

### 注意事項

- 一個 `MatrixAdapter` 實例對應一個 Matrix 房間
- `target` 參數被忽略（房間在建構時固定）
- 如需多房間支持，需建立多個 `MatrixAdapter` 實例

## 4. Matrix 命令系統

文件位置：`src/adapters/matrix/commands.rs`

### 命令格式

```
auth <code>      - TOTP 驗證
help / h         - 幫助訊息
status           - 系統狀態
menu             - 功能選單
xray status/add/del/pq status/pq gen - Xray 管理
sb / singbox status/add/del          - SingBox 管理
ops reload/upgrade/maintenance/bbr3/geo/fw - 系統操作
warp status/install/uninstall        - WARP 管理
destruct         - 暫不支持（提示使用 Telegram）
sched / schedule list/add/del        - 暫不支持
```

### 解析器

命令解析器在 `commands.rs` 中的 `parse(text: &str) -> Command` 函數：

- 區分大小寫（全部小寫化後匹配）
- 子命令按 `Command` enum 的不同 variant 分發
- 未知命令返回 `Command::Unknown(String)`

### 分發器

`handlers.rs` 中的 `dispatch(cmd, adapter, target, state)`：

- 根據 `Command` variant 調用對應業務邏輯
- 所有輸出透過 `adapter.send_message()` 完成
- 獨立於 Telegram 的 callback 機制

## 5. 與平台無關業務層的對接建議

### Destruct / Schedule 未來擴展

若未來需要在 Matrix 支持自毀流程：

1. 文字命令 `destruct` → 映射為 `DestructInput::Button(BTN_DESTROY_ASK)`
2. 使用 `app::destruct_flow::handle_input` 處理業務邏輯
3. 從 `DestructOutput` 中提取文字，用 `adapter.send_message()` 渲染
4. **問題**：Matrix 無 inline keyboard，`DestructOutput::Prompt { buttons }` 中的按鈕需降級為純文字回覆

### 複用建議

- 對於純文字平台（Matrix），保留獨立命令解析有利於維持平台直覺
- `DestructInput::Button` 可從文字命令映射（例如 `destruct confirm` → `BTN_DESTROY_CONFIRM`）

## 6. 已知限制

- **無 inline keyboard**：用純文字回覆或 Markdown list 替代按鈕
- **E2EE 房間**：需要額外設備驗證流程，matrix-sdk-crypto 需初始化
- **文件上傳**：大小限制依 homeserver 配置而定
- **同步延遲**：`sync_once` / `sync_stream` 的輪詢間隔影響即時性

## 7. TODO / 未來方向

- [ ] 接入更多業務命令（如 batch result 查詢）
- [ ] 若 Matrix 客戶端支持互動按鈕，考慮按鈕式交互降級方案
- [ ] 評估建立統一命令註冊表（避免 Telegram / Matrix / Discord 各自維護命令列表）
- [ ] 考慮加入 prefix 命令（如 `!status`）與現有純文字命令並存
