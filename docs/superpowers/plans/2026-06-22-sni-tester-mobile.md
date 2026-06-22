# SNI Tester Mobile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy sni_tester to Android phone with Web UI, USB-adb result pull, full feature parity with CLI mode.

**Architecture:** Extract main.go's core logic into reusable `pkg/` library; create thin CLI wrapper (`cmd/sni_tester`) and Web server (`cmd/sni_web`); add one-command phone deploy/pull via Makefile.

**Tech Stack:** Go (cross-compile to Android arm64), `net/http`, `embed.FS`, SSE, BadgerDB, uTLS, GeoIP2.

---

## File Structure

```
sni_tester/
├── cmd/
│   ├── sni_tester/main.go          # Thin CLI: flag → Config → NewEngine → Run
│   └── sni_web/
│       ├── main.go                 # Web server, embedded FS, listen
│       ├── handlers.go             # API handlers + SSE
│       └── static/
│           └── index.html          # Single-page Web UI (embedded via //go:embed)
├── pkg/
│   ├── config.go                  # Config struct, all constants, DNS pools, uTLS profiles, seed ASN blocklist
│   ├── types.go                   # TLSResult, DomainResult, ASNResult, ProgressEvent, Stats, Result, callbacks
│   ├── dns.go                     # DNSRateLimiter, dnsCache, resolveWithFailover, DNSConfig
│   ├── tls.go                     # TLS handshake (uTLS), validateDomain, checkH3Support, tlsCache
│   ├── geo.go                     # GeoIP country/ASN lookups, caching, isBlockedCountry, ASN blocklist
│   ├── storage.go                 # BadgerDB ops: success/failure/blocked history, ASN blocklist persistence
│   ├── protobuf.go                # DomainList marshal/unmarshal, .pb file I/O, batchSave
│   ├── protobuf_test.go           # Tests moved from main+protobuf_test.go
│   └── engine.go                  # Engine: worker pool, AIMD concurrency, progress SSE, orchestration
├── proto/sni.proto                # (unchanged)
├── go.mod / go.sum                # (unchanged deps)
├── Makefile                       # + phone-deploy, phone-pull targets
└── main.go → cmd/sni_tester/main.go  # (old main.go deleted, replaced by cmd entry)
```

---

## Plan Conventions

- All new files under `sni_tester/` belong to the `sni_tester` Go module (`go.mod` already exists).
- `pkg/` package is `package pkg`, import path `sni_tester/pkg`.
- `cmd/` packages are `package main`.
- Where a step says "extract function X from main.go", read the existing code from `sni_tester/main.go` (3077 lines) and paste into the new pkg file, adapting:
  - Change `main` package to `pkg`
  - Mark exported functions with capital first letter
  - Replace global variable references with method receivers or passed config
  - Thread-safe maps (`sync.Map`) stay as package-level vars in pkg/
- Tests should verify the extracted function works identically to the original.

---

### Task 1: Create pkg/types.go — Shared Types

**Files:**
- Create: `sni_tester/pkg/types.go`

- [ ] **Step 1: Write pkg/types.go**

Types: `DNSProvider` (iota), `DnsHealth`, `ASNResult`, `DomainResult`, `ASNInfo`, `SuccessInfo`, `BlockedInfo`, `TLSResult`, `ProgressEvent` (Type, Domain, Success, Country, IP, Info, Progress float64, Stats), `Stats` (Total, Success, Failed, Skipped, RatePerSec int), `Result` (DomainResults []DomainResult, Stats Stats), `ProgressCallback func(ProgressEvent)`, `ValidationResult = DomainResult`.

- [ ] **Step 2: Verify compilation** — `cd sni_tester && go build ./pkg/`
- [ ] **Step 3: Commit**

---

### Task 2: Create pkg/config.go — Config and Constants

**Files:**
- Create: `sni_tester/pkg/config.go`

- [ ] **Step 1: Write pkg/config.go**

Move from main.go:
- `Config` struct: `FixedWorkers`, `ForceRetry`, `ResetAll`, `TTLDays`, `MaxLines`, `Debug`, `UseBuiltinDNS`, `DNSAddr`, `GeoDBFile`, `GeoASNFile`, `BadgerDBDir`, `OutputDir`
- `DefaultConfig()` returning sensible defaults
- All DNS constants (`DNSServerTimeout`, `DNSRetryRounds`, rate limits, etc.)
- All DNS server lists (`DNSServers` as `DNSConfig{DoH, DoT, UDP []string}`)
- `DNSProviderMapUDP`, `DNSProviderMapDoH`
- `ClientHelloProfiles`, `ALPNProfiles`, `UserAgentPool`
- `SeedBlockedASNs`
- File URLs (`GeoDBFileURL`, `GeoASNFileURL`)
- BadgerDB GC constants, AIMD constants

