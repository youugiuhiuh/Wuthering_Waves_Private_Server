# SNI Tester Protobuf Migration Design

**Date**: 2026-04-07
**Status**: Draft
**Related**: sni_tester data format migration

## Problem Statement

The current SNI tester uses a simple binary format for storing validated domains:
- Length-prefixed domain strings (2 bytes BE + domain)
- No metadata, no extensibility
- Separate storage for REALITY and XHTTP modes
- Requires exact coordination between Go writer and Rust reader

**Goals**:
1. Migrate to Protobuf format for better cross-language support
2. Remove txt and bin format support (no backward compatibility needed)
3. Merge REALITY and XHTTP modes (same requirements except H3)
4. Simplify storage structure

## Current Architecture

### Binary Format (Current)

```
[2 bytes length (BE)][domain string]...
```

- Length: uint16, Big Endian
- Max domain length: 512 characters
- No header, no metadata

### Directory Structure (Current)

```
rust/tgbot/src/resources/sni/
├── reality/
│   ├── US.bin
│   ├── GB.bin
│   └── ...
└── xhttp/
    ├── US.bin
    ├── GB.bin
    └── ...
```

### Mode Distinction (Current)

| Mode | Requirements |
|------|-------------|
| REALITY | TLS 1.3 + X25519 + H2 |
| XHTTP | TLS 1.3 + X25519 + (H2 or H3) |

**Difference**: Only H3 support check differs.

## Proposed Architecture

### Protobuf Definition

**File**: `proto/sni.proto`

```protobuf
syntax = "proto3";
package sni;

option go_package = "sni_tester/proto";

// DomainList stores a list of validated domain names for a country
message DomainList {
    repeated string domains = 1;
}
```

### Directory Structure (New)

```
rust/tgbot/src/resources/sni/
├── US.pb
├── GB.pb
├── ...
└── {COUNTRY_CODE}.pb
```

### Mode Simplification

Remove `-xhttp`, `-reality`, `-both` flags. Single unified mode:
- Validate TLS 1.3 + X25519 + (H2 or H3)
- All valid domains stored together

### Validation Logic

```go
func validateDomain(result *TLSResult, resolver *net.Resolver) (bool, string) {
    // 1. TLS 1.3 required
    if result.TLSVersion != utls.VersionTLS13 {
        return false, "TLS 1.3 required"
    }
    
    // 2. X25519-based key exchange required
    if !isValidKeyGroup(result.KeyGroup) {
        return false, "X25519-based key exchange required"
    }
    
    // 3. H2 OR H3 required
    if result.ALPN == "h2" {
        return true, "Validated (H2)"
    }
    
    // Check for H3 via Alt-Svc header
    h3Supported := checkH3Support(result.Domain, result.IP, resolver)
    if h3Supported {
        return true, "Validated (H3)"
    }
    
    return false, "Neither H2 nor H3 support detected"
}
```

## File Changes

### New Files

| File | Purpose |
|------|---------|
| `proto/sni.proto` | Protobuf definition |

### Modified Files

| File | Changes |
|------|---------|
| `sni_tester/main.go` | Replace bin format with protobuf, remove mode flags, merge validation logic |
| `rust/tgbot/src/logic/sni_selector.rs` | Update reader to use protobuf format |
| `sni_tester/convert.go` | Remove (no longer needed) |

### Deleted Files/Directories

| Path | Reason |
|------|--------|
| `sni_tester/convert.go` | No longer converting between formats |
| `rust/tgbot/src/resources/sni/reality/` | Merged into single directory |
| `rust/tgbot/src/resources/sni/xhttp/` | Merged into single directory |

## Implementation Steps

1. Create `proto/sni.proto` with DomainList message
2. Add protobuf dependencies to Go (`google.golang.org/protobuf`)
3. Add protobuf dependencies to Rust (`prost`)
4. Generate Go code: `protoc --go_out=. proto/sni.proto`
5. Generate Rust code: `protoc --prost_out=. proto/sni.proto`
6. Replace `writeBinaryDomainFile` with `writeProtobufDomainFile`
7. Replace `parseBinaryDomains` with `parseProtobufDomains`
8. Remove `-xhttp`, `-reality`, `-both` flags
9. Merge `validateReality` and `validateXHTTP` into single `validateDomain`
10. Update Rust `load_binary` to `load_protobuf`
11. Run migration tool to convert existing `.bin` files to `.pb`
12. Remove old binary format code
13. Clean up empty `reality/` and `xhttp/` directories

## Dependencies

### Go

```go
import (
    "google.golang.org/protobuf/proto"
    snipb "sni_tester/proto"
)
```

### Rust

```toml
[dependencies]
prost = "0.12"
```

```rust
pub mod sni {
    include!(concat!(env!("OUT_DIR"), "/sni.rs"));
}
```

## Migration Strategy

Since backward compatibility is NOT required:

1. Run sni_tester to generate new `.pb` files
2. Delete old `.bin` files
3. Remove old format code

No need for a migration tool.

## Testing

1. Verify protobuf generation: `protoc --go_out=. proto/sni.proto`
2. Verify Go build: `cd sni_tester && go build`
3. Verify Rust build: `cd rust/tgbot && cargo build`
4. Test end-to-end: Generate domains, verify Rust can read them

## Risks

| Risk | Mitigation |
|------|------------|
| Protobuf version mismatch | Use same proto file for both languages |
| File encoding issues | Use UTF-8 for domain strings (protobuf default) |
| Large file performance | Protobuf is efficient for repeated strings |

## Success Criteria

- [ ] `sni_tester` compiles with protobuf
- [ ] Rust `sni_selector` compiles with protobuf
- [ ] Generated `.pb` files readable by Rust
- [ ] Old `-xhttp`, `-reality`, `-both` flags removed
- [ ] Single unified validation mode works
- [ ] No txt or bin format code remains