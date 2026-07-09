### Task 1: bootstrap.rs — EncryptedConfig/SetupInput/Drop/run_setup + atomic clear helper

**Files:**
- Modify: `src/bootstrap.rs`

**Interfaces:**
- Produces: `matrix_recovery_key: Option<Vec<u8>>` on `EncryptedConfig`
- Produces: `matrix_recovery_key: Option<String>` on `SetupInput`
- Produces: `pub fn clear_matrix_recovery_key(config_dir: &Path) -> Result<()>`

Steps (exact code in plan file, each change is 1-3 lines following existing pattern):

1. Add `matrix_recovery_key: Option<Vec<u8>>` to `EncryptedConfig` (with `#[serde(default)]`)
2. Add `matrix_recovery_key: Option<String>` to `SetupInput` (with `#[serde(default)]`)
3. Add `if let Some(v) = &mut self.matrix_recovery_key { v.zeroize(); }` to Drop impl
4. In `run_setup()`: add `matrix_recovery_key: Option<&str>` param, encrypt, add to EncryptedConfig
5. In `run_setup_from_stdin()`: pass `input.matrix_recovery_key.as_deref()` to run_setup
6. Add `clear_matrix_recovery_key()` function: read config.enc → set field to None → write tmp + fsync + rename

Run `cargo check && cargo test` before commit.