- [ ] **Step 2: Verify compilation**
- [ ] **Step 3: Commit**

---

### Task 3: Create pkg/protobuf.go — Protobuf I/O

**Files:**
- Create: `sni_tester/pkg/protobuf.go`
- Create: `sni_tester/pkg/protobuf_test.go`

- [ ] **Step 1: Write pkg/protobuf_test.go**

Copy all test functions from `sni_tester/protobuf_test.go` (package main → package pkg), rename `writeProtobufDomainFile` → `WriteProtobufDomainFile`, `parseProtobufDomains` → `ParseProtobufDomains`.

- [ ] **Step 2: Run test — expect FAIL** — `cd sni_tester && go test ./pkg/ -v 2>&1 | grep "undefined"`
- [ ] **Step 3: Write pkg/protobuf.go**

Functions extracted from main.go (exported):
- `WriteProtobufDomainFile(domains []string, filePath string) error` — dedup, sort, marshal, write
- `ParseProtobufDomains(data []byte) ([]string, error)` — unmarshal, filter valid
- `LoadExistingBinFiles(dir string, m map[string]struct{})` — iterate .pb, parse, add to map
- `LoadExistingIntoMap(dir string, m map[string]struct{})` — iterate .txt, clean, add to map
- `CleanDomain(s string) string` — trim spaces, remove comments (after `#` or `//`), lowercase, skip empty, require `.`
- `CleanDomains(domains []string) []string` — filter+clean slice

Also need `saveBatch(targetDir string, m map[string][]string, db *badger.DB)`. This function writes .pb files per country, saves to BadgerDB success history. Keep signature identical.

- [ ] **Step 4: Run test — expect PASS** — `cd sni_tester && go test ./pkg/ -run TestWriteProtobuf -v`
- [ ] **Step 5: Delete old test** — `git rm sni_tester/protobuf_test.go`
- [ ] **Step 6: Commit**

---

### Task 4: Create pkg/storage.go — BadgerDB Operations

**Files:**
- Create: `sni_tester/pkg/storage.go`

- [ ] **Step 1: Write pkg/storage.go**

Extract from main.go (exported):
- `NewStorageManager(dbDir string) (*StorageManager, error)` — opens BadgerDB
- `(*StorageManager).Close()` — runs ValueLogGC, closes DB
- `(*StorageManager).SaveSuccess(domain, country string)` — write success entry with TTL
- `(*StorageManager).SaveFailure(domain string)` — write failure entry with TTL
- `(*StorageManager).SaveBlockedCountry(domain, code string)` — write country-blocked
- `(*StorageManager).SaveBlockedASN(domain string, asn uint32)` — write ASN-blocked
- `(*StorageManager).IsFailedRecently(domain string, now int64) bool`
- `(*StorageManager).LoadSuccessHistory() map[string]struct{}`
- `(*StorageManager).LoadBlockedHistory() map[string]struct{}`
- `(*StorageManager).LoadASNBlocklist() (sync.Map, error)` — loads persisted + seeds
- `(*StorageManager).AddASNToBlacklist(asn uint32, org, country string)`
- `(*StorageManager).ClearAll() error`
- `(*StorageManager).CleanAndCountFailure(now int64, ttlSec int64) (active, purged int)`
- `(*StorageManager).DB() *badger.DB` — for write batch in saveBatch

Internal helpers (same as main.go): `keyPrefixFailed()`, `keyPrefixSuccess()`, `keyPrefixBlockedCountry()`, `keyPrefixBlockedASN()`, `strKeyPrefixFailed()`, etc.

- [ ] **Step 2: Write a basic test** — Test open/close, save a success, load and verify
- [ ] **Step 3: Verify compilation and test** — `cd sni_tester && go test ./pkg/ -run TestStorage -v`
- [ ] **Step 4: Commit**

---

### Task 5: Create pkg/tls.go — TLS Operations

**Files:**
- Create: `sni_tester/pkg/tls.go`

