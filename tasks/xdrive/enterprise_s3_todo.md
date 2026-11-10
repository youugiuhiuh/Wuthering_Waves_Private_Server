# Implementation Todo: Enterprise S3 Presets

- [x] Task 1: Core S3 Constructors & Token Extractors in `xdrive.rs`
  - [x] 1.1: Write unit tests for `new_oracle_cloud_s3` and `new_backblaze_b2` (RED)
  - [x] 1.2: Implement `new_oracle_cloud_s3` and `new_backblaze_b2` (GREEN)
  - [x] 1.3: Write unit tests for `extract_oracle_cloud_tokens` and `extract_backblaze_b2_tokens` (RED)
  - [x] 1.4: Implement token extractors (GREEN)
- [ ] Task 2: Wizard Handlers & i18n
  - [ ] 2.1: Add `oracle` and `b2` buttons and routing in `xray.rs`
  - [ ] 2.2: Add wizard conversational flow in `message.rs`
  - [ ] 2.3: Add i18n entries in `zh.yml`, `en.yml`, `ja.yml`
  - [ ] 2.4: Integration unit tests for conversational flow
- [ ] Task 3: Client `aegis_v2rayN` Presets
  - [ ] 3.1: Add `Oracle Cloud S3` and `Backblaze B2` in `Global.cs` & `AddServerViewModel.cs`
  - [ ] 3.2: Verify via `dotnet test`
- [ ] Task 4: Lint, Review & Merge
  - [ ] 4.1: Run `cargo fmt`, `clippy`, and `nextest`
  - [ ] 4.2: Merge to main and clean up worktree
