# MessagePack Format Migration Design

**Date**: 2026-04-07
**Status**: Approved
**Related**: sni_tester DNS rate limiter enhancement

## Problem Statement

The current `.bin` format uses a custom length-prefixed binary format that requires manual implementation and maintenance in both Go and Rust. This introduces:
- Maintenance overhead for identical parsing logic
- Risk of implementation divergence between languages
- No standard library support

## Goals

1. **Replace custom binary format** with MessagePack standard serialization
2. **Remove txt and bin support** - completely migrate to MessagePack
3. **Use reliable third-party dependencies** - Go `msgpack/v5`, Rust `rmp-serde`
4. **Simplify codebase** - remove custom parsing functions

## Design Overview

### Format Change

**Before (custom `.bin` format)**:
```
[uint16 BE length][domain string][uint16 BE length][domain string]...
```

**After (MessagePack `.msgpack` format)**:
```rust
// Rust
Vec<String>  // MessagePack serialized string array

// Go
[]string     // MessagePack serialized string slice
```

### File Changes

| File | Action |
|------|--------|
| `sni_tester/go.mod` | Add `github.com/vmihailenco/msgpack/v5` |
| `sni_tester/convert.go` | Rewrite: remove txt/bin, add msgpack conversion |
| `sni_tester/main.go` | Remove bin/txt parsing, add msgpack read/write |
| `rust/tgbot/Cargo.toml` | Add `rmp-serde = "1.3"` |
| `rust/tgbot/src/logic/sni_selector.rs` | Remove load_binary/load_text, add load_msgpack |

### Dependencies

**Go**:
```
github.com/vmihailenco/msgpack/v5 v5.4.1
```

**Rust**:
```toml
rmp-serde = "1.3"
serde = { version = "1.0", features = ["derive"] }
```

## Implementation Details

### Go: MessagePack Writer

```go
package main

import (
    "github.com/vmihailenco/msgpack/v5"
)

// WriteMsgpack writes domain list to MessagePack format
func WriteMsgpack(domains []string) ([]byte, error) {
    return msgpack.Marshal(domains)
}

// ReadMsgpack reads domain list from MessagePack format
func ReadMsgpack(data []byte) ([]string, error) {
    var domains []string
    err := msgpack.Unmarshal(data, &domains)
    return domains, err
}
```

### Go: File Writing (sni_tester/main.go)

```go
// Save domains to file (MessagePack format)
func saveDomainsToFile(domains []string, filename string) error {
    data, err := msgpack.Marshal(domains)
    if err != nil {
        return err
    }
    return os.WriteFile(filename, data, 0644)
}

// Load domains from file (MessagePack format)
func loadDomainsFromFile(filename string) ([]string, error) {
    data, err := os.ReadFile(filename)
    if err != nil {
        return nil, err
    }
    var domains []string
    if err := msgpack.Unmarshal(data, &domains); err != nil {
        return nil, err
    }
    return domains, nil
}
```

### Rust: MessagePack Reader

```rust
use rmp_serde::{Serializer, Deserializer};
use serde::{Deserialize, Serialize};

fn load_msgpack(data: &[u8]) -> Option<Vec<String>> {
    match rmp_serde::from_slice::<Vec<String>>(data) {
        Ok(domains) if !domains.is_empty() => Some(domains),
        _ => None,
    }
}

fn save_msgpack(domains: &[String]) -> Option<Vec<u8>> {
    rmp_serde::to_vec(domains).ok()
}
```

### Rust: File Reading (sni_selector.rs)

```rust
// New: Load from .msgpack file
fn load_domains(proto_prefix: &str, code: &str) -> Vec<String> {
    let code_upper = code.to_uppercase();
    
    // Try .msgpack only
    let msgpack_file = format!("{}/{}.msgpack", proto_prefix, code_upper);
    
    if let Ok(data) = std::fs::read(&msgpack_file) {
        if let Some(domains) = load_msgpack(&data) {
            if domains.len() >= MIN_DOMAINS {
                return domains;
            }
        }
    }
    
    // Fallback to embedded default
    Self::load_embedded("default.msgpack")
        .unwrap_or_else(Self::default_domains)
}
```

### File Reading Priority (New)

**Old**:
```
XX.msgpack → XX.bin → XX.txt → embedded
```

**New**:
```
XX.msgpack → embedded (default.msgpack)
```

## Migration Strategy

### Phase 1: Data Conversion

Run one-time conversion script to migrate existing data:

```bash
# Go conversion tool
go run sni_tester/convert.go --convert-bin-to-msgpack

# Output:
# sni/reality/US.bin → sni/reality/US.msgpack
# sni/xhttp/US.bin → sni/xhttp/US.msgpack
# ... (all country codes)
```

### Phase 2: Code Cleanup

Remove all legacy parsing code:
- Remove `load_binary()` function
- Remove `load_text()` function
- Remove `is_binary_format()` function
- Remove `ReadBinaryDomains()` function
- Remove txt parsing logic

### Phase 3: Update Embedded Resources

Convert embedded `default.bin` to `default.msgpack` in Rust resources.

## File Structure After Migration

```
rust/tgbot/src/resources/sni/
├── reality/
│   ├── default.msgpack
│   ├── US.msgpack
│   ├── CN.msgpack
│   └── ... (other countries)
└── xhttp/
    ├── default.msgpack
    ├── US.msgpack
    ├── CN.msgpack
    └── ... (other countries)
```

## Testing Strategy

1. **Unit Tests**:
   - `ReadMsgpack` / `WriteMsgpack` roundtrip
   - Empty list, single element, large list
   - Invalid data handling

2. **Integration Tests**:
   - Write domains to .msgpack file
   - Read domains from .msgpack file
   - Compare with original data

3. **Cross-Language Tests**:
   - Write in Go, read in Rust
   - Write in Rust, read in Go
   - Verify identical data

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Data loss during migration | Keep .bin files until migration verified |
| Different MessagePack implementations | Use standard-compatible libraries |
| Performance regression | Benchmark before/after |

## Rollback Plan

If issues arise:
1. Revert code changes
2. Keep .bin files until msgpack is stable
3. Add fallback to bin format if needed

## Success Criteria

- [ ] All .bin files converted to .msgpack
- [ ] Go and Rust read/write consistent data
- [ ] Legacy txt/bin parsing code removed
- [ ] All tests pass
- [ ] Performance within acceptable bounds