- [ ] **Step 1: Write pkg/tls.go**

Extract from main.go (exported):
- `PickClientHelloID() utls.ClientHelloID`
- `PickALPNProfile() []string`
- `PickUserAgent() string`
- `PerformTLSHandshake(domain, targetIP string, tlsTimeout time.Duration, needTLS13 bool) (*TLSResult, error)`
- `ValidateDomain(result *TLSResult) (bool, string)` — check TLS 1.3 + X25519 + H2/H3
- `CheckH3Support(domain, targetIP string) bool` — HEAD request, check Alt-Svc header
- `IsValidKeyGroup(group utls.CurveID) bool`

Global state (thread-safe): `tlsCache sync.Map` (key `"domain:ip"` → `*TLSResult`), `GetCachedTLS(domain, ip string, tlsTimeout time.Duration, needTLS13 bool) *TLSResult`.

- [ ] **Step 2: Write basic test** — `PickClientHelloID` returns valid ID, `IsValidKeyGroup` checks X25519
- [ ] **Step 3: Verify compilation and test**
- [ ] **Step 4: Commit**

---

### Task 6: Create pkg/dns.go — DNS Resolution

**Files:**
- Create: `sni_tester/pkg/dns.go`

- [ ] **Step 1: Write pkg/dns.go`

Extract from main.go (exported):
- `NewDNSRateLimiter() *DNSRateLimiter` — init global + provider limiters
- `(*DNSRateLimiter).Acquire(ctx, server string, isDoHOrDoT bool) (release func(), error)`
- `(*DNSRateLimiter).TryAcquire() bool`
- `ResolveWithFailover(ctx context.Context, domain string) ([]string, error)` — multi-round DNS with pool
- `ShuffleStrings(s []string)` — randomize DNS pool order

Global state: `dnsCache sync.Map` (key domain → IP string), `dnsPrefetchCache sync.Map`, `dnsPrefetchQueue chan string`.

- [ ] **Step 2: Write basic test** — DNS resolver struct compile check
- [ ] **Step 3: Verify compilation**
- [ ] **Step 4: Commit**

---

### Task 7: Create pkg/geo.go — GeoIP Lookups

**Files:**
- Create: `sni_tester/pkg/geo.go`

- [ ] **Step 1: Write pkg/geo.go**

Extract from main.go (exported):
- `OpenGeoDBs(geoFile, asnFile string) (geo *geoip2.Reader, asn *geoip2.Reader, err error)`
- `GetCountry(ip string, db *geoip2.Reader) string` — with cache (`countryCache sync.Map`)
- `GetASN(ip string, db *geoip2.Reader) (uint32, string)` — with cache (`asnResultCache sync.Map`)
- `IsBlockedCountry(code string) bool` — CN/HK/MO/IR/RU/KP
- `PrepareGeoDBs(geoFile, asnFile string) error` — download if missing

- [ ] **Step 2: Write test** — `IsBlockedCountry`: CN→true, US→false
- [ ] **Step 3: Verify compilation**
- [ ] **Step 4: Commit**

---

### Task 8: Create pkg/engine.go — Core Engine

**Files:**
- Create: `sni_tester/pkg/engine.go`

- [ ] **Step 1: Write pkg/engine.go: Engine struct + constructor**

```go
type Engine struct {
    cfg     Config
    dns     *DNSRateLimiter
    geoDB   *geoip2.Reader
    asnDB   *geoip2.Reader
    storage *StorageManager
}

func NewEngine(cfg Config) (*Engine, error) {
    // Open GeoIP DBs
    // Open BadgerDB via StorageManager
    // Init DNS rate limiter
    // Return Engine
}

func (e *Engine) Close() {
    e.geoDB.Close()
    e.asnDB.Close()
    e.storage.Close()
}
```

- [ ] **Step 2: Write pkg/engine.go: Run method**

```go
type jobResult struct {
    domain  string
    success bool
    ip      string
    country string
    asn     uint32
    org     string
    info    string
}

