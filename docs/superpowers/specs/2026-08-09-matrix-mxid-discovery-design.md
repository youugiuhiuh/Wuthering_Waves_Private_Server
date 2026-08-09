# Matrix MXID Homeserver Discovery Design

## Goal

Generalize Matrix homeserver setup in `go/installer`: accept a Matrix user ID
(MXID), discover its Client API homeserver through the Matrix well-known
endpoint, and keep password authentication unchanged.

## Scope

Only the first-time Matrix configuration flow changes. The installer will no
longer present a fixed list of public homeservers. Rust runtime login, session
restoration, encrypted configuration field names, and setup payload shape stay
unchanged.

## MXID Input And Discovery

The installer accepts both `@localpart:server.name` and
`localpart:server.name`. It normalizes the latter by adding `@`, then stores the
normalized MXID in `matrix_username`.

The `server.name` part is used to request:

```
https://server.name/.well-known/matrix/client
```

The installer reads `m.homeserver.base_url` from the JSON response and stores
the valid HTTPS URL in `matrix_homeserver`. For example,
`@us-sanjose:matrix.org` discovers `https://matrix-client.matrix.org`.

## Fallback And Validation

Reject MXIDs that are empty, lack `:`, have an empty localpart or server name,
or contain whitespace. Discovery uses the Go standard library with a bounded
HTTP timeout and accepts only a valid HTTPS `m.homeserver.base_url`.

In an interactive terminal, discovery failures and invalid well-known responses
show the reason and prompt for an arbitrary HTTPS homeserver URL. The original
MXID remains the login identity. In non-interactive use, discovery failures are
returned as errors rather than requesting another input.

## Authentication Compatibility

The existing password prompt, room prompt, recovery-key prompt, and encrypted
payload fields remain unchanged. The Rust Matrix SDK receives the discovered or
manually supplied homeserver and the normalized MXID, then continues its
existing password login and session restoration behavior.

## Localization

Replace fixed-homeserver selector copy with MXID prompt, discovery status,
failure, and manual HTTPS homeserver fallback strings. Provide every new key in
English, Japanese, and Chinese.

## Testing

Add unit tests for MXID normalization, invalid MXIDs, successful well-known
discovery, malformed or non-HTTPS discovery responses, manual URL validation,
and the non-interactive failure path. Tests must use an in-process HTTP test server
rather than a public Matrix service.
