# DNS Rate Limiter Smart Scheduling Design

**Date**: 2026-04-07
**Status**: Approved
**Related**: sni_tester DNS rate limiting enhancement

## Problem Statement

The current DNS rate limiter in `sni_tester/main.go` uses `rate.Limiter.Wait(ctx)` which consumes the DNS query context's deadline (800ms timeout). When rate limiting requires waiting, the time spent waiting reduces the actual time available for DNS queries, causing `context deadline exceeded` errors.

**Symptom**:
```
[FAIL] domain: context deadline exceeded
```

**Root Cause**:
- `Acquire(ctx, server, isDoHOrDoT)` uses passed `ctx` for `Wait()` blocking
- Wait time > remaining DNS timeout → context deadline exceeded
- Prefetch and main tests compete for rate limiter slots without priority distinction

## Goals

1. **Isolate rate limiting from DNS timeout** - Rate limit wait should not consume DNS query timeout
2. **Smart scheduling** - Prefetch (fast path) vs Main test (normal path) with different strategies
3. **Backlog awareness** - Reduce wait times when system is overloaded
4. **Stability under load** - Handle large-scale tests (>1000 domains) gracefully

## Design Overview

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    DNSRateLimiter v2                        │
├─────────────────────────────────────────────────────────────┤
│  Layer 1: Concurrency Control                               │
│  - Semaphore (max 100 concurrent)                           │
│  - Prefetch: non-blocking try                               │
│  - Main test: brief wait for slot                            │
├─────────────────────────────────────────────────────────────┤
│  Layer 2: Global Rate Limiter                                │
│  - Global limit (300 QPS)                                    │
│  - Reserve() + Delay() prediction                            │
│  - Independent context for waiting                           │
├─────────────────────────────────────────────────────────────┤
│  Layer 3: Provider Rate Limiter                              │
│  - Aliyun DoH/DoT (15 QPS)                                   │
│  - Aliyun UDP/TCP (80 QPS)                                   │
│  - Tencent DNSPod (50 QPS)                                   │
│  - Other domestic (50 QPS)                                   │
│  - International (500 QPS)                                   │
└─────────────────────────────────────────────────────────────┘
```

### Priority System

| Priority | Use Case | Wait Strategy | Backlog Behavior |
|----------|----------|---------------|------------------|
| `PriorityPrefetch` | DNS prefetch workers | Non-blocking, skip on rate limit | Skip immediately when backlog > threshold |
| `PriorityNormal` | Main test workers | Wait up to maxWait, separate context | Reduce wait time when backlog > threshold |

### Key API Changes

#### New Types and Constants

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

#### Configuration Constants

```go
const (
    // Concurrency control
    dnsMaxConcurrent = 100 // Max concurrent DNS queries

    // Global rate limits by provider
    dnsGlobalLimit        = 300 // Global max QPS
    dnsAliyunDoHLimit    = 15  // Aliyun DoH/DoT: official 20 QPS/IP
    dnsAliyunUDPLimit    = 80  // Aliyun UDP/TCP: official ~100 QPS/IP
    dnsTencentLimit      = 50  // Tencent DNSPod: global limit
    dnsDomesticLimit     = 50  // Other domestic DNS
    dnsInternationalLimit = 500 // International DNS
    dnsBurstSize         = 20  // Burst size

    // Backlog thresholds
    dnsBacklogThreshold = 30 // Backlog threshold for high-load mode

    // Wait strategies (normal load)
    dnsNormalSemaphoreWait = 50 * time.Millisecond
    dnsNormalRateWait     = 50 * time.Millisecond

    // Wait strategies (high load)
    dnsHighLoadSemaphoreWait = 20 * time.Millisecond
    dnsHighLoadRateWait     = 10 * time.Millisecond
)
```

#### Updated DNSRateLimiter Structure

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

#### New Methods

**1. TryAcquireForPrefetch** (Non-blocking for prefetch workers)

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
        return nil, false // Concurrency full, skip
    }

    // 3. Check global rate limit (non-blocking)
    if !r.globalLimiter.Allow() {
        <-r.semaphore
        return nil, false
    }

    return func() { <-r.semaphore }, true
}
```

**2. Acquire** (With timeout isolation for main tests)

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

### Caller Modifications

#### Prefetch Worker (Line ~1063)

**Before**:
```go
if !dnsRateLimiter.TryAcquire() {
    continue
}
```

**After**:
```go
release, acquired := dnsRateLimiter.TryAcquireForPrefetch()
if !acquired {
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

#### resolveWithUDP (Line ~2387)

**Before**:
```go
release, err := dnsRateLimiter.Acquire(ctx, server, false)
if err != nil {
    lastErr = err
    continue
}
```

**After**:
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

## File Modifications Summary

| File | Lines | Change |
|------|-------|--------|
| `main.go` | 50-60 | Add new configuration constants |
| `main.go` | ~89 | Add `DNSPriority` type and error variables |
| `main.go` | ~92 | Add `MaxWaitConfig` struct |
| `main.go` | 149-156 | Update `DNSRateLimiter` struct (add `backlog`) |
| `main.go` | 175-201 | Replace `Acquire` and `TryAcquire` with new implementations |
| `main.go` | 1062-1073 | Update prefetch worker to use `TryAcquireForPrefetch` |
| `main.go` | 2387-2391 | Update `resolveWithUDP` to handle new error types |

## Testing Strategy

1. **Unit Tests**:
   - `TryAcquireForPrefetch` returns false when backlog exceeds threshold
   - `Acquire` respects cancellation during semaphore acquisition
   - `Acquire` returns `ErrRateLimited` when delay exceeds maxWait
   - Rate limit wait does not consume caller's context deadline

2. **Integration Tests**:
   - Run with >1000 domains to verify backlog handling
   - Verify DNS timeout isolation (rate limit wait + DNS query < total timeout)
   - Compare error rates before and after changes

3. **Benchmark**:
   - Measure time spent in rate limiter vs DNS queries
   - Verify prefetch skip rate under high load

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Prefetch skipping too often | Backlog threshold tuned based on testing |
| Rate limit wait still causing timeouts | Separate context with `time.After()` |
| Complex state management | Clear separation of concerns, atomic operations |

## Rollback Plan

If issues arise:
1. Revert `DNSRateLimiter` changes
2. Restore original `Acquire` and `TryAcquire` methods
3. Original implementation used blocking `Wait(ctx)` which caused the problem initially

## Success Criteria

- [ ] DNS rate limiting does not interfere with DNS query timeouts
- [ ] Prefetch operates efficiently with non-blocking acquisition
- [ ] Main tests can complete DNS queries within timeout even under rate limiting
- [ ] Large-scale tests (>1000 domains) complete without `context deadline exceeded` errors