func (e *Engine) Run(ctx context.Context, domains []string, cb ProgressCallback) (*Result, error) {
    // 1. Load success history, failure history, blocked history into a skip map
    // 2. Create job channel, results channel
    // 3. Spawn workers (AIMD or fixed count)
    // 4. Feed domains to job channel
    // 5. Collect results from result channel
    // 6. Aggregate by country
    // 7. Call saveBatch(.pb output + BadgerDB)
    // 8. Return Result with stats
}
```

Worker loop (goroutine):
1. Receive domain from jobs channel
2. Check skip map (already succeeded/failed/blocked)
3. DNS resolve (`ResolveWithFailover`)
4. GeoIP country check (`GetCountry` + `IsBlockedCountry`) → skip if blocked
5. GeoIP ASN check → skip if blocked ASN
6. TLS handshake (`PerformTLSHandshake` + `ValidateDomain`)
7. Send result to results channel
8. Emit progress event via callback

AIMD concurrency: same logic as main.go lines ~1300-1420 (`ticker` adjusting `currentWorkers` via `workerSemaphore`). Extract as-is, using `Config.FixedWorkers` to disable.

- [ ] **Step 3: Write test** — Test that `NewEngine` with temp dirs returns no error; test `Run` with a small domain list (Mocked DNS/TLS: the test should just verify the worker pool starts and progress callbacks fire). For actual TLS/DNS tests, rely on integration testing.

- [ ] **Step 4: Verify compilation and test**
- [ ] **Step 5: Commit**

---

### Task 9: Refactor main.go → cmd/sni_tester/main.go

**Files:**
- Create: `sni_tester/cmd/sni_tester/main.go`
- Delete: `sni_tester/main.go`

- [ ] **Step 1: Write cmd/sni_tester/main.go**

```go
package main

import (
    "bufio"
    "context"
    "flag"
    "fmt"
    "os"
    "os/signal"
    "path/filepath"
    "strings"
    "syscall"

    "github.com/schollz/progressbar/v3"
    "sni_tester/pkg"
)

func main() {
    inputFile := flag.String("f", "", "Input TXT/CSV file containing SNIs")
    debugMode := flag.Bool("debug", false, "Enable debug logging")
    dnsAddr := flag.String("dns", "", "DNS server address")
    ttlDays := flag.Int("ttl", 7, "Days to remember failures")
    maxLines := flag.Int("max", 0, "Max lines to read")
    fixedWorkers := flag.Int("w", 0, "Fixed worker count")
    autoShutdown := flag.Bool("shutdown", false, "Shutdown after completion")
    forceRetry := flag.Bool("force", false, "Re-test skipped domains")
    resetAll := flag.Bool("reset", false, "Clear all history")
    proxyString := flag.String("p", "", "Proxy for Geo download")
    flag.Parse()

    if *inputFile == "" {
        flag.Usage()
        os.Exit(1)
    }

    // Find target directory (same as original findTargetDir)
    targetDir := findTargetDir()
    if targetDir == "" {
        fmt.Println("Error: Could not find rust/aegis/src/resources/sni directory.")
        os.Exit(1)
    }

    cfg := pkg.DefaultConfig()
    cfg.FixedWorkers = *fixedWorkers
    cfg.ForceRetry = *forceRetry
    cfg.ResetAll = *resetAll
    cfg.TTLDays = *ttlDays
    cfg.MaxLines = *maxLines
    cfg.Debug = *debugMode
    cfg.UseBuiltinDNS = *dnsAddr == ""
    cfg.DNSAddr = *dnsAddr
    cfg.OutputDir = targetDir

    // Prepare GeoIP DBs
    pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, *proxyString)

    engine, err := pkg.NewEngine(cfg)
    if err != nil {
        fmt.Printf("Error initializing engine: %v\n", err)
        os.Exit(1)
    }
    defer engine.Close()

    // Signal handling
    ctx, cancel := context.WithCancel(context.Background())
    sigChan := make(chan os.Signal, 1)
    signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
    go func() {
        <-sigChan
        cancel()
    }()

    // Read domains
    domains := readDomains(*inputFile, cfg.MaxLines)

    // Progress bar
    bar := progressbar.NewOptions(len(domains),
        progressbar.OptionSetDescription("Testing SNIs"),
        progressbar.OptionShowCount(),
        progressbar.OptionShowIts(),
        progressbar.OptionSetItsString("domains"),
        progressbar.OptionFullWidth(),
    )

    cb := func(event pkg.ProgressEvent) {
        bar.Add(1)
        if event.Type == "validated" && event.Success {
            // Could log successes
        }
    }

    result, err := engine.Run(ctx, domains, cb)
    if err != nil {
        fmt.Printf("Engine error: %v\n", err)
    }

    fmt.Printf("\nDone. %d succeeded, %d failed, %d skipped\n",
        result.Stats.Success, result.Stats.Failed, result.Stats.Skipped)

    if *autoShutdown {
        // trigger shutdown command
    }
}

