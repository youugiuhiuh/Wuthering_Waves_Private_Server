# Aegis Complete Platform Decoupling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Every task requires specification compliance review followed by code quality review.

**Goal:** Remove platform interaction from Aegis business code and route explicitly protected output to Matrix in Telegram + Matrix installations.

**Architecture:** Gateways map SDK updates into platform-neutral application events and render platform-neutral output actions. Application workflows assign sensitivity explicitly. A composition-level router sends Public output to the origin and Protected output to Matrix without inspecting content.

**Tech Stack:** Rust, Tokio, async-trait, Teloxide, matrix-sdk, Serenity/Poise, Go installer, systemd

## Global Constraints

- Follow RED -> GREEN -> REFACTOR for every behavior change.
- Do not add dependencies.
- Do not restore `RoutingAdapter`, `is_sensitive`, or content-based routing.
- Protected output must never fall back to Telegram in combination mode.
- Discord remains standalone; `--all` means Telegram plus Matrix only.
- Keep patches below 200 lines by completing one vertical slice at a time.
- Commit only after the task's targeted checks pass.

---

### Task 1: Characterize Legacy Interaction Behavior

**Files:** Modify tests in `rust/aegis/src/shared/dispatch.rs`, `rust/aegis/src/shared/handlers/*.rs`, and `rust/aegis/src/shared/destruct.rs`.

**Produces:** Tests recording send, edit, delete, callback acknowledgement, file input, and generated configuration behavior before migration.

- [ ] Add one focused fake-output assertion for each currently untested interaction operation.
- [ ] Run `cargo nextest run shared:: --no-capture` and confirm all characterization tests pass before production changes.
- [ ] Commit as `test(aegis): characterize legacy interaction flows`.

### Task 2: Define Complete Application Interaction Contracts

**Files:** Modify `rust/aegis/src/app/interaction.rs`, `rust/aegis/src/app/output.rs`, and `rust/aegis/src/app/mod.rs`.

**Produces:** `ApplicationEvent`, `InboundAttachment`, `OutputAction`, `OutputPayload`, `ActionButton`, and `BusinessOutput::publish(OutputAction)`.

- [ ] Write failing contract tests for commands, text, callbacks, downloaded attachments, cross-platform identity, and sensitivity preservation.
- [ ] Run `cargo nextest run app::interaction` and verify failures identify missing variants.
- [ ] Add platform-neutral event and output types; keep SDK types out of public signatures.
- [ ] Run the targeted tests and `cargo check --all-targets`.
- [ ] Commit as `refactor(app): define complete interaction contracts`.

### Task 3: Add Explicit Sensitive Output Router

**Files:** Create `rust/aegis/src/gateways/output_router.rs`; modify `rust/aegis/src/gateways/mod.rs`.

**Consumes:** `BusinessOutput::publish(OutputAction)` and explicit `Sensitivity`.

**Produces:** `SensitiveOutputRouter` configured with origin output, optional Matrix output, and combination-mode policy.

- [ ] Write failing tests for Public origin delivery, single-platform Protected delivery, combination-mode Matrix delivery, missing Matrix, and Matrix publish failure.
- [ ] Assert protected test bytes are never observed by the Telegram fake in both failure cases.
- [ ] Implement routing using only action type, `Sensitivity`, and configured mode.
- [ ] On protected Matrix failure, publish a fixed Public failure notice to the origin and return the original error.
- [ ] Run `cargo nextest run gateways::output_router`.
- [ ] Commit as `feat(gateways): route explicit protected output`.

### Task 4: Implement Presenter Contract on All Platforms

**Files:** Modify `rust/aegis/src/gateways/telegram/presenter.rs`, `rust/aegis/src/gateways/matrix/presenter.rs`, and `rust/aegis/src/gateways/discord/presenter.rs`.

**Produces:** Complete mappings for send, edit, delete, callback acknowledgement, buttons, and attachments.

- [ ] Add presenter contract tests using existing SDK mocking seams.
- [ ] Verify Protected text becomes `message.txt`, while Public text remains a normal message.
- [ ] Implement each output-action mapping without business branching.
- [ ] Run all gateway presenter tests and `cargo check --all-targets`.
- [ ] Commit one platform at a time using `refactor(<platform>): implement application output actions`.

### Task 5: Migrate Commands, Menu, and Authentication

**Files:** Modify `rust/aegis/src/app/service.rs`, `rust/aegis/src/app/auth.rs`, `rust/aegis/src/shared/commands.rs`, and `rust/aegis/src/shared/handlers/menu.rs`.

**Produces:** Command, menu, and authentication paths that consume application events and publish output actions without `BotAdapter`.

- [ ] Add failing Telegram/Matrix/Discord parity tests for Help, Start, Menu, Auth, and SetSecurityFile.
- [ ] Move authentication responses and semantic buttons behind application output actions.
- [ ] Remove the command bridge after all command variants use `ApplicationService` directly.
- [ ] Run `cargo nextest run app::service shared::commands handlers::menu`.
- [ ] Commit as `refactor(app): migrate command and auth interactions`.

### Task 6: Remove Adapters from Legacy Event Types

**Files:** Modify `rust/aegis/src/shared/types.rs`, `rust/aegis/src/shared/dispatch.rs`, and `rust/aegis/src/shared/handlers/mod.rs`.

