# SNI Tester 常規探測模式雙池持久化實施計劃

> **狀態:** 實施中  
> **分支:** `feat/engine-general-test-pq`  
> **工作樹目錄:** `/home/ub/Dark/wwps-feat-engine-pq`  

---

## 一、背景與現狀

- 現狀：目前只有 `-upgrade-dir` 與 `-upgrade-pb` 模式將測試結果保存為包含 `pq_domains` 與 `classic_domains` 的雙池結構；
- 常規探測模式（`./sni_tester -f domains.txt`）由 `engine.go` 控制，在 `jobResult` 與 `countryDomains` 中未記錄 PQ 標記，導致常規探測寫出的 `.pb` 僅包含單一 `domains` 清單。
- 目標：打通常規探測模式，使其與升級模式一致，全量自動輸出雙池 Protobuf。

---

## 二、任務拆解 (Task Breakdown)

### Task 1: 擴展 `jobResult` 記錄 `isPQ`
- **檔案路徑:** `sni_tester/pkg/engine.go`
- **設計:**
  - `jobResult` 新增 `isPQ bool`；
  - `processDomain` 中回傳 `tlsResult.IsPQ`。

### Task 2: 實作雙池批量儲存 `SaveBatchDual`
- **檔案路徑:** `sni_tester/pkg/protobuf.go`
- **設計:**
  - 支援傳入 `pqMap` 與 `classicMap`；
  - 增量讀取既有目標檔案，將新的 PQ 與 Classic 域名無損合併，調用 `WriteProtobufDualDomainFile` 寫入。

### Task 3: 改造 `engine.Run` 收集與儲存
- **檔案路徑:** `sni_tester/pkg/engine.go`
- **設計:**
  - `countryPQDomains` 與 `countryClassicDomains` 分流收集；
  - 測試完成後調用 `SaveBatchDual` 輸出雙池。

### Task 4: 測試驗證與品質閘門
- **驗證步驟:**
  - 單元測試驗證 `SaveBatchDual`；
  - `gofmt`, `go vet`, `go test` 全部通過；
  - Rust 端全套測試驗證通過。