// findTargetDir walks up from CWD looking for rust/aegis/src/resources/sni
func findTargetDir() string {
    cwd, _ := os.Getwd()
    dir := cwd
    for {
        target := filepath.Join(dir, "rust", "aegis", "src", "resources", "sni")
        if info, err := os.Stat(target); err == nil && info.IsDir() {
            return target
        }
        parent := filepath.Dir(dir)
        if parent == dir {
            break
        }
        dir = parent
    }
    return ""
}

func readDomains(path string, maxLines int) []string {
    file, err := os.Open(path)
    if err != nil {
        fmt.Printf("Error opening file: %v\n", err)
        os.Exit(1)
    }
    defer file.Close()

    var domains []string
    sc := bufio.NewScanner(file)
    for sc.Scan() {
        d := pkg.CleanDomain(sc.Text())
        if d != "" {
            domains = append(domains, d)
            if maxLines > 0 && len(domains) >= maxLines {
                break
            }
        }
    }
    return domains
}
```

- [ ] **Step 2: Remove old main.go**

```bash
git rm sni_tester/main.go
```

- [ ] **Step 3: Build and verify**

```bash
cd sni_tester && go build ./cmd/sni_tester/
```
Expected: compiles to `sni_tester/sni_tester`.

- [ ] **Step 4: Run existing tests**

```bash
cd sni_tester && go test ./pkg/ -v
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

---

### Task 10: Create cmd/sni_web — Web Server

**Files:**
- Create: `sni_tester/cmd/sni_web/main.go`
- Create: `sni_tester/cmd/sni_web/handlers.go`
- Create: `sni_tester/cmd/sni_web/static/index.html`

- [ ] **Step 1: Write cmd/sni_web/static/index.html**

Single HTML file with embedded CSS/JS. Layout:
- File upload area (drag/drop or click to select TXT/CSV)
- Parameter form: workers (number input), DNS (text input), TTL, force/reset checkboxes
- Start/Stop buttons
- Progress: progress bar, stats (total/done/success/fail/skip, speed), live result table (last 50 results)
- Download button (visible after completion)
- SSE connection to `/api/progress`

Key JS:
```js
const evtSource = new EventSource('/api/progress');
evtSource.onmessage = (e) => {
    const data = JSON.parse(e.data);
    // update progress bar, stats, table
};
```

- [ ] **Step 2: Write cmd/sni_web/main.go**

```go
package main

import (
    "embed"
    "io/fs"
    "log"
    "net/http"
    "os"
    "sni_tester/pkg"
)

//go:embed static/*
var staticFiles embed.FS

func main() {
    outputDir := "/data/local/tmp/sni_output"
    if d := os.Getenv("SNI_OUTPUT_DIR"); d != "" {
        outputDir = d
    }

    cfg := pkg.DefaultConfig()
    cfg.OutputDir = outputDir
    cfg.Debug = true // skip network isolation on phone

    engine, err := pkg.NewEngine(cfg)
    if err != nil {
        log.Fatalf("Failed to init engine: %v", err)
    }
    defer engine.Close()

    srv := &Server{
        engine:      engine,
        cfg:         cfg,
        subscribers: make(map[chan pkg.ProgressEvent]struct{}),
    }

    mux := http.NewServeMux()
    mux.HandleFunc("GET /", func(w http.ResponseWriter, r *http.Request) {
        sub, _ := fs.Sub(staticFiles, "static")
        http.FileServer(http.FS(sub)).ServeHTTP(w, r)
    })
    mux.HandleFunc("GET /api/progress", srv.handleSSE)
    mux.HandleFunc("POST /api/start", srv.handleStart)
    mux.HandleFunc("POST /api/stop", srv.handleStop)
    mux.HandleFunc("GET /api/status", srv.handleStatus)
    mux.HandleFunc("GET /api/download", srv.handleDownload)
    mux.HandleFunc("POST /api/upload", srv.handleUpload)

    log.Println("SNI Web UI: http://localhost:8080")
    log.Fatal(http.ListenAndServe(":8080", mux))
}
```

