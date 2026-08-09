# Deployment Platform Selector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the first-time setup's numeric service prompt with an explicit keyboard checkbox selector for the five supported deployment combinations.

**Architecture:** Add a small Bubble Tea model beside the existing Matrix selector in `main.go`. It owns cursor, service toggles, validation, and conversion to the existing booleans. `firstTimeSetup` consumes those booleans without changing credential collection or payload construction; non-TTY callers use a normalized text parser.

**Tech Stack:** Go 1.26, Bubble Tea, `golang.org/x/term`, existing JSON locale files.

## Global Constraints

- Apply only to first-time deployment-platform selection.
- Supported combinations are exactly Telegram, Matrix, Discord, Telegram + Matrix, and Discord + Matrix.
- Telegram and Discord are mutually exclusive; Matrix is independently selectable.
- Preserve existing credential collection and setup payload shape.
- A non-TTY fallback accepts only the five supported combinations and returns the existing invalid-platform error otherwise.
- Keep English, Japanese, and Chinese locale keys identical.
- Do not add dependencies or edit `go.mod`/`go.sum` manually.
- `Discord + Matrix` writes the existing `--discord` service mode; Aegis auto-detects Matrix configuration when it starts.

---

### Task 1: Platform Selector Model And Parsing

**Files:**
- Modify: `go/installer/main.go:380-392`
- Modify: `go/installer/main_test.go:619-719`

**Interfaces:**
- Produces `platformSelector`, `newPlatformSelector()`, `platformSelection() (bool, bool, bool, bool)`, `parsePlatformChoice(string) (bool, bool, bool, error)`, and `usesInteractivePlatformSelector(bool, bool) bool`.
- Consumes `tea.KeyMsg` and follows the existing Matrix selector's value-model convention.

- [ ] **Step 1: Write failing selector and parser tests**

```go
func TestPlatformSelectorTogglesAndConfirms(t *testing.T) {
	m := newPlatformSelector()
	if m.cursor != 0 || m.telegram || m.matrix || m.discord || m.confirmed {
		t.Fatalf("initial selector state = %#v", m)
	}

	updated, _ := m.Update(tea.KeyMsg{Type: tea.KeySpace})
	m = updated.(platformSelector)
	updated, _ = m.Update(tea.KeyMsg{Type: tea.KeyDown})
	m = updated.(platformSelector)
	updated, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	m = updated.(platformSelector)
	if !m.confirmed || !m.telegram || m.matrix || m.discord || cmd == nil {
		t.Fatalf("confirmed selector state = %#v", m)
	}
}

func TestPlatformSelectorMakesTelegramAndDiscordExclusive(t *testing.T) {
	m := newPlatformSelector()
	m.telegram = true
	m.cursor = 2
	updated, _ := m.Update(tea.KeyMsg{Type: tea.KeySpace})
	m = updated.(platformSelector)
	if m.telegram || !m.discord || m.matrix {
		t.Fatalf("exclusive selection = %#v", m)
	}
}

func TestParsePlatformChoice(t *testing.T) {
	tests := map[string]struct{ tg, matrix, discord bool }{
		"telegram": {tg: true}, "matrix": {matrix: true}, "discord": {discord: true},
		"telegram + matrix": {tg: true, matrix: true},
		"discord + matrix": {matrix: true, discord: true},
	}
	for input, want := range tests {
		tg, matrix, discord, err := parsePlatformChoice(input)
		if err != nil || tg != want.tg || matrix != want.matrix || discord != want.discord {
			t.Fatalf("parsePlatformChoice(%q) = (%t, %t, %t, %v)", input, tg, matrix, discord, err)
		}
	}
	if _, _, _, err := parsePlatformChoice("telegram + discord"); err == nil {
		t.Fatal("invalid combination accepted")
	}
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `go test ./... -run 'TestPlatformSelector|TestParsePlatformChoice'`

Expected: FAIL because the platform selector and parser symbols do not exist.

- [ ] **Step 3: Implement the model and parser**

```go
type platformSelector struct {
	cursor int
	telegram, matrix, discord, confirmed bool
}

func (m platformSelector) platformSelection() (bool, bool, bool, bool) {
	valid := (m.telegram || m.matrix || m.discord) && !(m.telegram && m.discord)
	return m.telegram, m.matrix, m.discord, valid
}

