# Aegis Platform Decoupling Design

## Goal

Separate chat-platform implementations from Aegis business code so a new compiled-in platform requires only a platform module and one static registry registration. Existing Telegram, Discord, and Matrix behavior, text, buttons, callbacks, edits, media delivery, and progress reporting must remain unchanged.

This refactor separates chat transports, not operating systems. Aegis remains a Linux server-management application.

## Current Problem

Concrete SDK imports are already mostly confined to `gateways`, but the abstraction boundary still exposes a Telegram-like interaction model:

- Shared events carry `Arc<dyn BotAdapter>`.
- Handlers directly call send, edit, delete, callback, and media methods.
- Business code creates transport markup and tracks platform message IDs.
- `BotAdapter` mixes chat delivery with host operations such as locale and timezone setup.
- Capability declarations do not consistently govern behavior.
- `RoutingAdapter` does not forward `set_system_locale`, so Telegram-only and multi-platform modes behave differently.

The result is SDK isolation without full business/platform isolation.

## Chosen Approach

Use a normalized event/action boundary with a static Rust registry:

```text
Platform implementation -> DomainEvent -> Dispatcher -> UiAction -> Renderer
                                  |              |
                                  +-- state -----+-- HostOperations
```

Rust traits provide compile-time contracts. New platforms may require recompilation. Runtime dynamic libraries, WASM plugins, subprocess plugins, an event bus, and actor infrastructure are out of scope.

This follows the useful parts of Hermes Agent's plugin architecture: normalized inbound events, centralized outbound delivery, deterministic fallback, and registration-owned setup. It intentionally avoids Hermes' large multi-purpose base adapter and platform-specific prompt hints.

## Domain Events

Gateways translate SDK payloads into three platform-neutral variants:

```rust
enum DomainEvent {
    MessageReceived { context: EventContext, content: MessageContent },
    CommandInvoked { context: EventContext, name: String, args: Vec<String> },
    InteractionSelected { context: EventContext, action_id: String, value: String },
}
```

`EventContext` contains only opaque route, conversation, actor, and source-message identifiers. It contains no adapter, SDK payload, concrete platform enum, or platform message type. Runtime routing tables, not business state, resolve these opaque identifiers to concrete platform destinations.

Gateways reject malformed or unrecognized SDK payloads before dispatch. Authorization and business routing remain centralized in the dispatcher.

## UI Actions

Business handlers emit intent through a platform-neutral `ActionSink`:

```rust
enum UiAction {
    Publish { content: UiContent, key: Option<LogicalMessageKey> },
    Replace { key: LogicalMessageKey, content: UiContent },
    Remove { key: LogicalMessageKey },
    Acknowledge { interaction: InteractionRef, content: Option<UiContent> },
    SetActivity { activity: ActivityState },
    DeliverMedia { media: MediaSource, caption: Option<String> },
}
```

Names are illustrative; the implementation plan may align them with existing project naming without changing these semantics.

`UiContent` describes text and semantic choices, not Telegram/Discord/Matrix markup. Business-defined action IDs remain stable across renderers. Long-running tasks emit actions through `ActionSink` rather than receiving a chat adapter.

Business code identifies status messages with logical keys such as `progress:upgrade`. An application-level delivery ledger maps `(route, logical key)` to actual platform message IDs. Business state never stores those IDs.

## Components

### PlatformAdapter

Owns SDK connection lifecycle and inbound normalization. It emits `DomainEvent` values and contains no business decisions or host operations.

### ActionRenderer

Converts `UiAction` into platform API calls. It owns message-size rules, native controls, formatting, media support, and deterministic capability fallback. A platform may implement `PlatformAdapter` and `ActionRenderer` on one struct, but the contracts remain separate and small.

### Dispatcher

Owns authentication, command/message/interaction routing, state transitions, and handler invocation. It depends only on domain types, `ActionSink`, and application services.

### ActionRouter And Delivery Ledger

Routes actions back through the renderer associated with the event route. It records platform delivery receipts and resolves logical message keys for replace/remove operations. Ledger entries are runtime delivery state, not domain state.

### HostOperations

Owns locale, timezone, maintenance scheduling, systemd, upgrade, and related Linux effects. Chat-platform code cannot implement these operations. Calls return real failures so every initiating platform receives the same outcome.

### PlatformRegistry

