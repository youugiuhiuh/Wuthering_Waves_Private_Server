# rustls-webpki Security Update Design

## Scope

Resolve only the seven RustSec advisories reported for `rustls-webpki` in
`rust/aegis/Cargo.lock`. Do not address unrelated audit warnings or advisories.

## Current State

`rustls-webpki 0.101.7` is pulled in through the direct Rustls 0.21 and
Reqwest/Teloxide dependency path. `rustls-webpki 0.102.8` is pulled in through
the Serenity/Poise Tokio Tungstenite path. RustSec requires at least
`rustls-webpki 0.103.13` for every affected advisory.

## Design

Upgrade Aegis's direct TLS and client dependencies to mutually compatible
versions that resolve to Rustls 0.23 and `rustls-webpki 0.103.13` or later:

- `reqwest`, `teloxide`, `rustls`, and `tokio-rustls`

The latest Serenity and Poise releases still depend on `tokio-tungstenite
0.21`, which retains the vulnerable WebPKI path. Keep those crate versions but
switch Serenity from `rustls_backend` to `native_tls_backend`, removing the
Rustls 0.22 dependency tree. This uses the system TLS provider for Discord
traffic.

Use Cargo commands to update dependencies and regenerate the lockfile. Adapt
only source code made incompatible by the Rustls 0.23 upgrade. Do not add
overrides, forks, or direct `rustls-webpki` dependencies.

## Verification

1. `cargo tree` shows no `rustls-webpki 0.101.7` or `0.102.8`.
2. `cargo audit --file rust/aegis/Cargo.lock` reports none of the seven target
   advisories.
3. Run formatting, Clippy, and the Aegis test suite.

## Non-Goals

- Upgrading unrelated dependencies.
- Resolving non-webpki audit findings.
- Changing application behavior beyond required compatibility fixes.
