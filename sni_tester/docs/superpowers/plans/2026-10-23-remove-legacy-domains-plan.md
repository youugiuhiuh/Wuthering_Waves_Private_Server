# 移除冗餘 `domains` 全集字段實施計劃 (Clean Dual-Pool PB)

> **狀態:** 實施中  
> **分支:** `refactor/pb-dual-only`  
> **工作樹目錄:** `/home/ub/Dark/wwps-refactor-pb-dual-only`  

---

## 一、背景與目標

- **背景:** 舊的 `domains = 1` 是作為過渡期的全集相容字段。現在用戶準備全量重新製作 172 個國家的 `.pb` 文件，不再需要保留歷史兼容包袱。
- **目標:**
  1. `proto/sni.proto` 精簡為純粹的雙池結構：
     - `tag 1: repeated string pq_domains`
     - `tag 2: repeated string classic_domains`
  2. 移除 Go 與 Rust 兩端對 `domains` 字段的重複儲存與記憶體佔用。
  3. 產出的 `.pb` 體積進一步縮小約 40%（不再存重複的全集字串）。

---

## 二、任務拆解 (Task Breakdown)

### Task 1: Protobuf 協議精簡與 Go 代碼生成
- **檔案路徑:**
  - `proto/sni.proto`
  - `sni_tester/proto/sni.pb.go`
  - `sni_tester_app/go_engine/proto/sni.pb.go`
- **設計:**
  - `DomainList` 只包含 `pq_domains` (tag 1) 與 `classic_domains` (tag 2)。
  - 重新生成 Go proto 檔案。

### Task 2: Go 儲存與解析模組精簡
- **檔案路徑:**
  - `sni_tester/pkg/protobuf.go`
  - `sni_tester_app/go_engine/pkg/protobuf.go`
  - 各單元測試檔案
- **設計:**
  - `WriteProtobufDualDomainFile` 只序列化 `PqDomains` 與 `ClassicDomains`；
  - `ParseProtobufDomains` 回傳 `append(PqDomains, ClassicDomains...)`。

### Task 3: Rust `build.rs` 與 `SNISelector` 適配
- **檔案路徑:**
  - `rust/aegis/build.rs`
  - `rust/aegis/src/core/sni/selector.rs`
- **設計:**
  - `build.rs` 手寫結構對齊新 tag 1/2，域數為 `pq.len() + classic.len()`；
  - `selector.rs` 的 `load_protobuf_detailed` 直接讀取雙池。

### Task 4: 測試驗證與品質閘門
- **驗證步驟:**
  - Go 與 Rust 雙端全套單測通過；
  - `gofmt/vet` 與 `cargo fmt/clippy` 通過。
