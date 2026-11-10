# Task Plan: XDRIVE Enterprise S3 Presets (Oracle Cloud & Backblaze B2)

## Architecture & Dependency Boundaries
- **Group A (Independent)**: Core constructors and token extractors in `rust/aegis/src/core/xray/xdrive.rs` (Task 1).
- **Group B (Depends on A)**: Telegram wizard routing, message handling, and i18n in `rust/aegis/src/shared/handlers/` (Task 2).
- **Group C (Independent parallel client)**: `aegis_v2rayN` GUI preset wiring & tests (Task 3).
- **Group D (Depends on A, B, C)**: Lint/Format, full regression tests, PR & cleanup (Task 4).

## Detailed Tasks
1. **Task 1: Core S3 Constructors & Token Extractors**
   - Write unit tests in `xdrive.rs` (TDD RED).
   - Implement `new_oracle_cloud_s3` and `new_backblaze_b2`.
   - Implement `extract_oracle_cloud_tokens` and `extract_backblaze_b2_tokens`.
   - Verify GREEN with `cargo test`.
2. **Task 2: Wizard Handlers & i18n**
   - Add buttons in `u_xdrive_menu`.
   - Handle callback routing in `xray.rs`.
   - Add conversational session flow in `message.rs`.
   - Update `zh.yml`, `en.yml`, `ja.yml`.
   - Unit tests for conversational wizard.
3. **Task 3: Client `aegis_v2rayN` Presets**
   - Add `"Oracle Cloud S3"`, `"Backblaze B2"` to `Global.XdriveProviders`.
   - Configure `ApplyXdriveProviderPreset` with pathStyle and default regions.
   - Run `dotnet test`.
4. **Task 4: Quality Review & Ship**
   - `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run`.
   - Commit under 200 lines micro-commits.
   - Merge PR, clean up worktree.