Routes:
- `GET /` → serve embedded static/index.html
- `GET /api/progress` → SSE stream
- `POST /api/start` → parse JSON body → `engine.Run(ctx, domains, cb)` in goroutine
- `POST /api/stop` → cancel context
- `GET /api/status` → idle/running + current stats
- `GET /api/download` → zip of output .pb files
- `POST /api/upload` → accept multipart file upload → store in temp dir

- [ ] **Step 3: Write cmd/sni_web/handlers.go — full API**

```go
package main

import (
    "archive/zip"
    "bufio"
    "bytes"
    "context"
    "encoding/json"
    "fmt"
    "io"
    "log"
    "net/http"
    "os"
    "path/filepath"
    "strings"
    "sync"
    "sni_tester/pkg"
)

type Server struct {
    engine      *pkg.Engine
    cfg         pkg.Config
    cancel      func()
    mu          sync.Mutex
    running     bool
    subscribers map[chan pkg.ProgressEvent]struct{}
    results     *pkg.Result
    inputText   string
}

func (s *Server) handleUpload(w http.ResponseWriter, r *http.Request) {
    // multipart file upload → store in s.inputText (as domain list bytes)
    r.ParseMultipartForm(10 << 20)
    file, _, err := r.FormFile("file")
    if err != nil {
        http.Error(w, err.Error(), 400)
        return
    }
    defer file.Close()
    buf := new(bytes.Buffer)
    io.Copy(buf, file)
    s.inputText = buf.String()
    json.NewEncoder(w).Encode(map[string]string{"status": "ok", "domains": s.inputText})
}

func (s *Server) handleStart(w http.ResponseWriter, r *http.Request) {
    s.mu.Lock()
    if s.running {
        s.mu.Unlock()
        http.Error(w, "already running", 409)
        return
    }
    s.running = true
    s.mu.Unlock()

    // Parse request body for parameter overrides
    var params struct {
        Workers int    `json:"workers"`
        DNS     string `json:"dns"`
        TTL     int    `json:"ttl"`
        Force   bool   `json:"force"`
        Reset   bool   `json:"reset"`
        Debug   bool   `json:"debug"`
    }
    json.NewDecoder(r.Body).Decode(&params)

    cfg := s.cfg
    if params.Workers > 0 { cfg.FixedWorkers = params.Workers }
    if params.DNS != "" { cfg.DNSAddr = params.DNS; cfg.UseBuiltinDNS = false }
    if params.TTL > 0 { cfg.TTLDays = params.TTL }
    cfg.ForceRetry = params.Force
    cfg.ResetAll = params.Reset
    cfg.Debug = params.Debug

    ctx, cancel := context.WithCancel(context.Background())
    s.cancel = cancel

    // Parse domains from inputText
    var domains []string
    sc := bufio.NewScanner(strings.NewReader(s.inputText))
    for sc.Scan() {
        d := pkg.CleanDomain(sc.Text())
        if d != "" {
            domains = append(domains, d)
        }
    }

    go func() {
        result, err := s.engine.Run(ctx, domains, func(ev pkg.ProgressEvent) {
            s.broadcast(ev)
        })
        s.mu.Lock()
        s.running = false
        s.results = result
        s.mu.Unlock()
        s.broadcast(pkg.ProgressEvent{Type: "done", Progress: 1.0})
        if err != nil {
            log.Printf("Engine error: %v", err)
        }
    }()

    json.NewEncoder(w).Encode(map[string]string{"status": "started"})
}

func (s *Server) handleStop(w http.ResponseWriter, r *http.Request) {
    s.mu.Lock()
    if s.cancel != nil {
        s.cancel()
    }
    s.running = false
    s.mu.Unlock()
    json.NewEncoder(w).Encode(map[string]string{"status": "stopped"})
}

func (s *Server) handleStatus(w http.ResponseWriter, r *http.Request) {
    s.mu.Lock()
    running := s.running
    results := s.results
    s.mu.Unlock()

    resp := map[string]interface{}{"running": running}
    if results != nil {
        resp["stats"] = results.Stats
    }
    json.NewEncoder(w).Encode(resp)
}

func (s *Server) handleDownload(w http.ResponseWriter, r *http.Request) {
    // Zip all .pb files from output directory
    w.Header().Set("Content-Type", "application/zip")
    w.Header().Set("Content-Disposition", "attachment; filename=sni_results.zip")

    zw := zip.NewWriter(w)
    filepath.Walk(s.cfg.OutputDir, func(path string, info os.FileInfo, err error) error {
        if err != nil || info.IsDir() || !strings.HasSuffix(info.Name(), ".pb") {
            return nil
        }
        f, _ := os.ReadFile(path)
        zw.WriteHeader(&zip.FileHeader{Name: info.Name()})
        zw.Write(f)
        return nil
    })
    zw.Close()
}

func (s *Server) broadcast(event pkg.ProgressEvent) {
    s.mu.Lock()
    defer s.mu.Unlock()
    for ch := range s.subscribers {
        select {
        case ch <- event:
        default:
        }
    }
}

func (s *Server) handleSSE(w http.ResponseWriter, r *http.Request) {
    flusher, ok := w.(http.Flusher)
    if !ok {
        http.Error(w, "Streaming unsupported", http.StatusInternalServerError)
        return
    }
    w.Header().Set("Content-Type", "text/event-stream")
    w.Header().Set("Cache-Control", "no-cache")
    w.Header().Set("Connection", "keep-alive")
    ch := make(chan pkg.ProgressEvent, 100)
    s.mu.Lock()
    s.subscribers[ch] = struct{}{}
    s.mu.Unlock()

    notify := r.Context().Done()
    for {
        select {
        case <-notify:
            s.mu.Lock()
            delete(s.subscribers, ch)
            s.mu.Unlock()
            return
        case event := <-ch:
            data, _ := json.Marshal(event)
            fmt.Fprintf(w, "data: %s\n\n", data)
            flusher.Flush()
        }
    }
}
```

