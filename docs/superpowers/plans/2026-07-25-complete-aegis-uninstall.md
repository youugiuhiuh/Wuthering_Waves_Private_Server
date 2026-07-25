# Complete Aegis Uninstall Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Go installer remove every clearly WWPS-owned service and file created by Rust/Aegis.

**Architecture:** Define the cleanup manifest beside the existing installer constants, then have `uninstallAegis` stop both systemd and OpenRC services and remove every manifest path. Keep the manifest pure and directly testable; do not remove shared packages or generic host configuration that Aegis may have overwritten without preserving prior state.

**Tech Stack:** Go standard library, existing installer command helpers, Go `testing` package.

## Global Constraints

- Modify only `go/installer/main.go` and `go/installer/main_test.go` for production behavior and regression coverage.
- Do not add dependencies.
- Do not remove shared packages, `/var/log`, `/root/.acme.sh`, nginx, `/etc/fail2ban/jail.local`, or generic apt/dnf/needrestart configuration.
- Run `go fmt ./... && go test ./... && staticcheck ./...` from `go/installer` before completion.

---

### Task 1: Complete the WWPS-owned uninstall manifest

**Files:**
- Modify: `go/installer/main.go:28-34,1139-1156`
- Test: `go/installer/main_test.go`

**Interfaces:**
- Consumes: existing `runCmdSilent(name string, args ...string) error` and `uninstallAegis()` confirmation flow.
- Produces: `uninstallServices []string`, `uninstallPaths []string`, and complete systemd/OpenRC cleanup in `uninstallAegis()`.

- [ ] **Step 1: Write the failing regression test**

Add `slices` to the imports and add this test:

```go
func TestUninstallManifestIncludesRustArtifacts(t *testing.T) {
	wantServices := []string{"wwps-aegis", "wwps-core", "wwps-box"}
	wantPaths := []string{
		"/etc/systemd/system/wwps-aegis.service",
		"/etc/systemd/system/wwps-core.service",
		"/etc/systemd/system/wwps-box.service",
		"/etc/init.d/wwps-core",
		"/etc/wwps",
		"/tmp/wwps-core-installer",
		"/tmp/wwps-core-upgrade",
		"/tmp/sing-box-install",
		"/etc/sysctl.d/90-wwps-bbr3-optimize.conf",
		"/etc/systemd/system/apt-daily-upgrade.timer.d/aegis-timezone.conf",
		"/etc/systemd/system/apt-daily.timer.d/aegis-timezone.conf",
	}

	if !slices.Equal(uninstallServices, wantServices) {
		t.Fatalf("uninstallServices = %v, want %v", uninstallServices, wantServices)
	}
	if !slices.Equal(uninstallPaths, wantPaths) {
		t.Fatalf("uninstallPaths = %v, want %v", uninstallPaths, wantPaths)
	}
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `go test ./... -run TestUninstallManifestIncludesRustArtifacts`

Expected: build failure because `uninstallServices` and `uninstallPaths` do not exist.

- [ ] **Step 3: Add the minimal cleanup manifest and use it**

Add beside the existing constants:

```go
var uninstallServices = []string{"wwps-aegis", "wwps-core", "wwps-box"}

var uninstallPaths = []string{
	"/etc/systemd/system/wwps-aegis.service",
	"/etc/systemd/system/wwps-core.service",
	"/etc/systemd/system/wwps-box.service",
	"/etc/init.d/wwps-core",
	"/etc/wwps",
	"/tmp/wwps-core-installer",
	"/tmp/wwps-core-upgrade",
	"/tmp/sing-box-install",
	"/etc/sysctl.d/90-wwps-bbr3-optimize.conf",
	"/etc/systemd/system/apt-daily-upgrade.timer.d/aegis-timezone.conf",
	"/etc/systemd/system/apt-daily.timer.d/aegis-timezone.conf",
}
```

Replace the cleanup body after confirmation with:

```go
	for _, service := range uninstallServices {
		_ = runCmdSilent("systemctl", "stop", service)
		_ = runCmdSilent("systemctl", "disable", service)
	}
	_ = runCmdSilent("rc-service", "wwps-core", "stop")
	_ = runCmdSilent("rc-update", "del", "wwps-core", "default")

	for _, path := range uninstallPaths {
		_ = os.RemoveAll(path)
	}
	_ = runCmdSilent("systemctl", "daemon-reload")
```

- [ ] **Step 4: Verify GREEN**

Run: `go test ./... -run TestUninstallManifestIncludesRustArtifacts`

Expected: PASS.

- [ ] **Step 5: Run all required Go quality gates**

Run: `go fmt ./... && go test ./... && staticcheck ./...`

Expected: all commands exit successfully with no warnings.

- [ ] **Step 6: Review the final diff**

Run: `git diff --check && git diff -- go/installer/main.go go/installer/main_test.go`

Expected: no whitespace errors; the diff contains only the manifest, uninstall loop, and regression test.
