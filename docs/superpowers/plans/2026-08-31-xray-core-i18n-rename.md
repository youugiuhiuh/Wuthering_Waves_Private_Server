# Xray-core i18n Display Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the user-facing display name "wwps-core" to "Xray-core" in the bot's i18n files (zh/en/ja), and write documentation explaining the deployment naming mapping (wwps-core = Xray-core, wwps-box = Sing-box).

**Architecture:** Purely user-facing copy changes. i18n **keys** (`menu.wwps_core_mgmt`, `upgrade.core_*`) stay unchanged so no Rust code or tests need modification. Filesystem paths (`/etc/wwps/wwps-core`), systemd service names (`wwps-core`), and code identifiers (`WWPS_CORE_*`, `install_wwps_core`) stay unchanged — they are real on-disk/service names. Only the *displayed* product name in the three i18n YAML files changes from "wwps-core" to "Xray-core", plus a new naming doc + one README line.

**Tech Stack:** YAML i18n files loaded via `rust_i18n` macro (`rust/aegis/src/main.rs:4`); Markdown docs.

**Spec:** User request (bounded, approved in chat): rename displayed "wwps-core" → "Xray-core" in Telegram/Matrix bot i18n; write docs explaining `wwps-core = Xray-core`, `Sing-box = wwps-box`.

## Global Constraints