**Produces:** Adapter-free command, message, and callback events; `dispatch_event(event, state, output)`.

- [ ] Write failing tests proving events can be constructed with identities and payloads only.
- [ ] Remove `Arc<dyn BotAdapter>` fields and pass `&dyn BusinessOutput` explicitly through dispatch.
- [ ] Preserve `(Platform, ConversationId)` workflow isolation.
- [ ] Run dispatch and type tests.
- [ ] Commit as `refactor(app): remove adapters from input events`.

### Task 7: Migrate Callback and Operations Handlers

**Files:** Modify `rust/aegis/src/shared/handlers/callback.rs`, `log.rs`, `message.rs`, `ops.rs`, and `state_ops.rs`.

**Produces:** Platform-neutral callback and operations output.

- [ ] For each handler, write a failing output-action test before replacing direct adapter calls.
- [ ] Migrate send/edit/delete/answer-callback calls one handler at a time.
- [ ] Run the handler's targeted test after each migration.
- [ ] Commit each independently reviewable handler group.

### Task 8: Migrate Configuration-Producing Workflows

**Files:** Modify `rust/aegis/src/shared/handlers/xray.rs`, `singbox.rs`, and `warp.rs`.

**Produces:** Explicitly classified configuration output with no text inspection.

- [ ] Add failing tests that proxy URIs, complete client configurations, passwords, private keys, and secret keys produce `Sensitivity::Protected`.
- [ ] Add tests that status, progress, menus, and validation errors remain Public.
- [ ] Replace direct sends with output actions and assign sensitivity at construction.
- [ ] Run targeted workflow tests after each file.
- [ ] Commit each workflow separately.

### Task 9: Migrate Destruct, Files, and Reporters

**Files:** Modify `rust/aegis/src/shared/destruct.rs`, `state_ops.rs`, `reporters.rs`, and gateway input mapping modules.

**Produces:** Downloaded attachment bytes at the application boundary and output-port-based background reporting.

- [ ] Add failing tests for gateway file download mapping and protected security material.
- [ ] Download platform files in gateways before creating `ApplicationEvent`.
- [ ] Ensure TOTP, secrets, and security-file contents are never Public or echoed.
- [ ] Replace reporter adapter storage with a supplied application output port.
- [ ] Run destruct, file, and scheduler reporter tests.
- [ ] Commit as `refactor(app): migrate file and reporter interactions`.

### Task 10: Wire Runtime and Degraded Matrix Mode

**Files:** Modify `rust/aegis/src/main.rs`, `rust/aegis/src/main/runtime.rs`, `rust/aegis/src/main/matrix.rs`, and `rust/aegis/src/main/mod.rs`.

**Produces:** Runtime composition for single-platform and Telegram + Matrix routing.

- [ ] Add failing runtime tests for all installation modes and Matrix connection failure policy.
- [ ] Wire gateway input mapping, presenters, and `SensitiveOutputRouter` outside `AppState`.
- [ ] Keep Matrix-only startup fail-fast; let combination mode continue Telegram with a missing protected sink.
- [ ] Run runtime tests and `cargo check --all-targets`.
- [ ] Commit as `refactor(main): compose platform-neutral application runtime`.

### Task 11: Delete Legacy Interaction Infrastructure

**Files:** Modify or delete legacy definitions under `rust/aegis/src/common`, `rust/aegis/src/shared`, and temporary bridge code.

**Produces:** No business-layer `BotAdapter`, `RoutingAdapter`, `PlatformCapabilities`, or content heuristic.

- [ ] Add an architecture check that rejects SDK, gateway, and `BotAdapter` references in core/application/business paths.
- [ ] Delete unreachable legacy code and mocks only after all callers migrate.
- [ ] Run `cargo nextest run --all-features` and the architecture check.
- [ ] Commit as `refactor(aegis): remove legacy platform interaction layer`.

### Task 12: Preserve Installer Platform Mode on Upgrade

**Files:** Modify `go/installer/main.go` and existing installer test files.

**Produces:** Fresh-install flags for four modes and upgrade preservation of the current systemd flag.

- [ ] Write failing table tests for `tg`, `matrix`, `tg-matrix`, and `discord`.
- [ ] Write a failing upgrade test proving an existing `--all`, `--matrix`, or `--discord` ExecStart is not replaced with Telegram-only mode.
- [ ] Implement minimal parsing/preservation of the existing service flag.
- [ ] Run `go test ./...` and `staticcheck ./...` from `go/installer`.
- [ ] Commit as `fix(installer): preserve platform mode during upgrade`.

### Task 13: Final Verification and Review

**Files:** No production changes unless verification or review finds a defect.

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo nextest run --all-features`.
- [ ] Run `cargo test --doc`.
- [ ] Run Go formatting, tests, and static analysis.
- [ ] Search for forbidden SDK/gateway imports, `BotAdapter`, `RoutingAdapter`, `PlatformCapabilities`, and `is_sensitive` in business paths.
- [ ] Use `requesting-code-review`; block completion on Critical findings.
- [ ] Use `finishing-a-development-branch` and wait for the user's merge/PR/keep decision.

## Deployment Gate

Deployment is not part of implementation completion. After explicit approval, build only from the verified feature-worktree HEAD, record the commit SHA, deploy that binary, restart `wwps-aegis.service`, and verify the running artifact corresponds to that SHA.
