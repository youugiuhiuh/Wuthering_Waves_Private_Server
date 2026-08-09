# Matrix Homeserver Selector Design

## Goal

Improve the Matrix homeserver step in `go/installer` by adding commonly used
public homeservers and a keyboard-driven, single-choice selector. Keep the
existing custom/self-hosted homeserver path.

## Scope

Only the homeserver selection step during first-time Matrix configuration
changes. The main installer menu, other service-platform selectors, Matrix
credentials, and setup payloads remain unchanged.

## Selected Approach

Use Bubble Tea as the terminal interaction library. Add it with `go get` from
the `go/installer` module; do not edit dependency files manually.

Bubble Tea owns raw-terminal lifecycle handling so terminal state is restored
when the selector exits normally, is interrupted, reaches EOF, or returns an
error.

## Interaction

The selector is single-choice and presents options in this fixed order:

1. `https://matrix.org`
2. `https://unredacted.org`
3. `https://nope.chat`
4. `https://pub.solar`
5. `https://frei.chat`
6. `https://private.coffee`
7. Custom homeserver

Up and Down move the highlighted row. Space records the highlighted row as the
single choice. Enter confirms the highlighted or selected row and returns its
value to the existing first-time setup flow.

Choosing Custom homeserver closes the selector and uses the existing line-input
prompt to accept an arbitrary self-hosted URL. Blank custom input continues to
default to `https://matrix.org`, preserving the existing behavior.

## Fallback And Errors

When stdin or stdout is not a terminal, the installer does not start Bubble Tea.
It uses the current text-input path and retains the `https://matrix.org`
default. Selector interruptions and errors return control to the surrounding
setup flow without leaving the terminal in raw mode.

## Localization

Add English and Chinese strings for the selector title, keyboard instructions,
server labels, custom option, and custom-address prompt. Keep all existing
homeserver strings that are still used by the non-interactive fallback.

## Testing

Add focused unit tests for selector state behavior: initial selection, Up/Down
navigation, Space selection, Enter confirmation, and Custom homeserver result.
Do not require an interactive terminal in tests. Run the existing Go installer
test suite after the change.
