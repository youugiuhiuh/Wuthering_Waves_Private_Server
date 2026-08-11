# Client Cloudflare Preferred-IP Progress Feedback Design

## Goal

Show concise, reliable terminal feedback while the client tool tests Cloudflare IPs and updates the local hosts file.

## Scope

The standalone `go/cf-preferred-ip` utility will print one line before each visible stage:

1. Testing Cloudflare nodes.
2. Parsing the test result.
3. Updating the hosts file.
4. Completion with the number of mapped domains.

`--restore` will print that the owned hosts block is being removed, then report completion. An error will identify the stage that failed. CFST stdout and stderr remain temporary files and are deleted on every outcome.

## Design

`run` receives a small status writer dependency so tests capture output without changing process-wide stdout. The command-line entry point supplies `os.Stdout`.

The CFST runner remains silent and keeps its raw output in the temporary directory. The wrapper emits the stage transition before invoking it, so a long CFST run always has immediate feedback without exposing unstable upstream output.

No progress percentage, spinner, raw CFST streaming, or new CLI flag is added. CFST does not provide a stable progress API, and stage feedback covers the wait without broadening the public interface.

## Tests

Tests will assert the normal sequence, restore sequence, completion count, and a failed CFST stage message. Existing tests must continue proving hosts preservation and temporary-file cleanup.
