# Spec: 多架構發布 (amd64 / arm64) 與 x86-64 指令集基線鎖定

## 1. 目標與背景 (Objective & Background)

### 1.1 背景
- **x86-64 指令集外洩導致崩潰**：在 2.5.1 版本發布中，兩台生產 VPS（CPU 為 Intel Xeon E5-2680 v2，Ivy Bridge 架構，不支援 AVX2/BMI1）升級後，`wwps-aegis.service` 於啟動時遭遇 `SIGILL`（`status=4/ILL`，非法指令）循環崩潰。經分析，因 CI 建置環境（GCC 15+ / 新版 LLVM）在編譯 C 依賴（`ring` 的 `curve25519.c` 中 `table_select`）時未強制 generic baseline，編譯器外洩了 BMI1 `andn` 指令（`c4 e2 e8 f2 d0`）。
- **ARM64 支援脫節**：`README.md` 與 `go/installer` 名義上支援 `arm64`，但 CI（`.github/workflows/public-release.yml` 與 `.forgejo/`）僅編譯 `GOARCH=amd64` 與預設本機 x86_64 二進位，Release 僅有單一名稱 `aegis` 與 `installer`，導致 ARM VPS 用戶完全無法安裝使用。

### 1.2 目標
1. **多架構矩陣發布**：CI 構建矩陣同時支援 `linux/amd64` 與 `linux/arm64`，發布明確帶有架構後綴的二進位產物。
2. **x86-64 通用基線鎖定**：嚴格鎖定編譯參數（`RUSTFLAGS="-C target-cpu=generic"`、`CFLAGS="-march=x86-64 -mtune=generic"`），保證 x86-64 產物可在任何 64 位元 x86 機器（含舊款平價 VPS）上穩定運行，杜絕未經 CPUID 動態探測的指令集外洩。
3. **客戶端自動架構探測與精確匹配**：
   - `go/installer` 依 `runtime.GOARCH`（`amd64` / `arm64`）精確匹配 `aegis-linux-${arch}`，向下相容舊版無後綴 `aegis`（限 amd64）。
   - `rust/aegis` 自更新模組根據 `std::env::consts::ARCH` 精確尋找對應架構資產，杜絕危險的模糊前綴匹配。
4. **安全校驗與簽名對齊**：Minisign 簽名鏈路自動覆蓋所有架構資產，`matchTrustedComment` 嚴格校驗資產名與版本。

---

## 2. 規格細節 (Specification)

### 2.1 Release 資產矩陣與命名規範
發布資產清單如下：
- `aegis-linux-amd64` 與 `aegis-linux-amd64.minisig`
- `aegis-linux-arm64` 與 `aegis-linux-arm64.minisig`
- `installer-linux-amd64` 與 `installer-linux-amd64.minisig`
- `installer-linux-arm64` 與 `installer-linux-arm64.minisig`
- 向下相容別名（由 CI 複製/連結）：
  - `aegis`（指向 `aegis-linux-amd64`）與 `aegis.minisig`
  - `installer`（指向 `installer-linux-amd64`）與 `installer.minisig`
  *目的：確保現存使用 `wget .../installer` 的一鍵指令與舊版升級腳本不中斷。*

### 2.2 編譯參數規範
- **amd64 (x86_64)**：
  - Rust target: `x86_64-unknown-linux-gnu`
  - `RUSTFLAGS="--remap-path-prefix ${{ github.workspace }}=/project -C target-cpu=generic"`
  - `CFLAGS="-march=x86-64 -mtune=generic"`
  - `CGO_ENABLED=0 GOOS=linux GOARCH=amd64`
- **arm64 (aarch64)**：
  - Rust target: `aarch64-unknown-linux-gnu`
  - 使用 `cross` 或安裝 `gcc-aarch64-linux-gnu` 進行交叉編譯。
  - `CGO_ENABLED=0 GOOS=linux GOARCH=arm64`

### 2.3 Go 安裝器 (`go/installer`)
1. **目標資產判定**：
   ```go
   func expectedAegisAssetName(arch string) string {
       switch arch {
       case "amd64":
           return "aegis-linux-amd64"
       case "arm64":
           return "aegis-linux-arm64"
       default:
           return ""
       }
   }
   ```