- [ ] **Step 4: Verify compilation**

```bash
cd sni_tester && go build ./cmd/sni_web/
```
Expected: compiles to `sni_tester/sni_web`.

- [ ] **Step 5: Commit**

---

### Task 11: Add Makefile — Phone Deploy/Pull Targets

**Files:**
- Create: `sni_tester/Makefile`

- [ ] **Step 1: Write Makefile**

```makefile
BINARY=sni_web
OUTPUT_DIR=sni_output

.PHONY: phone-deploy phone-pull clean

phone-deploy: $(BINARY)
	adb push $(BINARY) /data/local/tmp/
	adb push GeoLite2-Country.mmdb /data/local/tmp/ 2>/dev/null || true
	adb push GeoLite2-ASN.mmdb /data/local/tmp/ 2>/dev/null || true
	adb shell "mkdir -p /data/local/tmp/$(OUTPUT_DIR)"
	adb shell "cd /data/local/tmp && SNI_OUTPUT_DIR=/data/local/tmp/$(OUTPUT_DIR) chmod +x $(BINARY) && ./$(BINARY)"
	adb forward tcp:8080 tcp:8080
	@echo "Open http://localhost:8080 in your browser"

$(BINARY):
	GOOS=android GOARCH=arm64 CGO_ENABLED=0 go build -o $(BINARY) ./cmd/sni_web/

phone-pull:
	mkdir -p ../rust/aegis/src/resources/sni/
	adb pull /data/local/tmp/$(OUTPUT_DIR)/. ../rust/aegis/src/resources/sni/
	@echo "Results pulled. Run 'cargo build' in rust/aegis to embed."

clean:
	rm -f $(BINARY)
	rm -rf $(OUTPUT_DIR) badger_db
```

Note: the phone-deploy uses `-output` flag (added to sni_web for specifying output directory).

- [ ] **Step 2: Verify Makefile syntax**

```bash
cd sni_tester && make -n phone-deploy
```
Expected: prints commands without executing.

- [ ] **Step 3: Commit**

---

### Task 12: Integration Verification

**Files:** none

- [ ] **Step 1: Build all**

```bash
cd sni_tester
go build ./cmd/sni_tester/
go build ./cmd/sni_web/
go test ./pkg/ -v
```

Expected: all builds + tests pass.

- [ ] **Step 2: Check CLI works (syntax only, no actual domains)**

```bash
cd sni_tester && ./sni_tester 2>&1 | head -3
```
Expected: usage message.

- [ ] **Step 3: Verify old main.go is completely replaced**

```bash
cd sni_tester && go vet ./...
```
Expected: no errors.

- [ ] **Step 4: Final commit of all changes**

```bash
git add sni_tester/
git rm sni_tester/main.go sni_tester/protobuf_test.go
git commit -m "feat(sni_tester): refactor to pkg/ + add Web UI + phone deploy"
```
