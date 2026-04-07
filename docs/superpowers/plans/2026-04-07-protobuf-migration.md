# SNI Tester Protobuf Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate SNI tester from custom binary format to Protobuf, merge REALITY and XHTTP modes into single unified mode.

**Architecture:** Single Protobuf message per country file, unified validation logic (TLS 1.3 + X25519 + H2/H3), removal of legacy format code.

**Tech Stack:** Go with `google.golang.org/protobuf`, Rust with `prost`

---

## File Structure

| File | Action |
|------|--------|
| `proto/sni.proto` | Create |
| `sni_tester/main.go` | Modify (protobuf write, remove mode flags, merge validation) |
| `sni_tester/convert.go` | Delete |
| `rust/tgbot/src/logic/sni_selector.rs` | Modify (protobuf read) |
| `rust/tgbot/build.rs` | Create/Modify (protobuf code generation) |
| `rust/tgbot/Cargo.toml` | Modify (add prost dependency) |

---

## Task 1: Create Protobuf Definition

**Files:**
- Create: `proto/sni.proto`

### Step 1.1: Create proto directory

```bash
mkdir -p proto
```

### Step 1.2: Write protobuf definition

Create `proto/sni.proto`:

```protobuf
syntax = "proto3";
package sni;

option go_package = "sni_tester/proto";

// DomainList stores a list of validated domain names for a country
message DomainList {
    repeated string domains = 1;
}
```

- [ ] **Step 1.3: Commit**

```bash
git add proto/sni.proto
git commit -m "feat(sni): add protobuf definition for domain list"
```

---

## Task 2: Set Up Go Protobuf Generation

**Files:**
- Modify: `sni_tester/go.mod`

### Step 2.1: Install protoc (if not installed)

Check if protoc is installed:
```bash
protoc --version
```

If not installed, install via system package manager.

### Step 2.2: Install Go protobuf plugins

```bash
go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
```

### Step 2.3: Update go.mod to add protobuf dependency

```bash
cd sni_tester && go get google.golang.org/protobuf/proto
```

- [ ] **Step 2.4: Commit**

```bash
git add sni_tester/go.mod sni_tester/go.sum
git commit -m "chore(sni): add protobuf dependency for go"
```

---

## Task 3: Generate Go Protobuf Code

**Files:**
- Create: `sni_tester/proto/sni.pb.go`

### Step 3.1: Generate Go protobuf code

```bash
cd sni_tester
protoc --go_out=. --go_opt=paths=source_relative ../proto/sni.proto
```

This generates `sni.pb.go` in the `sni_tester/proto/` directory.

- [ ] **Step 3.2: Commit**

```bash
git add sni_tester/proto/sni.pb.go
git commit -m "feat(sni): generate go protobuf code"
```

---

## Task 4: Implement Protobuf Write Function

**Files:**
- Modify: `sni_tester/main.go`

### Step 4.1: Add protobuf import

Add near other imports:
```go
import (
    // ... existing imports ...
    "google.golang.org/protobuf/proto"
    snipb "sni_tester/proto"
)
```

### Step 4.2: Create writeProtobufDomainFile function

Replace `writeBinaryDomainFile` (around line 2113):

```go
func writeProtobufDomainFile(domains []string, filePath string) error {
    // Sort and deduplicate
    sort.Strings(domains)
    uniqueDomains := []string{}
    for i, d := range domains {
        if i == 0 || d != domains[i-1] {
            uniqueDomains = append(uniqueDomains, d)
        }
    }

    // Create protobuf message
    pb := &snipb.DomainList{Domains: uniqueDomains}
    
    // Marshal to binary
    data, err := proto.Marshal(pb)
    if err != nil {
        return fmt.Errorf("failed to marshal protobuf: %w", err)
    }

    // Write to file
    if err := os.WriteFile(filePath, data, 0644); err != nil {
        return fmt.Errorf("failed to write file: %w", err)
    }

    return nil
}
```

### Step 4.3: Remove old writeBinaryDomainFile function

Delete the `writeBinaryDomainFile` function.

- [ ] **Step 4.4: Verify compilation**

```bash
cd sni_tester && go build
```

- [ ] **Step 4.5: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(sni): replace binary write with protobuf write"
```

---

## Task 5: Implement Protobuf Read Function

**Files:**
- Modify: `sni_tester/main.go`

### Step 5.1: Create parseProtobufDomains function

Replace `parseBinaryDomains` (around line 2149):

```go
func parseProtobufDomains(data []byte) ([]string, error) {
    var pb snipb.DomainList
    if err := proto.Unmarshal(data, &pb); err != nil {
        return nil, fmt.Errorf("failed to unmarshal protobuf: %w", err)
    }
    
    // Filter valid domains
    domains := []string{}
    for _, domain := range pb.Domains {
        if domain != "" && strings.Contains(domain, ".") {
            domains = append(domains, domain)
        }
    }
    
    return domains, nil
}
```

### Step 5.2: Remove old parseBinaryDomains function

Delete the `parseBinaryDomains` function.

- [ ] **Step 5.3: Verify compilation**

```bash
cd sni_tester && go build
```

- [ ] **Step 5.4: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(sni): replace binary read with protobuf read"
```

---

## Task 6: Update File Extension Usage

**Files:**
- Modify: `sni_tester/main.go`

