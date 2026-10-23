# SNI Tester & Aegis — 後量子 (PQ) 雙卡牌池與降級通知設計

**日期:** 2026-10-23  
**狀態:** 設計提案（待批准）  

---

## 1. 背景與目標

### 1.1 背景
現代 Web 流量中，主流瀏覽器（Chrome 124+、Edge、Firefox 132+）與頭部 CDN（Cloudflare、Google、AWS CloudFront）已全量預設啟用後量子混合密鑰交換（主要為標準化後的 **`X25519MLKEM768`** / 早期 **`X25519Kyber768Draft00`**），佔據了 65%~70% 以上的人類 Web 流量。  
然而，自建伺服器或老舊站點仍使用經典 **`X25519`**。

在目前的 `sni_tester` 與 `aegis` 實現中：
- `sni_tester` 雖然放行了 PQ 密鑰組，但輸出產物（`.pb`）僅記錄了純域名列表（`DomainList.domains`），丟失了 PQ 狀態。
- `aegis` 的 `SNISelector` 無法區分域名是否支援後量子，只能在全部域名中統一隨機抽取。
- 若客戶端支援 PQ 但抽到的偽裝 SNI 是傳統站點，握手將回退到經典 X25519，可能在流量特徵分析中留下與主流流量不一致的偏差。

### 1.2 目標
1. **PB 支援 PQ 狀態**：在保持 Protobuf 向後相容的前提下，記錄每個域名的 PQ 特徵。
2. **雙卡牌池機制（Dual Card Pool）**：
   - **主卡牌池（PQ = True）**：優先使用支援後量子的域名。
   - **手牌備用池（PQ = False）**：作為次級降級池。
3. **單機確定性隨機 + 多機獨立不確定性**：延續基於種子（`seed`）與消耗數（`used_count`）的緊湊狀態持久化（常數級幾十位元組），重啟後確定性恢復進度，不同主機之間獨立隨機不衝突。
4. **耗盡降級警報**：當主卡牌池（PQ）全部用完、切換至備用手牌池時，觸發通知告警，提醒管理員擴充/重測 SNI 池，並防止頻繁刷屏告警。
5. **存量 .pb 快速升級模式（Retest & Upgrade Mode）**：直接讀取現存 172 個國家的 `.pb` 文件，快速並行複測密鑰算法，無需重新解析原始文字與重複過濾國家/ASN，原地升級產出雙池 `.pb`。

---

## 2. 總體架構與數據流

```
┌─────────────────────────── sni_tester (Go) ───────────────────────────┐
│                                                                        │
│  1. 握手探測 (uTLS Chrome_Auto / Firefox_Auto)                          │
│     KeyGroup ∈ {X25519MLKEM768, X25519Kyber768Draft00} → is_pq = true  │
│     KeyGroup == X25519                                → is_pq = false │
│                                                                        │
│  2. 分類匯出 Protobuf (.pb)                                             │
│     ├─ domains (全集，保持向後相容)                                      │
│     ├─ pq_domains (優先主卡牌池)                                        │
│     └─ classic_domains (備用手牌池)                                     │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ 生成資源文件 (*.pb)
                                    ▼
┌───────────────────────────── aegis (Rust) ─────────────────────────────┐
│                                                                        │
│  1. SNISelector 雙池加載與排列 (單機確定性洗牌)                         │
│     排列序列: [ PQ 洗牌序列 (0..N_pq-1) ] + [ 經典洗牌序列 (N_pq..) ]   │
│                                                                        │
│  2. 抽牌指標推進 (used_count)                                           │
│     ├─ used_count < N_pq: 抽 PQ 域名 (主池正常)                        │
│     ├─ used_count == N_pq: 觸發降級事件 (PQ 耗盡) ───┐                  │
│     ├─ used_count >= N_pq: 抽 經典域名 (手牌池降級)   │                  │
│     └─ used_count == N_total: 全池耗盡，重擲 seed     │                  │
│                                                       │                │
│  3. 通知聯動 (BotAdapter) ◀───────────────────────────┘                │
│     推送 Telegram / SimpleX 警報 (本輪限通知一次，抑制告警風暴)          │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 3. 詳細模組設計與改動清單

### 3.1 Protobuf 定義（協議層）

**檔案:** `proto/sni.proto`

為保證 wire 格式嚴格向後相容，保留 `tag 1`，新增 `tag 2` 與 `tag 3`：

```protobuf
syntax = "proto3";
package sni;

