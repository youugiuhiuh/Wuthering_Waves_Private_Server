# Aegis Complete Platform Decoupling Design

**Date:** 2026-08-02
**Status:** Awaiting implementation approval

## Goal

Completely separate Aegis business behavior from Telegram, Matrix, and Discord interaction details while preserving explicit cross-platform delivery of protected output.

## Decisions

- Business code classifies output with `Sensitivity::Public` or `Sensitivity::Protected` at the point where the output is created.
- No component may infer sensitivity from message text. `is_sensitive` and equivalent content heuristics must not return.
- Telegram and Matrix combination mode sends Public output to the request's origin platform and Protected output to Matrix.
- If Matrix is unavailable in combination mode, Aegis discards the protected payload and sends only a generic failure notice to Telegram.
- Protected data never falls back to Telegram in combination mode.
- Single-platform installations render Protected output as a file attachment on that platform.
- Discord remains a standalone platform and is not included by `--all`.
- No protected-output queue, retry store, or delayed delivery is included.

## Current State

The feature branch already provides platform-neutral `BusinessInput`, `BusinessMessage`, `BusinessOutput`, and `Sensitivity` types. The concrete SDKs are isolated under `src/gateways` and `src/main`.

The separation is incomplete:

- `src/app/auth.rs` sends messages through `BotAdapter` and constructs interaction markup.
- Legacy events carry `Arc<dyn BotAdapter>` into business handlers.
- `src/shared` handlers perform business decisions and platform interaction together.
- `ApplicationService` handles only a subset of commands; other events fall through `dispatch_event`.
- No current business path emits `Sensitivity::Protected`.
- Existing Presenter support for Protected output is therefore not enough to enforce routing.

## Target Boundaries

```text
Telegram / Matrix / Discord SDK
              |
          gateways
  input mapping / presenters
              |
      ApplicationEvent
              |
      ApplicationService
              |
         OutputAction
              |
    SensitiveOutputRouter
              |
      concrete presenter
```

### Core

`src/core` contains domain rules and system capabilities. It imports no application UI types, platform SDKs, gateways, or interaction ports.

### Application

`src/app` owns platform-neutral input and output contracts and orchestrates workflows. Application events contain identities, text, callback data, and already-downloaded attachment bytes, but never SDK objects or adapters.

Business handlers produce `OutputAction` values. They may describe semantic buttons, edits, callback acknowledgements, attachments, and sensitivity, but do not send anything directly.

### Gateways

`src/gateways` converts SDK updates into application events and converts output actions into SDK calls. Gateways download incoming platform files before passing them into the application boundary.

### Composition

`src/main` connects configured platforms and wires the output router. Routing configuration does not live in `AppState` and is not visible to business workflows.

## Application Contracts

The application input model must cover:

- Commands
- Plain text
- Callback actions
- Downloaded attachments

The output model must cover:

- Send message or attachment
- Edit message
- Delete message
- Answer callback

Every sendable payload carries an explicit `Sensitivity`. Non-send actions always target the origin platform because cross-platform editing, deletion, and callback acknowledgement are invalid.

Platform and conversation IDs remain namespaced identity values. Their presence in application contracts does not introduce SDK coupling.

## Routing Policy

| Mode | Public | Protected |
| --- | --- | --- |
| Telegram | Telegram text | Telegram attachment |
| Matrix | Matrix text | Matrix attachment |
| Discord | Discord text | Discord attachment |
| Telegram + Matrix | Origin platform | Matrix attachment |
| Telegram + Matrix, Matrix unavailable | Origin platform | Drop payload; origin receives generic notice |

The router examines only `Sensitivity` and installation mode. It must not inspect payload text.

When a Matrix-only installation cannot connect, startup fails because no usable interface remains. When a Telegram + Matrix installation cannot connect to Matrix, Telegram starts in degraded mode. Matrix delivery failures after startup use the same fail-closed behavior.

## Sensitivity Ownership

Sensitivity is assigned where business output is constructed:

- Proxy URIs and complete client configurations are Protected.
- Passwords, private keys, secret keys, TOTP values, and security-file contents are Protected.
- Menus, status reports, progress messages, validation errors, and generic failure notices are Public unless they contain protected material.
- Authentication input must never be echoed in any output.

Tests assert the assigned enum value. They must not duplicate the removed text-scanning heuristic.

## Installer Behavior

Fresh installation keeps the existing modes:

- `tg`: no platform flag
- `matrix`: `--matrix`
- `tg-matrix`: `--all`
- `discord`: `--discord`

An upgrade with an existing `config.enc` must preserve the current systemd platform flag rather than silently replacing it with Telegram-only mode. Discord remains exclusive, and `--all` continues to mean Telegram plus Matrix only.

## Error Handling

- Protected Matrix routing failure never exposes the protected payload to the origin presenter.
- The origin receives a fixed Public notice that Matrix is unavailable and the protected output was not sent.
- The original Matrix error is logged and returned for observability.
- Public-output errors are returned normally.
- Input mapping and file-download errors are translated at the gateway boundary without SDK error types entering the application layer.

## Migration Strategy

Use a strangler migration in independently testable vertical slices:

1. Establish complete platform-neutral contracts and routing tests.
2. Migrate commands, menu, and authentication.
3. Migrate callbacks and operational handlers.
4. Migrate configuration-producing workflows and mark protected output.
5. Migrate destruct, file, and reporter flows.
6. Rewire runtimes and remove the legacy adapter/event bridge.
7. Fix installer upgrade mode preservation.

Each slice uses characterization tests before replacing its legacy path. No big-bang rewrite is permitted.

## Acceptance Criteria

- `src/app`, `src/core`, and business handlers contain no platform SDK, gateway, or `BotAdapter` references.
- Legacy events do not carry adapters.
- `BotAdapter`, `RoutingAdapter`, `PlatformCapabilities`, `is_sensitive`, and temporary dispatch bridges are removed when no longer referenced.
- Every supported user action still works on Telegram, Matrix, and Discord where the platform supports it.
- Equal raw conversation IDs on different platforms remain isolated.
- Public output remains on the request's origin platform.
- Protected output follows the routing table exactly.
- Matrix failure tests prove that protected bytes never reach Telegram.
- Installer tests cover all four platform modes and existing-install upgrades.
- Rust formatting, linting, unit, integration, and documentation tests pass.
- Go formatting, tests, and static analysis pass.

## Out of Scope

- Persisted protected-message queues
- Automatic delayed retry after Matrix recovery
- Discord combined with another platform
- New platform integrations
- Unrelated business or UI redesign
