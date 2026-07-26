# SNI Tester Resumable Streaming Design

## Goal

Keep the Android app responsive during local backend startup, bound memory for
100-500 MB inputs, and automatically resume an incomplete test after the app
process is killed, provided Android app data has not been cleared.

## Evidence and Constraints

- Device exit records show the overnight process was force-stopped by vivo RMS,
  not by an Android OOM or native crash. Its last recorded RSS was 763 MB.
- Three startup ANRs occurred after app data was cleared. Missing GeoDB files
  were downloaded synchronously through Go FFI on Flutter's UI isolate.
- The current upload and execution path keeps several full copies of the input
  in memory and builds a full historical skip map.
- Android "Clear data" intentionally deletes all local inputs, checkpoints, and
  results and is outside recovery scope.
- A privileged OEM process can still force-stop the app. Recovery, rather than
  immunity from force-stop, is the required guarantee.

## Architecture

Use a durable input file, a streaming Go execution pipeline, and atomic task
checkpoints:

1. Flutter starts the local Go backend in a background isolate so GeoDB
   preparation and other synchronous FFI work cannot block the UI isolate.
2. Uploads stream to a temporary file and atomically rename it only after the
   full input has been persisted.
3. Go scans the persisted input from disk into a bounded worker queue instead
   of retaining complete `inputText`, `domains`, or `toTest` collections.
4. Task metadata and progress are atomically checkpointed once per minute.
5. Badger remains the durable record of completed results and is queried per
   domain during scanning instead of being copied into a full in-memory map.
6. On app startup, an incomplete task is resumed automatically without asking
   the user. The UI reconnects to current progress through the status and SSE
   APIs.

Recovery is at least once: work completed since the last checkpoint may be
tested again, but already persisted results must not be lost.

## Durable Files

The backend owns these files in app-private storage:

- `task-input.tmp`: incomplete upload; never eligible for execution.
- `task-input.txt`: atomically published durable input.
- `task.json`: input path, original run parameters, creation time, and task
  state (`ready`, `running`, `paused`, `completed`, or `failed`).
- `checkpoint.json`: latest valid scanner position and cumulative statistics.
- `checkpoint.prev.json`: previous valid checkpoint used as fallback.

The input is persisted once. Minute checkpoints update only task state,
scanner position, parameters, and statistics; they do not copy the input.

## Components and Data Flow

### Flutter Startup

`ApiClient` opens and invokes `NativeBridge.startServer` inside a background
isolate. The UI isolate remains responsive while it polls the health endpoint
and displays backend initialization state. A bridge in the UI isolate remains
available for later shutdown because Go server state is process-global.

After health becomes available, Flutter requests status. If an incomplete task
was resumed by the backend, Flutter reconnects SSE and displays its restored
statistics. The Dart status model must match the Go boolean `running` contract.

### Upload and Task Creation

The upload handler streams multipart content directly to `task-input.tmp`.
After flushing and synchronizing the complete file, it atomically renames the
file to `task-input.txt` and atomically publishes `task.json` in the `ready`
state. A crash during upload leaves no executable task.

Starting a task changes the manifest to `running` before processing begins.
The request parameters are applied to the engine used for this run, including
the configured fixed worker count. The default mobile worker count remains 20.

### Streaming Execution

The scanner opens `task-input.txt` at the byte offset from the latest valid
checkpoint. It normalizes and validates one domain at a time and feeds a
bounded channel. Workers query durable history for each domain and skip entries
that already have a completed result.

The scanner and workers must not build full-file slices or a full historical
skip map. Existing bounded TLS, country, and ASN caches remain bounded.

Successful, blocked, and failed results continue to be written in batches.
Once per minute, the scanner briefly stops feeding new work, lets the bounded
queue drain, flushes pending result batches, and then atomically checkpoints
the resulting safe byte offset and statistics. A checkpoint offset must never
advance past work that has not reached durable storage. Orderly pause and
completion use the same barrier. On completion the backend writes a final
checkpoint and changes the task state to `completed`.

### Automatic Recovery

When the backend starts, it loads `task.json`. A `ready`, `running`, or `paused`
task with a valid input file is started automatically using its saved
parameters. It loads `checkpoint.json`, falls back to `checkpoint.prev.json` if
necessary, and otherwise scans from the beginning. Durable result lookups make
a full rescan safe.

Recovery does not depend on the old Go process, Flutter isolate, SSE connection,
or notification state.

## Error Handling

- Atomic JSON writes use a temporary file, file synchronization, and rename.
- Before replacing the current checkpoint, the last valid version becomes
  `checkpoint.prev.json`.
- If both checkpoints are invalid, processing restarts at byte zero and skips
  completed domains through Badger.
- Startup removes abandoned upload temporary files. It never executes them.
- A missing durable input stops recovery and surfaces a clear UI error rather
  than creating an empty task.
- A checkpoint or task-manifest write failure pauses processing while retaining
  the previous valid checkpoint. Processing must not continue without a
  recoverable state.
- SSE disconnection triggers status reconciliation and reconnection; it does
  not mark the task complete.
- GeoDB preparation failures are reported without freezing the UI.
- Clearing Android app data is destructive and cannot be recovered locally.

## Memory Controls

- Do not retain multipart uploads in a `bytes.Buffer` or convert the complete
  input to a Go string.
- Do not construct full `domains`, `toTest`, or historical skip-map collections.
- Use a bounded scanner-to-worker channel and the configured fixed worker count.
- Advance the durable input offset only after the queue drains and all preceding
  results have been flushed.
- Apply run-time request configuration to the actual engine executing the task.
- Keep existing result batching and bounded caches.

For a 500 MB input, device RSS must remain below 450 MB and show no continuing
upward trend after warm-up. This is an acceptance threshold, not a guarantee
that vivo RMS will never force-stop the process.

## Notification and UI Behavior

Platform notification updates must be throttled independently from raw SSE
event frequency. Flutter displays restored progress after reconnection and must
not reset counters merely because the process or SSE connection restarted.

The app automatically resumes an incomplete task after launch. No confirmation
dialog is shown.

## Testing and Acceptance

### Automated Tests

- Atomic checkpoint write, previous-checkpoint fallback, and corrupt-checkpoint
  recovery.
- Task-manifest parsing and state transitions.
- Dart and Go status contract compatibility.
- Streaming a large generated input through a bounded queue without full-file
  collections.
- Resume from a saved byte offset and skip already persisted results.
- Application of fixed-worker and other run parameters to the executing engine.
- Upload interruption leaves only a non-executable temporary file.

### Device Verification

- Delete GeoDB files and launch the app; initialization remains responsive and
  no 5-second input ANR occurs.
- Run 100 MB and 500 MB inputs while sampling RSS and confirming it does not
  grow with the number of processed lines.
- Force-stop the process during a run, reopen the app, and verify automatic
  continuation to completion with no persisted result loss. Reprocessing up to
  approximately one minute of work is acceptable.
- Run continuously for 24 hours while recording RSS, progress, notification
  freshness, and Android process exit information.
- For a 500 MB input, RSS peak is below 450 MB and stable after warm-up.

## Out of Scope

- Recovering data after Android "Clear data" or app uninstall.
- Preventing a privileged OEM process from force-stopping the package.
- A per-domain durable job queue or Android WorkManager/service rewrite.
- Exact-once execution of the final minute before an abrupt process death.