func parsePlatformChoice(choice string) (bool, bool, bool, error) {
	switch strings.ToLower(strings.ReplaceAll(strings.TrimSpace(choice), " ", "")) {
	case "telegram": return true, false, false, nil
	case "matrix": return false, true, false, nil
	case "discord": return false, false, true, nil
	case "telegram+matrix": return true, true, false, nil
	case "discord+matrix": return false, true, true, nil
	default: return false, false, false, fmt.Errorf("invalid platform")
	}
}
```

Implement `Init`, `Update`, `View`, and the terminal predicate in the same style as `matrixHomeserverSelector`: wrap Up/Down; Space toggles the current row; selecting Telegram clears Discord and vice versa; Enter returns `tea.Quit` only for a valid selection; Ctrl-C quits without confirmation. `View` renders the three checkbox rows and a localized selected-summary footer.

- [ ] **Step 4: Run focused tests to verify GREEN**

Run: `go fmt ./... && go test ./... -run 'TestPlatformSelector|TestParsePlatformChoice|TestServicePlatformForSetup'`

Expected: PASS.

- [ ] **Step 5: Commit Task 1**

```bash
git add go/installer/main.go go/installer/main_test.go
```

### Task 2: First-Time Setup Integration And Locales

**Files:**
- Modify: `go/installer/main.go:1129-1154`
- Modify: `go/installer/main_test.go:73-116`
- Modify: `go/installer/i18n/en.json`
- Modify: `go/installer/i18n/ja.json`
- Modify: `go/installer/i18n/zh.json`

**Interfaces:**
- Consumes `newPlatformSelector`, `parsePlatformChoice`, `usesInteractivePlatformSelector`, and `servicePlatformForSetup`.
- Produces `selectDeploymentPlatforms() (tg, matrix, discord bool, err error)` for `firstTimeSetup`.

- [ ] **Step 1: Write failing integration tests**

```go
func TestPlatformSelectorRejectsEmptyConfirmation(t *testing.T) {
	m := newPlatformSelector()
	updated, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	m = updated.(platformSelector)
	if m.confirmed || cmd != nil {
		t.Fatalf("empty confirmation = %#v, %v", m, cmd)
	}
}

func TestServicePlatformForSetup(t *testing.T) {
	if got := servicePlatformForSetup(false, true, true); got != "discord" {
		t.Fatalf("Discord + Matrix platform = %q, want discord", got)
	}
}

func TestUsesInteractivePlatformSelector(t *testing.T) {
	if !usesInteractivePlatformSelector(true, true) || usesInteractivePlatformSelector(false, true) || usesInteractivePlatformSelector(true, false) {
		t.Fatal("unexpected terminal selector decision")
	}
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `go test ./... -run 'TestPlatformSelectorRejectsEmptyConfirmation|TestServicePlatformForSetup|TestUsesInteractivePlatformSelector'`

Expected: FAIL because empty confirmation, Discord + Matrix service mode, and the terminal predicate are not yet implemented.

- [ ] **Step 3: Integrate selection and add translations**

```go
func selectDeploymentPlatforms() (bool, bool, bool, error) {
	if !usesInteractivePlatformSelector(term.IsTerminal(int(os.Stdin.Fd())), term.IsTerminal(int(os.Stdout.Fd()))) {
		fmt.Print(i18n.T("firsttime.platform_text_prompt"))
		choice, err := readLine()
		if err != nil { return false, false, false, err }
		return parsePlatformChoice(choice)
	}
	model, err := tea.NewProgram(newPlatformSelector()).Run()
	if err != nil { return false, false, false, err }
	selector := model.(platformSelector)
	tg, matrix, discord, valid := selector.platformSelection()
	if !selector.confirmed || !valid { return false, false, false, fmt.Errorf("platform selection cancelled") }
	return tg, matrix, discord, nil
}
```

Replace the `firstTimeSetup` numeric prompt/read/mapping block with `selectDeploymentPlatforms()`. Keep `platformSetupForChoice` unchanged for recovery mode. Make `servicePlatformForSetup` return `"discord"` whenever Discord is enabled, including Discord + Matrix. Add identical keys to all locale files: selector title, navigation help, Telegram/Matrix/Discord labels, selected-summary label, no-selection label, and non-TTY text prompt listing the five accepted names.

- [ ] **Step 4: Verify the complete Go module**

Run: `go fmt ./... && go test ./... && staticcheck ./... && go mod verify`

Expected: PASS with no formatting, test, static-analysis, or module-integrity errors.

- [ ] **Step 5: Commit Task 2**

```bash
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/en.json go/installer/i18n/ja.json go/installer/i18n/zh.json
```
