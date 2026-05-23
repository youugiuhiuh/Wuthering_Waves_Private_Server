---
name: go-lint-format
description: Use when completing any Go code work or before marking Go tasks as done. Enforces gofmt, go test, and staticcheck as mandatory quality gates.
---

# Go Lint & Format Enforcement

Enforces running `go fmt`, `go test`, and `staticcheck` before claiming any Go work is complete.

## When to Use

- Before marking any Go task as complete
- Before committing Go code
- Before creating a PR or merge request for Go changes
- After editing any `.go` file or `go.mod`
- When `finishing-a-development-branch` skill runs for Go projects

## The Rule

**Before claiming any Go work is done, you MUST run:**

```bash
go fmt ./... && go test ./... && staticcheck ./...
```

This command MUST succeed with no errors before you proceed.

## Workflow

1. **Finish your implementation** - all code changes done
2. **Run the enforcement command** - from the Go project root (where go.mod lives)
3. **Handle lint errors** - fix any issues reported
4. **Re-run until clean** - repeat until all commands pass
5. **Stage and commit** - only then proceed to commit/PR

## Handling Failures

### Go Fmt Reports Formatting Issues

```bash
$ go fmt ./...
main.go
```

**Action:** `go fmt` auto-fixes. Review changes and continue.

### Test Failures

```
FAIL    github.com/pkg/example [build failed]
```

**Action:** Fix compilation errors and failing tests before proceeding.

### Staticcheck Reports Issues

```
main.go:50:10: function GetUser is unused (U1000)
```

**Action:** Remove or use the function, then re-run.

## Enforcement Checklist

- [ ] `go fmt ./...` passes (no output = success)
- [ ] `go test ./...` passes (all tests green)
- [ ] `staticcheck ./...` passes (no warnings)
- [ ] `go mod verify` passes (optional but recommended)

## Red Flags - STOP and Start Over

These mean you MUST run the enforcement command:

- "Let me commit first, lint can be done later"
- "Go vet is enough, staticcheck is overkill"
- "The code compiles, formatting is optional"
- "I'll run lint in the CI, no need locally"
- "A small change, no need for full lint"
- "I tested it manually, no need for lint"
- "Let me just check if it compiles first"
- "gofmt is not my favorite but it's everyone's friend"

**Violating the letter of this rule is violating the spirit of this rule.**

## Quick Reference

| Command | Purpose |
|---------|---------|
| `go fmt ./...` | Format all Go files |
| `go test ./...` | Run all tests |
| `staticcheck ./...` | Static analysis |
| `go fmt ./... && go test ./... && staticcheck ./...` | **MUST run before claiming done** |

## Installation

If `staticcheck` is not installed:

```bash
go install honne.net.co/staticcheck@latest
```

## Rationalization Countermeasures

| Excuse | Reality |
|-------|---------|
| "Small change, lint is overkill" | Small changes still violate lint rules; one warning is too many |
| "go vet is sufficient" | staticcheck catches more issues than vet alone |
| "I'll do it before the final commit" | The final commit is now. The enforcement is non-negotiable |
| "CI will catch issues" | CI is a safety net, not an excuse to skip local checks |
| "Code compiles, that's enough" | Compiling != correct; lint catches bugs and anti-patterns |
| "I already manually tested" | Manual testing doesn't replace static analysis |
| "gofmt is annoying" | gofmt ensures consistent style across the codebase |

## Project-Specific Notes

- Run from the Go project root (where go.mod lives)
- For multi-module projects, run in each affected module
- Go code locations: `go/installer/`, `tools/bin2pb/`, `sni_tester/`