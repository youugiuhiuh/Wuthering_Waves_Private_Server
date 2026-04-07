# DNS Rate Limiter Smart Scheduling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement smart DNS rate limiting with timeout isolation to prevent `context deadline exceeded` errors during large-scale DNS queries.

**Architecture:** Three-layer rate limiting (concurrency control, global rate limit, provider rate limit) using `Reserve()` + `Delay()` pattern instead of blocking `Wait(ctx)`. Prefetch uses non-blocking acquisition, main tests use timeout-isolated waiting.

**Tech Stack:** Go, `golang.org/x/time/rate`, `sync/atomic`

---

## File Structure

| File | Purpose |
|------|---------|
| `sni_tester/main.go` | All changes in single file: types, constants, DNSRateLimiter methods, caller sites |

---

## Task 1: Add New Types and Constants

**Files:**
- Modify: `sni_tester/main.go:50-115`

### Step 1.1: Add wait configuration constants

Add after line 60 (after existing DNS rate limiter constants):

```go
// DNS Rate Limiter Wait Configuration
const (
    dnsBacklogThreshold         = 30                      // Backlog threshold for high-load mode
    dnsNormalSemaphoreWait      = 50 * time.Millisecond   // Normal: semaphore wait
    dnsNormalRateWait           = 50 * time.Millisecond   // Normal: rate limit wait
    dnsHighLoadSemaphoreWait    = 20 * time.Millisecond   // High-load: semaphore wait
    dnsHighLoadRateWait         = 10 * time.Millisecond   // High-load: rate limit wait
)
```

### Step 1.2: Add priority type and error variables

Add after line 89 (after `ProviderGlobal DNSProvider = iota`):

```go
// DNS Priority Type
type DNSPriority int

const (
    PriorityPrefetch DNSPriority = iota // Prefetch: non-blocking, skip on rate limit
    PriorityNormal                      // Normal: wait with timeout isolation
)

// DNS Rate Limiter Errors
var (
    ErrRateLimited      = errors.New("DNS rate limited")
    ErrConcurrencyLimit = errors.New("DNS concurrency limit reached")
)

// MaxWaitConfig holds wait limits for different states
type MaxWaitConfig struct {
    Semaphore time.Duration
    Rate      time.Duration
}
```

- [ ] **Step 1.3: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success (no errors)

- [ ] **Step 1.4: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): add types and constants for smart rate limiting"
```

---

## Task 2: Update DNSRateLimiter Structure

**Files:**
- Modify: `sni_tester/main.go:149-156`

### Step 2.1: Add backlog field to DNSRateLimiter

Modify the struct at line 149-156:

```go
type DNSRateLimiter struct {
    globalLimiter    *rate.Limiter
    providerLimiters map[DNSProvider]*rate.Limiter
    semaphore        chan struct{}
    backlog          atomic.Int32 // Current backlog count
    providerMapUDP   map[string]DNSProvider
    providerMapDoH   map[string]DNSProvider
}
```

- [ ] **Step 2.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 2.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): add backlog field to DNSRateLimiter"
```

---

## Task 3: Implement tryAcquireForPrefetch Method

**Files:**
- Modify: `sni_tester/main.go:232-244` (replace TryAcquire)

### Step 3.1: Write TryAcquireForPrefetch method

Replace the existing `TryAcquire` method (lines 232-244) with:

```go
// TryAcquireForPrefetch attempts to acquire without blocking
// Returns release function and whether acquisition succeeded
func (r *DNSRateLimiter) TryAcquireForPrefetch() (release func(), acquired bool) {
    // 1. Check backlog - skip if overloaded
    if r.backlog.Load() > dnsBacklogThreshold {
        return nil, false
    }

    // 2. Try to acquire semaphore (non-blocking)
    select {
    case r.semaphore <- struct{}{}:
        // Got concurrency slot
    default:
        return nil, false
    }

    // 3. Check global rate limit (non-blocking)
    if !r.globalLimiter.Allow() {
        <-r.semaphore
        return nil, false
    }

    return func() { <-r.semaphore }, true
}
```