2. **資產選取策略**：
   - 優先尋找 `expectedAegisAssetName(arch)`。
   - 若 Release 中無該架構專用資產，且當前架構為 `amd64`，則回退尋找 `aegis`（相容舊版本 Release）。
   - 若為 `arm64` 且無 `aegis-linux-arm64`，直接報錯中斷，嚴禁回退至 x86_64 的 `aegis`。
3. **簽名驗證**：
   - Minisign 驗證的 `expectedAsset` 必須傳入實際選中的資產名（如 `aegis-linux-amd64`），確保與該資產的 `.minisig` 簽名匹配。

### 2.4 Rust 核心自更新 (`rust/aegis`)
1. **目標資產判定**：
   - 定義當前二進位架構標準資產名：
     - `x86_64` -> `aegis-linux-amd64`
     - `aarch64` -> `aegis-linux-arm64`
2. **資產選取安全重構 (`select_asset`)**：
   - 替換原有的 `starts_with` 模糊前綴匹配。
   - 邏輯：
     1. 精確匹配當前架構資產（如 `aegis-linux-amd64`）。
     2. 若未找到，且當前架構為 `x86_64`，允許回退至 `aegis`。
     3. 否則返回 `None`（明確報錯「未找到匹配當前架構的 Release 產物」）。

---

## 3. 涉及文件與模組結構 (Project Structure & Modified Files)

```text
go/installer/
├── main.go                     # Aegis 資產選擇邏輯、架構相容回退
├── main_test.go                # 資產選擇與回退邏輯單測
├── minisign_verify.go          # Minisign 校驗配合動態資產名
└── minisign_verify_test.go     # 資產名比對測試

rust/aegis/
├── src/
│   └── core/
│       └── system/
│           ├── upgrade.rs      # UpgradeManager 架構探測與 select_asset 安全選取
│           └── upgrade/        # 相關單元測試

.github/workflows/
├── public-release.yml          # matrix 多架構構建、generic baseline 鎖定、相容資產複製
└── sign-release.yml            # 自動簽名所有資產（現有邏輯已支援，覆核即可）

.forgejo/workflows/
└── public-release.yml          # Forgejo 構建工作流同步更新
```

---

## 4. 測試策略 (Testing Strategy - TDD)

1. **Go 測試**：
   - 單測 `expectedAegisAssetName` 對 `amd64` 與 `arm64` 的輸出。
   - 單測資產選擇函式：給定包含 `["aegis", "installer"]` 的舊 Release，`amd64` 正確 fallback 到 `aegis`，`arm64` 報錯拒絕。
   - 單測給定包含 `["aegis-linux-amd64", "aegis-linux-arm64"]` 的新 Release，兩者各自精確命中。
2. **Rust 測試**：
   - 單測 `select_asset`：
     - 驗證 `aarch64` 目標在同時存在 `aegis-linux-amd64` 與 `aegis-linux-arm64` 時必中 `aegis-linux-arm64`，絕不誤中 amd64。
     - 驗證舊版 Release 僅有 `aegis` 時，x86_64 回退成功，aarch64 拒絕。
3. **二進位指令集驗證**：
   - 本地驗證在加註 `CFLAGS="-march=x86-64 -mtune=generic"` 及 `RUSTFLAGS="-C target-cpu=generic"` 下編譯出的二進位，不包含任何外部未隔離的 BMI1 `andn` 指令。
4. **全量門禁保障**：
   - `go test ./...` 100% 通過。
   - `cargo test --workspace` / `cargo nextest run` 100% 通過。
   - `cargo clippy` & `cargo fmt` 無警告。

---

## 5. 邊界與約束 (Boundaries)

- **Always do (必須做)**:
  - 嚴格維持向後相容：繼續發布 `aegis` 與 `installer` 作為 `amd64` 的 alias/copy，避免既有部署指令失效。
  - 嚴禁 arm64 靜默 fallback 到 amd64 二進位。
  - 嚴格執行 TDD：先編寫測試用例證明資產選取正確性，再實裝代碼。
- **Never do (嚴禁做)**:
  - 嚴禁手改 `Cargo.toml` / `go.mod` 中的直接依賴版本（依 AGENTS.md 依賴管理規範）。
  - 單次 commit 不超過 200 行 diff。
