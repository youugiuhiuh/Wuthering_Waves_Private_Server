# SNI Tester 終極性能極限實施計劃 (Ultimate Performance)

> **狀態:** 實施中  
> **分支:** `perf/sni-tester-ultimate`  
> **工作樹目錄:** `/home/ub/Dark/wwps-perf-ultimate`  

---

## 一、優化目標與技術架構

1. **Early Termination（預握手提前終止）**：
   - 在 TLS 1.3 握手中，收到 `ServerHello` 提取出 `KeyGroup`（確認後量子 0x11EC 等）與 `ALPN` 後，立即中斷連線返回。
   - 避免傳輸龐大 X.509 證書鏈（節省 80% 網路流量與 60% 延遲）。
2. **H3 探測極速化與超時收斂**：
   - `CheckH3Support` 由 8 秒縮減至 1.5 秒，失敗立即返回，杜絕 Worker 佇列被慢網站卡死。
3. **全域引擎 (`engine.go`) 統一接入極速 DNS**：
   - `processDomain` 全面切換至 `ResolveFast`，讓常規全量探測同享 10~30ms 競速解析與快取。
4. **存量目錄批量複測並行調度**：
   - 支援大檔案與小檔案的多 Worker 全局調度，避免超大國家 `.pb` 阻塞後續小文件。

---

## 二、任務拆解 (Task Breakdown)

### Task 1: 實作 TLS 握手提前終止 (`PerformTLSPreHandshake` / Early Termination)
- **檔案路徑:** `sni_tester/pkg/tls.go`
- **設計:**
  - 透過自訂 uTLS ClientHello 與握手狀態監聽，在收到 `ServerHello` 產生 `ServerShare` 後立即終止 socket 連線並判定成功；
  - 顯著降低單次探測的時間與頻寬。

### Task 2: 收斂 `CheckH3Support` 逾時時間
- **檔案路徑:** `sni_tester/pkg/tls.go`
- **設計:**
  - `CheckH3Support` 的 HTTP Client 超時從 8s 壓縮至 1.5s；
  - 快速放行或剔除無效站點。

### Task 3: 常規引擎 (`engine.go`) 切換極速 DNS
- **檔案路徑:** `sni_tester/pkg/engine.go`
- **設計:**
  - `processDomain` 改用 `ResolveFast`；
  - 享受記憶體快取與多路 UDP 競速。

### Task 4: 測試驗證與對比基準測試
- **驗證步驟:**
  - 單元測試保證功能完整性；
  - 實機測速驗證 QPS 提升幅度；
  - 雙端質量閘門全綠。
