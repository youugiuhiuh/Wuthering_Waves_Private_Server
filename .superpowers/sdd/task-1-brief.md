### Task 1: `PlatformCapabilities::DISCORD` const + `DiscordAdapter::capabilities()`

**Files:**
- Modify: `src/adapters/common/trait.rs` — add const
- Modify: `src/adapters/discord/adapter.rs` — use const in `capabilities()`
- Test: inline `#[cfg(test)]` in adapter.rs

**Interfaces:**
- Consumes: `PlatformCapabilities::TELEGRAM` (existing pattern, line ~46)
- Produces: `PlatformCapabilities::DISCORD` const, used by `DiscordAdapter::capabilities()`

- [ ] **Step 1: Write failing test in adapter.rs**

```rust
#[cfg(test)]
mod tests {
    use crate::adapters::common::PlatformCapabilities;

    #[test]
    fn discord_capabilities_matches_expected() {
        let caps = PlatformCapabilities::DISCORD;
        assert!(caps.can_edit_message);
        assert!(caps.can_delete_message);
        assert!(!caps.has_file_transfer);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test discord_capabilities -- --ignored
```
Expected: compile error — `DISCORD` not defined on `PlatformCapabilities`

- [ ] **Step 3: Add const to `src/adapters/common/trait.rs` after the existing `TELEGRAM` const (line ~52)**

```rust
pub const DISCORD: Self = Self {
    can_edit_message: true,
    can_delete_message: true,
    has_inline_keyboard: true,
    has_slash_commands: true,
    has_file_transfer: false,
};
```

- [ ] **Step 4: Replace `DiscordAdapter::capabilities()` (lines 89-97 of adapter.rs)**

Change from manual struct to:
```rust
fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities::DISCORD
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test discord_capabilities -v
```
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/adapters/common/trait.rs src/adapters/discord/adapter.rs
git commit -m "feat(aegis): add PlatformCapabilities::DISCORD const"
```

---