Uses a static name-to-factory registry. Each registration owns platform-specific configuration decoding and validation, factory construction, and metadata. Existing CLI options remain compatible; a generic named selection path supports future registrations without adding business branches.

Only the composition root can see concrete platforms, the registry, the dispatcher, and their wiring together.

## Dependency Rules

- `business`, `shared`, and reusable `core` flows must not import gateways or platform SDKs.
- Domain events and UI actions must not contain `BotAdapter`, concrete platform types, SDK payloads, or SDK message IDs.
- Business code must not branch on platform capabilities or platform identity.
- Capability decisions and fallback belong exclusively to renderers.
- Host operations must not be methods on chat adapters.
- Adding a platform must not modify handlers, commands, state operations, or domain contracts unless the product itself gains new semantics.

## Capability Degradation

Fallback is automatic only when it preserves meaning and safety:

| Intent | Deterministic fallback |
| --- | --- |
| Buttons | Numbered text choices mapped to the same business action IDs |
| Replace | Publish a new message and update the ledger only for unsupported edit or missing target |
| Media | Send an existing safe URL when available |
| Callback acknowledgement | Send a short message when native acknowledgement is unavailable |

Transient edit failures must not publish duplicates. Local or sensitive files must not be exposed to create a media fallback. Fallback must never fabricate business success. Each action has at most one explicit fallback path; recursive degradation and catch-all success are forbidden.

## Error Model

- **Inbound errors:** gateways log malformed or unsupported updates and do not dispatch partial events.
- **Business errors:** the dispatcher maps them to consistent UI actions; renderers do not interpret domain failures.
- **Delivery errors:** typed errors distinguish at least `Unsupported`, `NotFound`, `RateLimited`, `Unauthorized`, and `Transient`. Only explicitly allowed categories trigger fallback.
- **Host errors:** propagate before a success action is emitted; no chat platform receives a false success result.

Logs include registration name, opaque route/conversation identity, action type, and error category. They exclude tokens, message bodies, and raw SDK payloads. Failure of one enabled platform must not terminate other platform runtimes.

## Migration

1. Characterize current Telegram, Discord, and Matrix behavior for commands, messages, callbacks, buttons, edits, progress updates, and media.
2. Introduce domain events, UI actions, action sink, typed delivery results, and the delivery ledger.
3. Convert each gateway to inbound normalization and remove adapters from shared events.
4. Convert outbound handler flows in small batches from adapter calls to actions, running equivalence tests after each batch.
5. Extract locale/timezone/maintenance behavior into `HostOperations` and remove the routing no-op.
6. Wire the static registry at the composition root and delete obsolete `BotAdapter` coupling, concrete message IDs, and temporary migration bridges.

No temporary compatibility layer remains in the final architecture.

## Verification

### Behavior Baseline

Characterization tests lock existing visible behavior and text before conversion.

### Pure Business Tests

A `RecordingActionSink` verifies that the same `DomainEvent` produces the same actions without constructing a platform mock.

### Renderer Contract Suite

Every renderer runs the same cases for native output, button fallback, edit fallback, safe media fallback, and typed errors. A new registration is incomplete until it passes this suite.

### Host Operation Tests

An injected command runner proves requests from every platform invoke the same host operation once, propagate failures, and never acknowledge success early.

### Architecture Guard

CI scans business/shared/core source boundaries and rejects imports or references to gateways, platform SDKs, `BotAdapter`, concrete platform enums, and concrete message-ID types.

### Required Gates

Run `cargo fmt`, Clippy with warnings denied, the complete test suite, and the architecture guard.

## Acceptance Criteria

- Telegram, Discord, and Matrix preserve all currently observable behavior and text.
- Business events, handlers, commands, state operations, and long-running task reporting contain no chat adapter or concrete platform dependency.
- Host behavior is identical regardless of the initiating chat platform.
- A test platform can be added using only a platform module and one static registry registration.
- Unsupported features follow the documented deterministic and safety-preserving degradation rules.
- All required formatting, lint, behavior, contract, host-operation, and architecture tests pass.

## External Reference

Hermes Agent repository reviewed on 2026-08-03: <https://github.com/NousResearch/hermes-agent>. Relevant patterns were taken from `gateway/platforms/base.py`, `gateway/platform_registry.py`, and the documented plugin registration path.
