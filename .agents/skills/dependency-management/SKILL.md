---
name: dependency-management
description: Use when adding, removing, or updating Go/Rust dependencies. Enforces using go/cargo commands instead of direct file edits.
---

# Dependency Management

Enforces using official package manager commands instead of direct file edits for dependency changes.

## When to Use

- Adding new Go dependencies
- Adding new Rust dependencies
- Removing Go/Rust dependencies
- Updating dependency versions

## The Rule

**NEVER edit `go.mod`, `go.sum`, or `Cargo.toml` directly.**

### Go Dependencies

**Add/Update:**
```bash
go get github.com/pkg/example@latest
go add ./...
```

**Remove (remove import first, then):**
```bash
go mod tidy
```

**Install specific version:**
```bash
go get github.com/pkg/example@v1.2.3
```

### Rust Dependencies

**Add:**
```bash
cargo add serde --features derive
cargo add tokio --features "full"
cargo add anyhow
```

**Add dev/test dependency:**
```bash
cargo add --dev mockall
cargo add --test serde_json
```

**Remove:**
```bash
cargo remove unused_crate
```

**Update:**
```bash
cargo update
cargo update package_name
```

## Workflow

1. **Identify** the dependency needed
2. **Run** the appropriate `go get` / `cargo add` command
3. **Verify** `go.mod` / `Cargo.toml` updated automatically
4. **Run** lint checks (`go mod verify` / `cargo check`)
5. **Commit** the changes

## Project-Specific Notes

- Go projects: `go/installer/`, `tools/bin2pb/`, `sni_tester/`
- Rust projects: `rust/tgbot/`, `rust/version-sync/`
- Run from the project root (where `go.mod` or `Cargo.toml` lives)