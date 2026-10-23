# SNI Tester: 升級為 IPinfo Lite GeoIP/ASN 數據庫實施計劃

> **狀態:** 實施中  
> **分支:** `feat/ipinfo-lite-geoip`  
> **工作樹目錄:** `/home/ub/Dark/wwps-feat-ipinfo-lite`  

---

## 一、背景與目標

- **背景:** MaxMind `GeoLite2` 數據庫對 Anycast IP（如 Cloudflare, Fastly, Google 等）存在大量誤判，往往將全球節點直接誤判為美國（US）。XTLS 官方在 `Xray-rules-dat` 中已全面遷移至 IPinfo Lite 作為第一標準。
- **目標:**
  1. `sni_tester` 支援 IPinfo Lite MMDB（單一檔案同步包含 Country 與 ASN，精準度高且每日更新）。
  2. 保持對舊版 MaxMind MMDB 的自適應向下相容，兩種格式自動偵測。
  3. 自動下載源遷移至每日自動構建的 IPinfo Lite 鏡像。

---

## 二、任務拆解 (Task Breakdown)

### Task 1: 實作雙模 GeoIP/ASN 解析器（IPinfo + MaxMind 雙相容）
- **檔案路徑:** `sni_tester/pkg/geo.go`
- **設計:**
  - 支援 `maxminddb.Reader` 的通用頂層鍵值抽取：
    - IPinfo schema: `country` / `country_code`, `asn` (格式如 "AS13335")
    - MaxMind schema: `country.iso_code`, `autonomous_system_number`
  - 新增 `LookupGeoIP(ip string) (country string, asn uint32, org string, err error)`

### Task 2: 配置與下載源升級
- **檔案路徑:** `sni_tester/pkg/config.go`
- **設計:**
  - 預設數據庫切換為 `ipinfo_lite.mmdb`；
  - 下載源更新為 `https://github.com/NetworkCats/IPinfoLite-Download/releases/latest/download/ipinfo_lite.mmdb`；
  - 支援鏡像加速。

### Task 3: 引擎與各模組適配
- **檔案路徑:** `sni_tester/pkg/engine.go`
- **設計:**
  - 引擎加載單個統一的 IPinfo DB，同時為 Country 與 ASN 提供判定服務。

### Task 4: 測試驗證與品質閘門
- **驗證步驟:**
  - 單元測試驗證 IPinfo 與 MaxMind 記錄解析正確；
  - `gofmt`, `go vet`, `go test` 全部通過；
  - Rust 端全套測試驗證不受影響。