option go_package = "sni_tester/proto";

// DomainList stores validated domain names with PQ classification
message DomainList {
    repeated string domains = 1;          // 全集域名（保持舊版相容）
    repeated string pq_domains = 2;       // 支援後量子密鑰算法的域名（主卡池）
    repeated string classic_domains = 3;  // 傳統 X25519 域名（備用手牌池）
}
```

*相容性保證：*
- 舊版程式讀取新生成的 `.pb` 時，只解析 `tag 1`，忽略 `tag 2/3`，無任何解析錯誤。
- 新版程式如果讀取到舊版 `.pb`（`pq_domains` 為空），則自動將 `domains` 視為經典池處理，優雅降級。

---

### 3.2 `sni_tester` 測試引擎與匯出（Go 端）

#### 3.2.1 常規模式：TLS 結果擴展
**檔案:** `sni_tester/pkg/tls.go`
- 在 `TLSResult` 中明確定義 `IsPQ bool`：
  ```go
  result.IsPQ = (result.KeyGroup == utls.X25519MLKEM768 || result.KeyGroup == utls.X25519Kyber768Draft00)
  ```

**檔案:** `sni_tester/pkg/protobuf.go` & `sni_tester_app/go_engine/pkg/protobuf.go`
- 調整 `SaveBatch` 與 `WriteProtobufDomainFile`：
  - 保存時接收包含 PQ 狀態的列表。
  - 將域名分流構建 `snipb.DomainList`：
    - `domains`: 全部去重域名（保留全集以向後相容）
    - `pq_domains`: 去重後的 PQ 域名（主卡池）
    - `classic_domains`: 去重後的經典 X25519 域名（備用手牌池）
- 重新生成 Go proto 檔案（`protoc --go_out=.`）。

---

#### 3.2.2 新增模式：存量 .pb 批量複測與就地升級（Retest/Upgrade Mode）

針對現有 `rust/aegis/src/resources/sni/` 內多達 172 個既有國別 `.pb` 檔案，若重新從海量 TXT/CSV 進行全局檢測耗時巨大。設計專用快升級模式：

**CLI 參數擴展 (`cmd/sni_tester/main.go`):**
```bash
# 批量升級整個目錄下的所有 .pb 文件
sni_tester -upgrade-dir rust/aegis/src/resources/sni -w 50

# 或指定單個國家 .pb 文件測試
sni_tester -upgrade-pb rust/aegis/src/resources/sni/US.pb -w 20

