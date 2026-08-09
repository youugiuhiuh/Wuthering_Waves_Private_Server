# Deployment Platform Selector Design

## Goal

Replace the first-time setup's numbered deployment-platform prompt with a
keyboard-driven selector that makes selected services and supported
combinations explicit.

## Scope

This change applies only to first-time deployment platform selection. It does
not change credential collection, Matrix homeserver selection, or the setup
payload format.

## Interaction

The selector displays Telegram, Matrix, and Discord as checkboxes. Up/Down
moves the cursor, Space toggles the highlighted service, and Enter confirms.
The footer always renders the current selection in a human-readable form.

Supported results are exactly:

- Telegram
- Matrix
- Discord
- Telegram + Matrix
- Discord + Matrix

Telegram and Discord are mutually exclusive. Selecting either automatically
clears the other. Matrix can be independently selected or cleared. The UI
must prevent an empty selection, Telegram + Discord, and all-three selection.

## Fallback And Errors

When stdin or stdout is not a TTY, preserve a text prompt. It accepts only the
five supported results and rejects other values with the existing invalid
platform error. Cancellation and terminal-program errors are returned to the
first-time setup caller.

## Compatibility

Map each accepted selection to the existing Telegram, Matrix, and Discord
booleans so the unchanged credential and payload code continues to run for
each selected service. Keep existing locale support complete for English,
Japanese, and Chinese.

## Testing

Add pure selector tests for the default state, cursor movement, service
toggling, Telegram/Discord mutual exclusion, valid confirmation, invalid
empty confirmation, and conversion to the existing platform booleans. Test
the non-TTY parser against all five valid values and invalid input.
