# SNI Tester 性能極致優化實施計劃

> **狀態:** 待實施  
> **分支:** `perf/sni-tester-speedup`  
> **工作樹目錄:** `/home/ub/Dark/wwps-perf-sni-tester`  

---

## 一、性能瓶頸與根因分析

1. **DNS 串行 DoH 冷連線（最大元兇）**：
   `resolveWithDNS` 預設優先串行嘗試 3 個 DoH 伺服器，且每次解析都現場建立 `http.Client` 進行全新的 TCP + TLS 握手。境外 DoH 若遭遇阻斷，單個域名白白等待 5~15 秒。
2. **UDP 53 淪為次級兜底**：
   本應只需 10~30ms 的 UDP 53 被排在 DoH 與 DoT 之後。
3. **`DNSRateLimiter` 硬編碼限流瓶頸**：
   阿里/騰訊 DoH/UDP 限流被硬編碼為 15~80 QPS，導致所有並發 Worker 被鎖死排隊。
4. **死鏈超時時間過長**：
   預設 5s 超時導致無效域名佔用 Worker 佇列時間過長。

---

## 二、優化任務拆解 (Task Breakdown)

### Task 1: 實現極速 UDP 併發 DNS 解析器 (`ResolveFastUDP`)
- **檔案路徑:** `sni_tester/pkg/dns.go`
- **設計:**
  - 繞過低效的 DoH 冷連線，直接使用高速公共 UDP DNS 伺服器池（國內阿里、騰訊、字節 + 國際 Cloudflare、Google）；
  - 加入單機記憶體 DNS 緩存（`sync.Map`），相同域名 0ms 返回；
  - 超時收斂至 1.0 秒，NXDOMAIN 快速熔斷；
  - 提供 `ResolveFast(ctx, domain)` 供批量測試與升級引擎直接調用。

### Task 2: 解除批量測試中的限流死鎖
- **檔案路徑:** `sni_tester/pkg/config.go` & `sni_tester/pkg/dns.go`
- **設計:**
  - 提高全域 DNS 限制至 5,000 QPS，將突發容量 `dnsBurstSize` 調高至 500；
  - 讓 `-w 50` / `-w 100` 等並發 Worker 充分發揮多執行緒併發能力。

### Task 3: 升級引擎流水線與超時收斂
- **檔案路徑:** `sni_tester/pkg/upgrade.go` & `sni_tester/cmd/sni_tester/main.go`
- **設計:**
  - `UpgradePbFile` 切換至 `ResolveFast`；
  - TLS 握手超時預設收斂至 2.0 秒（逾期未回應的慢節點在真實代理中亦不可用）；
  - 新增 `-timeout` CLI 參數允許使用者自訂超時（預設 2s）。

### Task 4: 測試驗證與對比基準測試
- **驗證步驟:**
  - 單元測試保證 DNS 與升級邏輯正確；
  - 實機測速驗證 QPS 提升（預期提速 10~30 倍）。