### Step 6.1: Replace .bin with .pb extension

Find all occurrences of `.bin` extension and replace with `.pb`:

```bash
# In writeProtobufDomainFile call sites
# Replace: filepath.Join(targetDir, countryCode+".bin")
# With: filepath.Join(targetDir, countryCode+".pb")
```

### Step 6.2: Update output directory structure

Remove mode-specific subdirectories:
- Replace `reality/` and `xhttp/` subdirectories with flat structure
- Output directly to `targetDir/{COUNTRY_CODE}.pb`

- [ ] **Step 6.3: Verify compilation**

```bash
cd sni_tester && go build
```

- [ ] **Step 6.4: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(sni): update file extension to .pb and flatten directory structure"
```

---

## Task 7: Remove Mode Flags and Merge Validation

**Files:**
- Modify: `sni_tester/main.go`

### Step 7.1: Remove command line flags

Delete:
```go
xhttpMode := flag.Bool("xhttp", false, "Enable XHTTP validation (H2 minimum)")
realityMode := flag.Bool("reality", false, "Enable Reality validation (TLS 1.3, X25519, H2)")
runBoth := flag.Bool("both", false, "Run both Reality and XHTTP modes automatically")
```

Remove mode routing logic (around line 862-871).

### Step 7.2: Create unified validateDomain function

Replace both `validateReality` and `validateXHTTP`:

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

### Step 7.3: Remove old validation functions

Delete `validateReality` and `validateXHTTP` functions.

### Step 7.4: Update validation call sites

Replace all calls to `validateReality` and `validateXHTTP` with `validateDomain`.

- [ ] **Step 7.5: Verify compilation**

```bash
cd sni_tester && go build
```

- [ ] **Step 7.6: Commit**

```bash
git add sni_tester/main.go
git commit -m "feat(sni): merge REALITY and XHTTP into unified validateDomain"
```

---

## Task 8: Clean Up Legacy Code

**Files:**
- Delete: `sni_tester/convert.go`

### Step 8.1: Delete convert.go

```bash
rm sni_tester/convert.go
```

### Step 8.2: Remove any remaining bin/txt parsing code

Search for and remove:
- `writeBinaryDomainFile` remnants
- `parseBinaryDomains` remnants
- Any txt parsing functions

- [ ] **Step 8.3: Verify compilation**

```bash
cd sni_tester && go build
```

- [ ] **Step 8.4: Commit**

```bash
git add -A
git commit -m "chore(sni): remove legacy binary and text format code"
```

---

## Task 9: Update Rust Protobuf Support

**Files:**
- Modify: `rust/tgbot/Cargo.toml`
- Create/Modify: `rust/tgbot/build.rs`
- Modify: `rust/tgbot/src/logic/sni_selector.rs`

### Step 9.1: Add prost dependency to Cargo.toml

```toml
[dependencies]
prost = "0.12"

[build-dependencies]
prost-build = "0.12"
```

### Step 9.2: Create build.rs for protobuf generation

Create `rust/tgbot/build.rs`:

```rust
fn main() {
    prost_build::compile_protos(&["../../proto/sni.proto"], &["../../proto"])
        .expect("Failed to compile protobuf");
}
```

### Step 9.3: Update sni_selector.rs to use protobuf

Replace `load_binary` with `load_protobuf`:

```rust
pub mod sni {
    include!(concat!(env!("OUT_DIR"), "/sni.rs"));
}

fn load_protobuf(data: &[u8]) -> Option<Vec<String>> {
    let list: sni::DomainList = prost::Message::decode(data).ok()?;
    Some(list.domains)
}
```

### Step 9.4: Update resource loading

Change from `{reality,xhttp}/{COUNTRY_CODE}.bin` to `{COUNTRY_CODE}.pb`.

- [ ] **Step 9.5: Verify Rust build**

```bash
cd rust/tgbot && cargo build
```

- [ ] **Step 9.6: Commit**

```bash
git add rust/tgbot/
git commit -m "feat(sni): update rust to use protobuf format"
```

---

## Task 10: Clean Up Old Resource Files

### Step 10.1: Delete old .bin directories

```bash
rm -rf rust/tgbot/src/resources/sni/reality
rm -rf rust/tgbot/src/resources/sni/xhttp
```

- [ ] **Step 10.2: Commit**

```bash
git add -A
git commit -m "chore(sni): remove old binary resource files"
```

---

## Task 11: Final Verification

### Step 11.1: Build Go

```bash
cd sni_tester && go build
```

### Step 11.2: Build Rust

```bash
cd rust/tgbot && cargo build
```

### Step 11.3: Test End-to-End (Optional)

If test domains are available:
1. Run sni_tester to generate `.pb` files
2. Verify Rust can read them

---

## Summary

This plan migrates SNI tester from custom binary format to Protobuf:

1. **New protobuf definition** - `proto/sni.proto`
2. **Go protobuf support** - Generate code, replace read/write functions
3. **Rust protobuf support** - Add prost, update reader
4. **Mode unification** - Merge REALITY/XHTTP into single mode
5. **Directory flattening** - Remove `reality/` and `xhttp/` subdirectories
6. **Legacy cleanup** - Remove all bin/txt format code

**Key Benefits:**
- Standard protobuf format for cross-language support
- Simpler codebase (one mode, one format)
- Better extensibility for future fields