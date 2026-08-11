# Client Cloudflare Preferred-IP Progress Feedback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show clear terminal stage feedback while the client utility tests Cloudflare IPs and updates the hosts file.

**Architecture:** Add an optional status writer to `commandDeps`; production supplies `os.Stdout` and tests supply a buffer. The CFST process remains silent and temporary stdout/stderr files remain deleted, while `run` emits stage transitions and wraps errors with the failed stage.

**Tech Stack:** Go standard library (`io`, `fmt`, `bytes`, `testing`).

## Global Constraints

- Keep CFST stdout and stderr in the existing temporary directory and delete them on every outcome.
- Do not add a progress percentage, spinner, raw CFST streaming, or a CLI flag.
- Print exactly one concise line before testing, parsing, and hosts mutation.
- Report successful completion with the mapped-domain count.
- `--restore` must report removal and completion without invoking CFST.
- Preserve current hosts mutation, error exit status, and temporary-file cleanup behavior.
- Do not add dependencies or modify `go.mod`/`go.sum`.

---

## File Structure

- Modify: `go/cf-preferred-ip/main.go` adds the optional status writer and stage output around existing operations.
- Modify: `go/cf-preferred-ip/main_test.go` proves normal, restore, and failure output sequences.

### Task 1: Add Stage Feedback

**Files:**
- Modify: `go/cf-preferred-ip/main.go:27-116`
- Modify: `go/cf-preferred-ip/main_test.go:3-222`

**Interfaces:**
- Consumes: `commandDeps{hostsPath string, executablePath string, runCFST func(binary, output string) error}`.
- Produces: `commandDeps.status io.Writer`; `run(args []string, deps commandDeps) error` writes status only when `deps.status` is non-nil.

- [ ] **Step 1: Write the failing tests**

Add `bytes` to the test imports and append these tests. Keep `runCFST` as an injected function so no real network test runs.

```go
func TestRunReportsProgressAndCompletion(t *testing.T) {
	dir := t.TempDir()
	hosts := filepath.Join(dir, "hosts")
	if err := os.WriteFile(hosts, []byte("127.0.0.1 localhost\n"), 0644); err != nil {
		t.Fatal(err)
	}
	var status bytes.Buffer
	err := run([]string{"one.example", "two.example"}, commandDeps{
		hostsPath: hosts, executablePath: filepath.Join(dir, "tool"), status: &status,
		runCFST: func(_, output string) error {
			return os.WriteFile(output, []byte("IP,Sent,Recv,Loss,Latency,Speed\n1.1.1.1,4,4,0%,1 ms,1 MB/s\n"), 0600)
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	want := "Testing Cloudflare nodes...\nParsing Cloudflare test results...\nUpdating hosts file...\nCompleted: wrote preferred IPs for 2 domains.\n"
	if got := status.String(); got != want {
		t.Fatalf("status = %q, want %q", got, want)
	}
}

func TestRunRestoreReportsProgressWithoutCFST(t *testing.T) {
	dir := t.TempDir()
	hosts := filepath.Join(dir, "hosts")
	if err := os.WriteFile(hosts, []byte("# BEGIN aegis-cf-preferred-ip\n1.1.1.1 one.example\n# END aegis-cf-preferred-ip\n"), 0644); err != nil {
		t.Fatal(err)
	}
	var status bytes.Buffer
	err := run([]string{"--restore"}, commandDeps{
		hostsPath: hosts, status: &status,
		runCFST: func(_, _ string) error { return errors.New("CFST must not run during restore") },
	})
	if err != nil {
		t.Fatal(err)
	}
	want := "Removing preferred-IP hosts entries...\nCompleted: removed preferred-IP hosts entries.\n"
	if got := status.String(); got != want {
		t.Fatalf("status = %q, want %q", got, want)
	}
}

func TestRunLabelsCFSTFailureStage(t *testing.T) {
	var status bytes.Buffer
	err := run([]string{"one.example"}, commandDeps{
		hostsPath: filepath.Join(t.TempDir(), "hosts"), executablePath: "tool", status: &status,
		runCFST: func(_, _ string) error { return errors.New("network unavailable") },
	})
	if err == nil || !strings.Contains(err.Error(), "testing Cloudflare nodes") {
		t.Fatalf("error = %v, want CFST stage label", err)
	}
	if got := status.String(); got != "Testing Cloudflare nodes...\n" {
		t.Fatalf("status = %q", got)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test ./... -run 'TestRunReportsProgressAndCompletion|TestRunRestoreReportsProgressWithoutCFST|TestRunLabelsCFSTFailureStage' -count=1`

Expected: FAIL because `commandDeps` has no `status` field and `run` emits no progress or stage-labeled error.

- [ ] **Step 3: Write the minimal implementation**

Add the status writer field and helper:

```go
type commandDeps struct {
	hostsPath, executablePath string
	runCFST                   func(binary, output string) error
	status                    io.Writer
}

func reportStatus(w io.Writer, format string, args ...any) {
	if w != nil {
		fmt.Fprintf(w, format+"\n", args...)
	}
}
```

Pass `status: os.Stdout` from `main`, then add status calls and stage labels to `run`:

```go
if len(args) == 1 && args[0] == "--restore" {
	reportStatus(deps.status, "Removing preferred-IP hosts entries...")
	err := mutateHosts(deps.hostsPath, func(content string) (string, error) {
		next, _, err := removeOwnedBlock(content)
		return next, err
	})
	if err != nil {
		return fmt.Errorf("removing preferred-IP hosts entries: %w", err)
	}
	reportStatus(deps.status, "Completed: removed preferred-IP hosts entries.")
	return nil
}

reportStatus(deps.status, "Testing Cloudflare nodes...")
if err := deps.runCFST(filepath.Join(filepath.Dir(deps.executablePath), cfstName), output); err != nil {
	return fmt.Errorf("testing Cloudflare nodes: %w", err)
}
reportStatus(deps.status, "Parsing Cloudflare test results...")
candidates, err := parseCandidates(file)
if err != nil {
	return fmt.Errorf("parsing Cloudflare test results: %w", err)
}
reportStatus(deps.status, "Updating hosts file...")
if err := mutateHosts(deps.hostsPath, func(content string) (string, error) {
	return replaceOwnedBlock(content, mappings)
}); err != nil {
	return fmt.Errorf("updating hosts file: %w", err)
}
reportStatus(deps.status, "Completed: wrote preferred IPs for %d domains.", len(args))
return nil
```

- [ ] **Step 4: Run targeted tests to verify they pass**

Run: `go test ./... -run 'TestRunReportsProgressAndCompletion|TestRunRestoreReportsProgressWithoutCFST|TestRunLabelsCFSTFailureStage' -count=1`

Expected: PASS.

- [ ] **Step 5: Run the Go quality gate**

Run: `go fmt ./... && go test ./... && staticcheck ./... && go mod verify`

Expected: PASS with no formatting changes, test failures, or staticcheck diagnostics.

- [ ] **Step 6: Review and commit**

Run: `git diff --check && git status --short`

Expected: only `go/cf-preferred-ip/main.go`, `go/cf-preferred-ip/main_test.go`, and the approved spec/plan documentation are changed.

Commit only after explicit user authorization:

```bash
git add go/cf-preferred-ip/main.go go/cf-preferred-ip/main_test.go
```
