# SNI Tester — 手機獨立運行模式

**日期:** 2026-07-11
**狀態:** 設計完成

## 目標

手機獨立運行 SNI 測試，不需電腦終端長期掛起。結果存手機，USB 回傳電腦供 `rust/aegis` 使用。

## 架構

```
┌────────── 手機 (Android) ──────────┐
│  Flutter App                       │
│    ├─ 選網域文件 (file_picker)      │
│    ├─ 設定參數 → /api/start         │
│    ├─ SSE 進度 → 即時 UI           │
│    ├─ 前台服務通知 → Android 不殺   │
│    └─ 匯出結果 → Download/ 目錄     │
│                                    │
│  sni_web (Go, port 18080)          │
│    ├─ Engine (TLS + DNS + GeoIP)   │
│    ├─ BadgerDB (歷史記錄)           │
│    └─ .pb 結果 → outputDir         │
└────────────────────────────────────┘
         │ USB 連線
         ▼
┌──── 電腦 ─────┐
│  make phone-pull → rust/aegis/    │
│  cargo build  → 嵌入 aegis        │
└───────────────┘
```

## 改動清單

### 1. Go 後端 — 參數補全

**檔案:** `cmd/sni_web/handlers.go`

`/api/start` 的 JSON 參數目前只認 5 個（`workers`, `dns`, `ttl`, `force`, `reset`），
Flutter 客戶端發 10 個參數。補全並統一字段名以匹配客戶端。

新增解析: `debug_mode`, `max_lines`, `auto_shutdown`, `geo_proxy`, `timeout_sec`
統一: `workers→max_concurrent`, `ttl→ttl_days`, `force→force_retest`, `reset→reset_history`

**檔案:** `pkg/config.go`

`Config` 結構新增 `Shutdown`、`GeoProxy`、`TimeoutSec`、`MaxLines` 字段。

### 2. Flutter 客戶端 — 修復本地模式

**檔案:** `lib/data/services/api_client.dart`

新增 `uploadContent(String content)` 方法，傳域名文本內容到 `/api/upload`。

**檔案:** `lib/ui/features/home/view_models/home_view_model.dart`

- `startTest()`: 本地模式下，先讀取域名檔案內容，調 `api.uploadContent()` 上傳，再調 `/api/start`。
- 新增 `exportResults()`: 調 `/api/download` 取 zip，寫入外部儲存 `Download/sni_results.zip`。

**檔案:** `lib/ui/features/home/views/home_screen.dart`

手機端顯示「匯出結果」按鈕（`ResultDownloadCard`），調用 `_vm.exportResults()`。

### 3. Android 前台服務

**檔案:** `android/app/src/main/AndroidManifest.xml`

添加：
- `<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />`
- `<uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE" />`
- `<service android:name=".SniForegroundService" android:foregroundServiceType="specialUse" />`

**檔案:** `android/app/src/main/kotlin/com/example/sni_tester/SniForegroundService.kt`

新增前台服務，通過 MethodChannel 調用：

| 通道命令 | 行為 |
|---------|------|
| `startForeground` | 顯示通知 "SNI 測試中..."，`startForeground(id, notification)` |
| `updateProgress` | 更新通知文字 (進度百分比、完成數) |
| `stopForeground` | 更新通知為最終結果，`stopForeground(false)`，`stopSelf()` |

**檔案:** `android/app/src/main/kotlin/com/example/sni_tester/MainActivity.kt`

註冊 MethodChannel 轉發命令到 `SniForegroundService`。

**Flutter 端通知層（新建）:**

`lib/data/services/notification_service.dart`:
- `start()` → MethodChannel `startForeground`
- `update(progress)` → MethodChannel `updateProgress`
- `complete(stats)` → MethodChannel `stopForeground`

ViewModel 整合：
- `startTest()` 結束後調 `NotificationService.start()`
- SSE 回調中調 `NotificationService.update()`
- SSE done 事件中調 `NotificationService.complete()`

### 4. 匯出到 Download 目錄

`HomeViewModel.exportResults()`:
1. 從 `/api/download` 取得 zip bytes
2. 寫入 `getExternalStorageDirectory()/Download/sni_results_<timestamp>.zip`
3. 用 `share_plus`（已安裝）或直接顯示文件路徑

## 數據流

### 輸出格式

```
sni_web outputDir/
  ├── US.pb  (protobuf: DomainList { domains: ["x.com", ...] })
  ├── JP.pb
  └── ...
```

### 回傳電腦

```bash
make phone-pull  # adb pull /data/local/tmp/sni_output/. → rust/aegis/src/resources/sni/
```

### 電腦讀取

Rust `SNISelector::load_domains()` → `sni_proto::DomainList::decode()` → `Vec<String>`

`#[folder = "src/resources/sni/"]` 編譯期嵌入。

## CLI 參數覆蓋

| CLI (`-flag`) | Web API JSON | Flutter Setting |
|---------------|-------------|-----------------|
| `-w` | `max_concurrent` | `fixedWorkers` |
| `-dns` | `dns` | `dns` |
| `-ttl` | `ttl_days` | `ttlDays` |
| `-force` | `force_retest` | `forceRetest` |
| `-reset` | `reset_history` | — |
| `-debug` | `debug_mode` | `debugMode` |
| `-max` | `max_lines` | `maxLines` |
| `-shutdown` | `auto_shutdown` | `autoShutdown` |
| `-p` | `geo_proxy` | `geoProxy` |
| — | `timeout_sec` | `timeoutSec` |

## 平台兼容性

Go 後端：全純 Go，`GOOS=android GOARCH=arm64 CGO_ENABLED=0`，零 CGO 依賴，Makefile 已正確配置。

## 建構

```bash
make flutter-build    # 產出 APK
make flutter-deploy   # 安裝到手機
```
