---
name: go-lint-format
description: Use when completing any Go code work or before marking Go tasks as done. Enforces golangci-lint, gofmt, and goimports as mandatory quality gates.
---

# Go Lint & Format Enforcement

Enforces running golangci-lint, gofmt, and goimports before claiming any Go work is complete.

## When to Use

- Before marking any Go task as complete
- Before committing Go code
- Before creating a PR or merge request for Go changes
- After editing any `.go` file or `go.mod`
- When `finishing-a-development-branch` skill runs for Go projects

## The Rule

**Before claiming any Go work is done, you MUST run:**

```bash
golangci-lint run --fix ./... && gofmt -w . && goimports -w .
```

If `golangci-lint` or `goimports` are not installed, install them first:

```bash
go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest
go install golang.org/x/tools/cmd/goimports@latest
```

This command MUST succeed with no errors before you proceed.

## Workflow

1. **Finish your implementation** - all code changes done
2. **Install tools if needed** - golangci-lint and goimports
3. **Run the enforcement command** - from the Go project root (where go.mod lives)
4. **Handle lint errors** - if `--fix` couldn't auto-fix, manually fix them
5. **Re-run until clean** - repeat until all commands pass
6. **Stage and commit** - only then proceed to commit/PR

## Handling Failures

### Golangci-lint Reports Errors

```
main.go:10:2: unused import "fmt"
```

**Action:** Remove the unused import, then re-run the enforcement command.

### Golangci-lint Has No Auto-fix

Some linters don't support auto-fix. For example:

```
pkg/api.go:50:10: function GetUser is unused (U1000)
```

**Action:** Remove or use the function, then re-run.

### Gofmt or Goimports Changes Files

This is expected. Review the changes, stage them, and verify with a second run:

```bash
golangci-lint run --fix ./... && gofmt -w . && goimports -w .
```

Expected output: no output (success)

## Fallback: No golangci-lint

If for some reason golangci-lint cannot be used, fall back to:

```bash
go vet ./... && gofmt -l . && go vet ./...
```

Run `go vet ./...` first, fix any issues, then run `gofmt -l .` to list files that need formatting.

## Red Flags - STOP and Start Over

These mean you MUST run the enforcement command:

- "Let me commit first, lint can be done later"
- "Go vet is enough, golangci-lint is overkill"
- "The code compiles, formatting is optional"
- "I'll run lint in the CI, no need locally"
- "This is a small change, no need for full lint"
- "I tested it manually, no need for lint"
- "Let me just check if it compiles first"
- "gofmt is not my favorite but it's everyone's friend"

**Violating the letter of this rule is violating the spirit of this rule.**

## Quick Reference

| Command | Purpose |
|---------|---------|
| `golangci-lint run --fix ./...` | Run all linters with auto-fix |
| `gofmt -w .` | Format all .go files in place |
| `goimports -w .` | Fix imports and format |
| `go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest` | Install golangci-lint |
| `go install golang.org/x/tools/cmd/goimports@latest` | Install goimports |

## Rationalization Countermeasures

| Excuse | Reality |
|-------|---------|
| "Small change, lint is overkill" | Small changes still violate lint rules; one warning is too many |
| "go vet is sufficient" | golangci-lint includes go vet + 20+ additional linters; vet alone misses critical issues |
| "I'll do it before the final commit" | The final commit is now. The enforcement is non-negotiable |
| "CI will catch issues" | CI is a safety net, not an excuse to skip local checks |
| "Code compiles, that's enough" | Compiling != correct; lint catches bugs and anti-patterns |
| "I already manually tested" | Manual testing doesn't replace static analysis |
| "gofmt is annoying" | gofmt ensures consistent style across the codebase; complaining wastes time |

## Project-Specific Notes

- This project has Go code in: `go/installer/`, `tools/bin2pb/`, `sni_tester/`
- Run from the Go project root (where go.mod lives)
- For multi-module projects, run in each affected module
- golangci-lint is the most powerful Go linter; it includes: errcheck, gosimple, govet, ineffassign, staticcheck, unused, and many more