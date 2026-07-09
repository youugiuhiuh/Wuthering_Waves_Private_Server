# Installer Discord 部署支援實現計劃

**Goal:** 在 installer 3 條 setup 路徑中添加 Discord bot 部署支援

**Architecture:** 與現有 Matrix 段完全對稱：`buildSetupPayload` 簽名擴展 + `setupConfig` 字段 + `parseKeyVal` case + `firstTimeSetup` 互動段 + i18n

**Tech Stack:** Go, memguard

## Global Constraints

- Use `go test ./...` for testing
- Use `go fmt ./...` for formatting
- No new dependencies needed
- Interaction with `--setup-stdin` (raw JSON) unchanged — aegis `SetupInput` already accepts discord fields

---

### Task 1: `buildSetupPayload` + `setupConfig` + `parseKeyVal` + `installFromKeyVal`

**Files:**
- Modify: `go/installer/main.go` — buildSetupPayload signature + body, setupConfig struct, parseKeyVal switch, installFromKeyVal call

- [ ] **Step 1: Extend `setupConfig` struct** — add `DiscordToken string` and `DiscordAdminID string`

- [ ] **Step 2: Extend `buildSetupPayload` signature** — add `discordToken, discordAdminID string` params; append JSON fields when non-empty

- [ ] **Step 3: Extend `parseKeyVal`** — add `case "discord_token"` and `case "discord_admin_id"`

- [ ] **Step 4: Update `installFromKeyVal` caller** — pass `cfg.DiscordToken, cfg.DiscordAdminID` to `buildSetupPayload`

- [ ] **Step 5: Run tests** — `go test ./...` should pass (update 4 existing callers)

- [ ] **Step 6: Commit** — `git add go/installer/main.go && git commit -m "feat(installer): add Discord fields to buildSetupPayload + parseKeyVal"`

---

### Task 2: `firstTimeSetup` Discord interactive section

**Files:**
- Modify: `go/installer/main.go` — add Discord section after Matrix section (~line 990)

- [ ] **Step 1: Add Discord section in `firstTimeSetup`** after Matrix block — y/n prompt → token (readSecureInput) → admin_id (readSecureInputStr) → intent warning → guild warning. Pass values to `buildSetupPayload`.

- [ ] **Step 2: Build + test** — `go build ./... && go test ./...`

- [ ] **Step 3: Commit** — `git add go/installer/main.go && git commit -m "feat(installer): add interactive Discord setup to firstTimeSetup"`

---

### Task 3: i18n — zh.json + en.json + ja.json

**Files:**
- Modify: `go/installer/i18n/zh.json` — add 16 `firsttime.discord_*` keys
- Modify: `go/installer/i18n/en.json` — same keys (English translations)
- Modify: `go/installer/i18n/ja.json` — same keys (Japanese translations)

- [ ] **Step 1: Read existing zh.json** — copy the `firsttime.matrix_*` block's format

- [ ] **Step 2: Add discord keys to zh.json** — all 16 keys with Chinese values

- [ ] **Step 3: Add discord keys to en.json** — English translations

- [ ] **Step 4: Add discord keys to ja.json** — Japanese translations

- [ ] **Step 5: `go test ./...`** — verify no regressions

- [ ] **Step 6: Commit** — `git add go/installer/i18n/ && git commit -m "feat(installer): add Discord setup i18n keys for zh/en/ja"`

---

### Task 4: Tests

**Files:**
- Modify: `go/installer/main_test.go` — update 4 existing `buildSetupPayload` callers; add Discord subtests

- [ ] **Step 1: Update existing `buildSetupPayload` callers** — add `"", ""` to all 4 calls (3 in TestBuildSetupPayload, 1 in TestParseKeyVal)

- [ ] **Step 2: Add "with discord" subtest** — `buildSetupPayload` with token+admin_id → verify JSON contains both fields

- [ ] **Step 3: Add "without discord" subtest** — `buildSetupPayload` with empty discord params → verify JSON does NOT contain discord fields

- [ ] **Step 4: Add parseKeyVal Discord subtest** — `discord_token=xxx\ndiscord_admin_id=123` → struct fields correct

- [ ] **Step 5: `go test ./...`** — all pass

- [ ] **Step 6: Commit** — `git add go/installer/main_test.go && git commit -m "test(installer): add Discord setup payload tests"`

---

## Verification Gate

```bash
go fmt ./... && go test ./...
```
All pass before finishing-a-development-branch.
