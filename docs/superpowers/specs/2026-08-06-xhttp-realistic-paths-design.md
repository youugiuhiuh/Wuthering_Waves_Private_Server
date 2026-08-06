# Realistic XHTTP Path Generation

## Goal

Replace generated paths such as `/xhttp_9wacq` with paths that resemble ordinary long-lived HTTP resources while preserving the existing XHTTP server/client configuration flow.

## Scope

- Change only `ConfigManager::generate_random_path()` and its focused tests in `rust/aegis/src/core/xray/config.rs`.
- Keep the function signature and all callers unchanged so Reality XHTTP and TLS XHTTP receive the same behavior.
- Do not read or modify `tranco_JZ8ZY.csv`; it contains ranked domains, not usable URL paths.
- Add no dependencies or configuration options.

## Path Format

Each generated path has this form:

```text
/<long-connection-resource>/<10-character-id>
```

Nested bases are also allowed, for example `/api/events/k7m2q9x4pc`.

The resource base is selected uniformly from this fixed list:

```text
/events
/event-stream
/stream
/live
/updates
/notifications
/subscribe
/subscriptions
/realtime
/feed
/activity
/changes
/sync
/messages
/channels
/sessions
/presence
/api/events
/api/stream
/api/updates
/api/notifications
/v1/events
/v1/stream
/v1/updates
```

The ID contains exactly 10 characters selected uniformly from lowercase ASCII letters and digits. It represents an opaque channel, subscription, session, or resource identifier. A short ID is preferred to a UUID because it is less conspicuous and remains sufficiently varied for generated node paths.

## Data Flow

`generate_random_path()` creates one complete path whenever existing node-generation code calls it. Existing code then copies that same value into the Xray server's `xhttpSettings.path` and percent-encodes it into the generated VLESS client link. No caller or serialization behavior changes.

The path contains no query string. This avoids relying on unspecified query matching behavior and keeps server and client path handling identical to the current implementation.

## Implementation

Define the fixed path bases next to `generate_random_path()`. Reuse the existing entropy-seeded `StdRng` and `rand::Rng` support to:

1. Select one base by index.
2. Generate 10 lowercase alphanumeric characters.
3. Return the base and ID joined by `/`.

The fixed non-empty base list and fixed character set require no runtime fallback. Existing behavior also has no recoverable random-generation error path.

## Testing

Add a focused unit test that generates multiple paths and verifies each result:

- starts with one of the approved bases followed by `/`;
- ends with exactly 10 lowercase ASCII alphanumeric characters;
- contains no query string;
- does not start with `/xhttp_`.

The test must not assert that every random base appears or depend on selection frequency. Existing XHTTP config/link tests continue to cover propagation into server configuration and client links.

## Non-Goals

- Runtime path-list loading or Tranco integration.
- Weighted path selection.
- UUIDs, query parameters, timestamps, or database lookups.
- Changes to XHTTP transport behavior, routing, or link encoding.