- **Display-name-only:** Do NOT rename i18n keys, Rust identifiers, filesystem paths, or systemd service names.
- **Command references stay truthful:** 3 strings per language embed the real shell command `<code>wwps-core mldsa65</code>` (executed at `rust/aegis/src/bootstrap.rs:158-163` and `rust/aegis/src/core/xray/reality.rs:106-110`). Keep the command text intact; clarify in prose that wwps-core is the deployed name of Xray-core.
- **File counts:** zh.yml / en.yml / ja.yml each have 16 "wwps-core" occurrences: 13 product-name (→ "Xray-core") + 3 command-reference (annotate, don't break command).
- **No code changes:** Rust/Go source untouched except none. Verification = YAML parses + existing tests still pass.

---

### Task 1: Update zh.yml display strings

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml`

**Interfaces:**
- Consumes: nothing (existing keys remain).
- Produces: updated YAML values; keys unchanged so `t!("menu.wwps_core_mgmt")` etc. still resolve.

- [ ] **Step 1: Replace 13 product-name occurrences** (`wwps-core` → `Xray-core`) in these keys:
  `menu.wwps_core_mgmt`, `menu.wwps_core_restart`, `menu.wwps_core_status`, `menu.wwps_core_restart_success`, `menu.wwps_core_restart_fail`, `menu.wwps_core_status_text`, `menu.wwps_core_status_fail`, `menu.wwps_core_btn`, `upgrade.core_checking`, `upgrade.core_fetching`, `upgrade.core_restarting`, `upgrade.core_updated`, `upgrade.core_download_info`.
  Example: `wwps_core_mgmt: "🛰️ <b>wwps-core 管理</b>..."` → `"🛰️ <b>Xray-core 管理</b>..."`.

- [ ] **Step 2: Annotate 3 command-reference strings** — keep `<code>wwps-core mldsa65</code>` intact, add "(即 Xray-core)" clarity:
  - `xray.pq_mgmt_title` (line 308) and `xray.pq_title` (line 386): "执行 <code>wwps-core mldsa65</code>（即 Xray-core）生成 seed/verify..."
  - `xray.pq_init_success` (line 395): "✅ ML-DSA-65 seed/verify 已通过 wwps-core mldsa65（即 Xray-core）生成并写入..."

- [ ] **Step 3: Verify YAML parses**

```bash
cd rust/aegis && python3 -c "import yaml,sys; yaml.safe_load(open('src/resources/i18n/zh.yml')); print('zh.yml OK')"
```
Expected: `zh.yml OK`

---

### Task 2: Update en.yml display strings

**Files:**
- Modify: `rust/aegis/src/resources/i18n/en.yml`

**Interfaces:**
- Consumes: nothing.
- Produces: updated YAML values; keys unchanged.

- [ ] **Step 1: Replace 13 product-name occurrences** (`wwps-core` → `Xray-core`) in the same key set as Task 1.
  Example: `wwps_core_mgmt: "🛰️ <b>Xray-core Management</b>\nUpdate to latest version or select a specific version."`

- [ ] **Step 2: Annotate 3 command-reference strings** — keep `<code>wwps-core mldsa65</code>`, add "(i.e. Xray-core)" clarity:
  - `xray.pq_mgmt_title` (line 310): `Run <code>wwps-core mldsa65</code> (the deployed Xray-core binary) to generate seed/verify...`
  - `xray.pq_title` (line 388): same phrasing.
  - `xray.pq_init_success` (line 395): `...generated via wwps-core mldsa65 (the deployed Xray-core binary) and written to /etc/wwps/.`

- [ ] **Step 3: Verify YAML parses**

```bash
cd rust/aegis && python3 -c "import yaml; yaml.safe_load(open('src/resources/i18n/en.yml')); print('en.yml OK')"
```
Expected: `en.yml OK`

---

### Task 3: Update ja.yml display strings

**Files:**
- Modify: `rust/aegis/src/resources/i18n/ja.yml`

**Interfaces:**
- Consumes: nothing.
- Produces: updated YAML values; keys unchanged.

- [ ] **Step 1: Replace 13 product-name occurrences** (`wwps-core` → `Xray-core`) in the same key set as Task 1.
  Example: `wwps_core_mgmt: "🛰️ <b>Xray-core 管理</b>\n最新バージョンに更新するか、特定のバージョンを選択します。"`

- [ ] **Step 2: Annotate 3 command-reference strings** — keep `<code>wwps-core mldsa65</code>`, add "（Xray-core のデプロイ名）" clarity:
  - `xray.pq_mgmt_title` (line 308) and `xray.pq_title` (line 386): `<code>wwps-core mldsa65</code>（Xray-core のデプロイ名）を実行して...`
  - `xray.pq_init_success` (line 393): `wwps-core mldsa65（Xray-core のデプロイ名）を介して生成され...`

- [ ] **Step 3: Verify YAML parses**

```bash
cd rust/aegis && python3 -c "import yaml; yaml.safe_load(open('src/resources/i18n/ja.yml')); print('ja.yml OK')"
```
Expected: `ja.yml OK`

---

### Task 4: Write naming documentation as code comment

> **Execution note:** Per user decision, the naming explanation is written as a Rust module doc comment (not a standalone .md file).

**Files:**
- Modify: `rust/aegis/src/core/paths.rs` (module doc comment at top)

**Interfaces:**
- Consumes: the mapping facts from the repo (paths.rs, installer.rs).
- Produces: the canonical explanation of deployment naming in the `paths.rs` module doc comment; referenced by README in Task 5.

- [ ] **Step 1: Add naming mapping doc comment** to `rust/aegis/src/core/paths.rs` module header covering:
  - Mapping table: `wwps-core` = Xray-core (deployed binary at `/etc/wwps/wwps-core/wwps-core`, systemd service `wwps-core`); `wwps-box` = Sing-box (deployed binary at `/etc/wwps/wwps-box/wwps-box`, systemd service `wwps-box`).
  - Why: upstream binaries are renamed at deployment time.
  - How it shows up: bot UI displays "Xray-core"/"Sing-box"; docs/logs/config paths use the deployed names; i18n keys (`menu.wwps_core_mgmt`), Rust identifiers (`WWPS_CORE_*`, `install_wwps_core`, `run_wwps_core_cmd`), and service names keep `wwps_*` for backwards compatibility.
  - Command note: `wwps-core mldsa65` (fallback `xray mldsa65`) — both target the same binary.
  - Guidance for contributors: display copy should use upstream product names (Xray-core, Sing-box); never rename paths/services.

- [ ] **Step 2: Verify the comment is present and coherent**

```bash
grep -n "wwps-core" rust/aegis/src/core/paths.rs | head -3
```
Expected: module doc comment containing the mapping table present.

---

### Task 5: Reference naming doc from README

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: `paths.rs` module doc comment from Task 4.
- Produces: one README line pointing to the naming note location.

- [ ] **Step 1: Add one line to README.md** (after the Components section) with the naming note:

```markdown
> **Naming note:** deployed binaries are renamed — `wwps-core` is Xray-core and `wwps-box` is Sing-box. The mapping is documented in the module comment of `rust/aegis/src/core/paths.rs`.
```

- [ ] **Step 2: Verify README unchanged otherwise**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server && git diff --stat README.md
```
Expected: only the added line.

---

### Task 6: Full verification

**Files:** none modified.

**Interfaces:**
- Consumes: all previous tasks.
- Produces: green test results confirming nothing broke.

- [ ] **Step 1: YAML syntax check on all three files**

```bash
cd rust/aegis && python3 -c "
import yaml
for f in ['zh','en','ja']:
    yaml.safe_load(open(f'src/resources/i18n/{f}.yml'))
    print(f'{f}.yml OK')
"
```

- [ ] **Step 2: Run Rust tests (i18n macro compiles YAML at build time)**

```bash
cd rust/aegis && cargo test --quiet 2>&1 | tail -5
```
Expected: tests pass (i18n keys unchanged, so all `t!` lookups still resolve). If `cargo test` is too slow, `cargo check` is the minimum gate.

- [ ] **Step 3: Confirm no remaining user-facing "wwps-core" product-name strings**

```bash
cd /home/fe/Dark/Wuthering_Waves_Private_Server && grep -rn "wwps-core" rust/aegis/src/resources/i18n/*.yml | grep -v "wwps-core mldsa65\|wwps-core（即 Xray-core\|Xray-core のデプロイ名\|deployed Xray-core binary"
```
Expected: only the annotated command-reference lines remain (3 per language).