# 參數說明：
# -upgrade-dir string : 包含現有 .pb 的目錄路徑
# -upgrade-pb string  : 單個 .pb 文件路徑
# -keep-dead          : 是否保留握手失敗的舊域名（預設 false：自動剔除已失效站點）
# -out-dir string     : 升級產物輸出目錄（預設覆蓋原目錄）
```

**快速複測執行流程：**
```
┌────────────────── 存量 .pb 批量升級管線 ──────────────────┐
│                                                           │
│  1. 掃描與載入: 讀取指定目錄下的 *.pb，解碼 domains 清單  │
│                                                           │
│  2. 輕量併發握手 (跳過耗時的 Geo/ASN 下載與檢驗):          │
│     ├─ 使用 ClientHelloProfiles (帶有 ML-KEM KeyShare)     │
│     ├─ 並行 Worker 池 (固定併發 -w，防止本機 socket 耗盡)   │
│     └─ 記錄 ServerHello 選定的 KeyGroup 與 ALPN           │
│                                                           │
│  3. 判定分類:                                             │
│     ├─ 握手成功 & IsPQ  -> pq_domains                     │
│     ├─ 握手成功 & 經典   -> classic_domains                │
│     └─ 握手失敗         -> 剔除 (或 --keep-dead 留底)     │
│                                                           │
│  4. 原子寫回與進度統計:                                    │
│     寫回新結構 .pb (domains = pq + classic)               │
│     終端打印: [US.pb] 120 域名 -> 86 PQ (71.7%),          │
│                                  30 Classic, 4 失效       │
└───────────────────────────────────────────────────────────┘
```

**架構優勢：**
1. **零冗餘 I/O**：不需要重新下載數十 MB 的 GeoIP / GeoASN 庫，也不重複進行龐大的國家與自治域過濾。
2. **存量資產無損更新**：既有 172 個國家的大量合法偽裝域名在數分鐘內全部完成「後量子等級」打標與清洗。


---

### 3.3 `aegis` 卡牌池選擇器（Rust 端）

**檔案:** `rust/aegis/src/core/sni/selector.rs`

#### 雙池內存佈局與確定性洗牌
`SNISelector` 維護內部域名清單與總體排列索引：

```rust
pub struct SNISelector {
    /// 包含所有域名的集合：[0..pq_count) 為 PQ 域名，[pq_count..total) 為經典域名
    domains: Vec<String>,
    pq_count: usize,
    /// 當前輪次的打亂順序（長度為 domains.len()）
    shuffled_indices: Vec<usize>,
    used_count: u32,
    seed: u64,
    cache_key: String,
    dirty: bool,
    /// 本輪是否已發送過 PQ 耗盡告警
    pq_exhausted_notified: bool,
}
```

#### 洗牌演算法（`build_order`）
保持確定性隨機：
1. 主池 `[0 .. pq_count)` 使用 `seed` 確定性洗牌。
2. 備用池 `[pq_count .. total)` 使用衍生種子（如 `seed ^ 0x5555_5555_5555_5555`）確定性洗牌。
3. 拼接序列：`shuffled = [PQ 洗牌序列] + [經典洗牌序列]`。
4. 跳過已消耗的 `used_count` 個。

#### 狀態持久化（`SNIState`）
依然只需保存：
```json
{
  "s": 1234567890,
  "u": 42,
  "c": "2026-10-23T12:00:00Z"
}
```
- 無需增加額外持久化檔案或破壞格式，常數大小（< 128 位元組）。

#### 提取與降級事件
提供提取方法，明確反饋狀態：
```rust
#[derive(Debug, PartialEq, Eq)]
pub enum SNIExtractEvent {
    /// 正常從 PQ 池取用
    PQ,
    /// 首次降級臨界點：PQ 池剛好耗盡，首次切入經典手牌池（需觸發告警）
    DegradedToClassicFirstTime,
    /// 處於經典手牌池取用中
    Classic,
    /// 全池耗盡並重新開啟全新一輪
    RoundReset,
}

pub fn get_next_with_event(&mut self) -> (String, SNIExtractEvent) {
    // 依據 used_count 與 pq_count 判定返回事件，並在耗盡時調用 start_new_round()
}
```

---

### 3.4 告警與通知聯動（Rust 機器人層）

**檔案:** `rust/aegis/src/shared/handlers/` 或上層定期輪換邏輯
- 在節點重新配置、定時 SNI 輪換或觸發抽取時：
  - 若捕獲到 `SNIExtractEvent::DegradedToClassicFirstTime`，且 `!self.pq_exhausted_notified`：
  - 調用 `AppState::send_message` 發送國際化通知：
    ```
    ⚠️ [SNI 警告] {country} 節點的後量子 (PQ) 偽裝域名池已全部消耗，目前已自動降級使用經典 X25519 手牌池。請適時補充或重新測試 SNI 資源庫。
    ```
  - 標記 `pq_exhausted_notified = true`，直至新一輪重置。

---

## 4. 驗證與測試策略

1. **Protobuf 解碼與向後相容測試**：
   - 用舊版 `DomainList` 解碼新生成的多 tag `.pb`，斷言 `domains` 完整且無報錯。
   - 用新版 `DomainList` 解碼舊版 `.pb`（缺失 `pq_domains`），斷言自動安全歸入經典池。
2. **單機確定性測試**：
   - 給定固定 `seed` 與 `used_count`，測試兩次重建後的抽取序列完全一致。
3. **多機獨立性測試**：
   - 模擬兩台機器生成獨立隨機種子，斷言首個與後續抽出的域名序列不同。
4. **雙池邊界與降級告警測試**：
   - 構造包含 2 個 PQ 域名與 2 個經典域名的測試 selector。
   - 抽取第 1、2 次：返回 `PQ` 事件；
   - 抽取第 3 次：返回 `DegradedToClassicFirstTime` 事件；
   - 抽取第 4 次：返回 `Classic` 事件；
   - 抽取第 5 次：重置並返回新一輪。

---

## 5. 待批准決策點

1. **舊數據兼容過渡**：若現有 `.pb` 尚未重新掃描標記 PQ，是否默認視為經典域名並在首次啟動時直接發送提示，還是靜默作為全集備用？
2. **通知渠道**：PQ 耗盡告警發送至 SimpleX / Telegram 管理員目標通道的優先級設定。
