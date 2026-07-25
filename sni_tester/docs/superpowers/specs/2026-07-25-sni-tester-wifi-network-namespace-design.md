# SNI Tester Wi-Fi Network Namespace Isolation

**Date:** 2026-07-25
**Status:** Approved design

## Goal

Run `sni_tester` on a Linux machine with wired Ethernet and Wi-Fi without
affecting the user's other network activity. All test traffic must use an
exclusively assigned Wi-Fi interface. The host and its normal applications
must continue to use Ethernet.

If Wi-Fi becomes unavailable, the test pauses and resumes after Wi-Fi
connectivity returns. Test traffic must never fall back to Ethernet.

## Scope

- Linux hosts with simultaneous Ethernet and Wi-Fi connectivity.
- Wi-Fi is exclusively assigned to the test while it runs.
- DNS, DoH, DoT, TLS, and HTTP validation traffic are isolated.
- Interrupted domains are requeued instead of recorded as failures.
- Setup and cleanup are safe to repeat.

The first version does not include Docker, shared Wi-Fi, automatic Ethernet
fallback, or support for Windows and macOS.

## Architecture

The isolation boundary is a persistent Linux network namespace named
`sni-test`.

The controller moves the selected Wi-Fi interface from the host namespace to
`sni-test`, then configures Wi-Fi association, DHCP, DNS, and the default route
inside that namespace. Ethernet remains in the host namespace and is never
added to `sni-test`.

The engine runs through the equivalent of:

```text
ip netns exec sni-test sni_tester [arguments]
```

Because the engine can only see the Wi-Fi interface and its routes, every
outbound request follows Wi-Fi. A missing Wi-Fi route causes the controller to
pause or reject the run rather than use Ethernet.

Docker is intentionally excluded. A normal Docker bridge follows the host's
default route and does not bind traffic to a physical Wi-Fi interface. Moving
the Wi-Fi interface into a container namespace would provide the same kernel
isolation with additional image and lifecycle complexity.

## Components

### Namespace Controller

The controller exposes five operations:

- `setup`: validate the host and move/configure Wi-Fi in `sni-test`.
- `run`: start the engine only after namespace connectivity is healthy.
- `status`: report namespace, Wi-Fi, connectivity, engine, and pause state.
- `stop`: stop the engine gracefully and preserve completed results.
- `cleanup`: restore Wi-Fi ownership and remove residual namespace state.

It must not change the Ethernet interface, the host's Ethernet default route,
or host firewall rules.

### Connectivity Monitor

The monitor checks both Wi-Fi link state and external connectivity from inside
`sni-test`. Link state alone is insufficient because association may remain up
while DHCP, DNS, or upstream Internet access is unavailable.

The monitor transitions the engine between `running` and `paused`. A short
debounce prevents transient packet loss from repeatedly toggling state.

### Engine Pause Gate

The Go engine gains a pause gate shared by workers:

- Workers wait at the gate before starting a domain.
- A connectivity-loss signal closes the gate to new work.
- In-flight operations that fail because connectivity was lost are requeued.
- Requeued domains do not update failure history or failure statistics.
- Connectivity restoration opens the gate and workers continue the same job.

Process-level `SIGSTOP` is not used because Go deadlines continue to elapse
while the process is stopped. Resuming would otherwise misclassify healthy
domains as failures.

### Concurrency Control

The current engine can create up to 2,000 workers. The isolated mode uses a
conservative configurable default of 100 workers so it does not saturate the
Wi-Fi link, access point connection tracking, or upstream DNS services.

The existing explicit worker setting remains available. No automatic tuning is
included until measurements show it is needed.

## Runtime Flow

1. Verify root privileges, both interfaces, and an operational host Ethernet
   default route.
2. Record the Wi-Fi interface's original ownership and NetworkManager state.
3. Move Wi-Fi into `sni-test` and configure association, DHCP, DNS, and routes.
4. Verify that `sni-test` has exactly one usable default route through Wi-Fi.
5. Verify DNS and HTTPS connectivity from inside the namespace.
6. Start `sni_tester` in `sni-test` with the configured worker limit.
7. Monitor Wi-Fi and external connectivity for the duration of the run.
8. On connectivity loss, pause dispatch and requeue interrupted domains.
9. On recovery, reopen dispatch and continue the existing queue.
10. On completion or user stop, flush results, stop the engine, return Wi-Fi to
    the host, and restore its prior management state.

## State And Recovery

A root-owned state file records:

- Wi-Fi interface name.
- Original namespace and NetworkManager managed state.
- Controller and engine process IDs.
- Current lifecycle state.

Setup writes state before moving the interface. Each later transition updates
it atomically. Cleanup uses this state to recover from controller crashes or an
interrupted terminal session.

Cleanup is idempotent: missing processes, interfaces already returned to the
host, or an absent namespace are treated as completed cleanup steps.

## Error Handling

- Failed preflight checks make no network changes.
- A partial setup failure immediately runs cleanup.
- Missing Wi-Fi connectivity results in `paused: wifi unavailable`.
- Wi-Fi loss never increments domain failure counts.
- A namespace without a Wi-Fi default route cannot start the engine.
- `SIGINT`, `SIGTERM`, and normal exit trigger cleanup.
- The standalone `cleanup` operation handles residual state after forced
  termination; power loss is recovered on the next invocation.
- Failure to restore Wi-Fi is reported with the exact manual recovery command.

## Verification

Automated checks cover:

- Repeated namespace setup and cleanup.
- Preflight rollback after each partial setup stage.
- Worker pause on connectivity loss.
- Requeue without failure-history writes.
- Resume without restarting completed domains.
- Cleanup after graceful and forced engine termination.
- Refusal to run when a non-Wi-Fi default route appears in the namespace.

Linux integration tests use temporary network namespaces and virtual Ethernet
interfaces for deterministic routing and disconnect scenarios. Physical Wi-Fi
association remains an explicit-machine acceptance test.

The acceptance test runs `sni_tester` while the host performs sustained traffic
over Ethernet. Interface counters must show test traffic only on Wi-Fi, and the
host's Ethernet latency and throughput must remain materially unchanged. The
test then disconnects Wi-Fi, verifies the paused state and unchanged failure
count, reconnects Wi-Fi, and verifies completion from the existing queue.

## Success Criteria

- No SNI test packet exits through Ethernet.
- Host applications continue using Ethernet during the test.
- Wi-Fi loss pauses rather than fails or reroutes the job.
- Wi-Fi recovery resumes pending work without retesting completed domains.
- Normal and abnormal termination restore Wi-Fi to the host.
- Repeated cleanup is safe.