- [ ] **Step 3.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 3.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): implement TryAcquireForPrefetch for non-blocking prefetch"
```

---

## Task 4: Implement calculateMaxWait Method

**Files:**
- Modify: `sni_tester/main.go` (add after TryAcquireForPrefetch)

### Step 4.1: Write calculateMaxWait method

Add after `TryAcquireForPrefetch`:

```go
// calculateMaxWait returns dynamic wait limits based on backlog
func (r *DNSRateLimiter) calculateMaxWait() MaxWaitConfig {
    backlog := r.backlog.Load()

    if backlog > dnsBacklogThreshold {
        // High load: reduce wait times
        return MaxWaitConfig{
            Semaphore: dnsHighLoadSemaphoreWait,
            Rate:      dnsHighLoadRateWait,
        }
    }

    // Normal load
    return MaxWaitConfig{
        Semaphore: dnsNormalSemaphoreWait,
        Rate:      dnsNormalRateWait,
    }
}
```

- [ ] **Step 4.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 4.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): implement calculateMaxWait for adaptive wait times"
```

---

## Task 5: Implement acquireWithRateLimit Helper

**Files:**
- Modify: `sni_tester/main.go` (add after calculateMaxWait)

### Step 5.1: Write acquireWithRateLimit method

Add after `calculateMaxWait`:

```go
// acquireWithRateLimit acquires rate limit with timeout isolation
// Uses Reserve() instead of Wait() to avoid consuming caller's context deadline
func (r *DNSRateLimiter) acquireWithRateLimit(ctx context.Context, server string, isDoHOrDoT bool, maxWait time.Duration) (func(), error) {
    // Use Reserve() instead of Wait() to avoid consuming ctx deadline
    globalReservation := r.globalLimiter.Reserve()
    globalDelay := globalReservation.Delay()

    if globalDelay > maxWait {
        globalReservation.Cancel()
        return nil, ErrRateLimited
    }

    // Wait using independent context, but respect cancellation
    if globalDelay > 0 {
        select {
        case <-time.After(globalDelay):
            // Wait complete
        case <-ctx.Done():
            globalReservation.Cancel()
            return nil, ctx.Err()
        }
    }

    // Provider rate limit (similar pattern)
    provider := r.getProvider(server, isDoHOrDoT)
    if limiter, ok := r.providerLimiters[provider]; ok {
        providerReservation := limiter.Reserve()
        providerDelay := providerReservation.Delay()

        if providerDelay > maxWait {
            providerReservation.Cancel()
            globalReservation.Cancel()
            return nil, ErrRateLimited
        }

        if providerDelay > 0 {
            select {
            case <-time.After(providerDelay):
                // Wait complete
            case <-ctx.Done():
                providerReservation.Cancel()
                globalReservation.Cancel()
                return nil, ctx.Err()
            }
        }
    }

    return func() {
        <-r.semaphore
    }, nil
}
```

- [ ] **Step 5.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 5.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): implement acquireWithRateLimit with timeout isolation"
```

---

## Task 6: Rewrite Acquire Method

**Files:**
- Modify: `sni_tester/main.go:175-200`

### Step 6.1: Replace Acquire method

Replace the existing `Acquire` method (lines 175-200) with:

```go
// Acquire acquires permission for DNS query with timeout isolation
// Uses Reserve() + Delay() to avoid consuming caller's context deadline
func (r *DNSRateLimiter) Acquire(ctx context.Context, server string, isDoHOrDoT bool) (func(), error) {
    // 1. Calculate dynamic max wait based on backlog
    maxWait := r.calculateMaxWait()

    // 2. Acquire semaphore (with cancellation support)
    select {
    case r.semaphore <- struct{}{}:
        // Got concurrency slot
    case <-ctx.Done():
        return nil, ctx.Err()
    case <-time.After(maxWait.Semaphore):
        return nil, ErrConcurrencyLimit
    }

    // 3. Acquire rate limit with timeout isolation
    release, err := r.acquireWithRateLimit(ctx, server, isDoHOrDoT, maxWait.Rate)
    if err != nil {
        <-r.semaphore
        return nil, err
    }

    return release, nil
}
```

- [ ] **Step 6.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 6.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): rewrite Acquire with timeout isolation"
```

