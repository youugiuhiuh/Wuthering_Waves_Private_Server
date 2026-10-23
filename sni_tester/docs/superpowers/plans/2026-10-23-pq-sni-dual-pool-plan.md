# SNI Tester & Aegis: 後量子 (PQ) 雙卡牌池與批量複測實施計劃

> **狀態:** 待批准  
> **分支:** `feat/pq-sni-dual-pool`  
> **工作樹目錄:** `/home/ub/Dark/wwps-pq-sni-dual-pool`  
> **設計文檔:** `sni_tester/docs/superpowers/specs/2026-10-23-pq-sni-dual-pool-design.md`  

---

## 任務清單 (Task Breakdown)

### Task 1: Protobuf 協議升級（向後相容）
- **檔案路徑:**
  - `proto/sni.proto`
  - `sni_tester/proto/sni.pb.go`
- **變更說明:**
  - 在 `proto/sni.proto` 中擴展 `DomainList`，保留 `tag 1`，新增 `tag 2: repeated string pq_domains` 與 `tag 3: repeated string classic_domains`。
  - 重新編譯生成 `sni_tester/proto/sni.pb.go`。
- **驗證步驟:**
  - 運行 `go test ./...` 驗證 Go 端舊測試通過。

---

### Task 2: Go 端 `sni_tester` 核心邏輯升級與存量 .pb 複測模式
- **檔案路徑:**
  - `sni_tester/pkg/types.go`
  - `sni_tester/pkg/tls.go`
  - `sni_tester/pkg/protobuf.go`
  - `sni_tester/pkg/upgrade.go` (新增)
  - `sni_tester/pkg/upgrade_test.go` (新增測試)
  - `sni_tester/cmd/sni_tester/main.go`
- **變更說明:**
  - `TLSResult` 記錄 `IsPQ bool`。
  - `WriteProtobufDomainFile` 支援雙池與全集並行寫入。
  - 實現 `UpgradePb` / `UpgradeDir` 模式：直接讀取已有的 `.pb`，輕量發起握手測試密鑰算法，原地原子升級寫回雙池 `.pb`，支援 `-w` 併發與 `-keep-dead` 選項。
- **驗證步驟:**
  - 執行 `cd sni_tester && go test -v ./pkg`，確認升級與解析單測全過。

---

### Task 3: Rust 端 `build.rs` 與 `DomainList` 解碼適配
- **檔案路徑:**
  - `rust/aegis/build.rs`
  - `rust/aegis/src/core/sni/selector.rs`
- **變更說明:**
  - 確保 `build.rs` 中掃描 `.pb` 最大檔案時，正確相容新多字段 `DomainList`（`prost` 屬性標記）。
  - 在 `selector.rs` 的 protobuf 解碼中提取 `pq_domains` 與 `classic_domains`。
- **驗證步驟:**
  - `cargo check --package aegis` 無編譯警告與錯誤。

---

### Task 4: Rust 端 `SNISelector` 雙池確定性洗牌與單機確定性恢復
- **檔案路徑:**
  - `rust/aegis/src/core/sni/selector.rs`
- **變更說明:**
  - `SNISelector` 支持內存分段：`[PQ 洗牌序列] + [經典洗牌序列]`。
  - 支援 `get_next_with_event()`，在 `used_count == pq_count` 臨界點返回 `DegradedToClassicFirstTime`。
  - 保持基於 `seed + used_count` 的常數級狀態持久化，重啟 100% 確定性恢復。
  - 新增單測覆蓋：雙池邊界、降級臨界事件、單機確定性、多機獨立性。
- **驗證步驟:**
  - 執行 `cargo nextest run core::sni`，確認所有單測全部 PASS。

---

### Task 5: 國際化多語言與降級告警防風暴
- **檔案路徑:**
  - `rust/aegis/src/resources/i18n/zh.yml`
  - `rust/aegis/src/resources/i18n/en.yml`
  - `rust/aegis/src/resources/i18n/ja.yml`
  - `rust/aegis/src/core/sni/selector.rs`
- **變更說明:**
  - 新增 `sni.alert_pq_exhausted` 翻譯文本。
  - `SNISelector` 內部記錄 `pq_exhausted_notified: bool`，每輪週期內僅發送一次告警，重置新輪次時重置標誌。
- **驗證步驟:**
  - `cargo nextest run` 驗證 i18n 檢查單測通過。

---

### Task 6: 整合驗證與程式碼品質閘門 (Quality Gate)
- **變更說明:**
  - 執行 `cargo fmt --check`。
  - 執行 `cargo clippy --all-targets -- -D warnings`。
  - 執行 Go 端代碼格式化與靜態檢查。
  - 執行 `cargo nextest run core::sni` 與 `go test ./...` 確保雙端全綠。