---

## Task 7: Update Prefetch Worker

**Files:**
- Modify: `sni_tester/main.go:1062-1073`

### Step 7.1: Update prefetch worker to use TryAcquireForPrefetch

Find the prefetch worker loop (around line 1062-1073) and replace:

**Before:**
```go
// Try to acquire rate limiter (non-blocking)
if !dnsRateLimiter.TryAcquire() {
    // Rate limited, skip this prefetch
    continue
}
ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
ips, err := resolveWithFailover(ctx, domain)
cancel()
if err == nil && len(ips) > 0 {
    dnsPrefetchCache.Store(domain, ips[0])
}
```

**After:**
```go
// Try to acquire rate limiter (non-blocking)
release, acquired := dnsRateLimiter.TryAcquireForPrefetch()
if !acquired {
    // Rate limited or backlog, skip this prefetch
    continue
}
ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
ips, err := resolveWithFailover(ctx, domain)
cancel()
release()
if err == nil && len(ips) > 0 {
    dnsPrefetchCache.Store(domain, ips[0])
}
```

- [ ] **Step 7.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 7.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): update prefetch worker to use TryAcquireForPrefetch"
```

---

## Task 8: Update resolveWithUDP Error Handling

**Files:**
- Modify: `sni_tester/main.go:2387-2391`

### Step 8.1: Update error handling in resolveWithUDP

Find the DNS rate limiter acquisition in `resolveWithUDP` (around line 2387-2391) and update:

**Before:**
```go
release, err := dnsRateLimiter.Acquire(ctx, server, false) // UDP, not DoH/DoT
if err != nil {
    lastErr = err
    continue
}
```

**After:**
```go
release, err := dnsRateLimiter.Acquire(ctx, server, false)
if errors.Is(err, ErrRateLimited) || errors.Is(err, ErrConcurrencyLimit) {
    // Rate limited, brief backoff then try next server
    time.Sleep(50 * time.Millisecond)
    lastErr = err
    continue
}
if err != nil {
    // Other error (context cancelled, etc.)
    lastErr = err
    continue
}
```

- [ ] **Step 8.2: Verify compilation**

Run: `cd sni_tester && go build`
Expected: Success

- [ ] **Step 8.3: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(dns): update resolveWithUDP with proper error handling"
```

---

## Task 9: Final Verification

### Step 9.1: Build the project

Run: `cd sni_tester && go build`
Expected: Success with no errors

### Step 9.2: Run any existing tests

Run: `cd sni_tester && go test ./... -v` (if tests exist)
Expected: All tests pass

### Step 9.3: Manual smoke test

Run: `cd sni_tester && ./sni_tester --help`
Expected: Help output displayed without errors

---

## Summary

This plan implements DNS rate limiter smart scheduling with:

1. **New types and constants** - Priority system, error types, wait configuration
2. **Updated DNSRateLimiter** - Backlog tracking
3. **TryAcquireForPrefetch** - Non-blocking acquisition for prefetch
4. **calculateMaxWait** - Adaptive wait times based on backlog
5. **acquireWithRateLimit** - Timeout-isolated rate limit acquisition
6. **Rewritten Acquire** - Timeout isolation for main tests
7. **Updated prefetch worker** - Uses new TryAcquireForPrefetch
8. **Updated resolveWithUDP** - Proper error handling

**Key Benefits:**
- Rate limiter wait does not consume DNS query timeout
- Prefetch has fast-path non-blocking acquisition
- Main tests have timeout-isolated waiting with cancellation support
- Backlog awareness reduces wait times under high load