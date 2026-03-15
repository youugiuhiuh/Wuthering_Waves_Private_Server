# WWPS Control Stack

Private source repository for the WWPS deployment and operations stack.

## Overview

This repository contains the control plane used to deploy, initialize, update, and operate the WWPS environment on Linux VPS hosts.

Main components:

- `rust/tgbot`: Telegram-based management bot
- `go/installer`: bootstrap installer and update entrypoint
- `rust/version-sync`: release/version synchronization helper
- `sni_tester`: standalone SNI and connectivity testing utility

## Scope

The project focuses on:

- host bootstrap and secure initialization
- Telegram-side operations and maintenance workflows
- inbound configuration generation and lifecycle management
- routing, geo data, certificate, and kernel/network maintenance helpers
- release packaging for a separate public distribution repository

## Repository Model

- This repository is private and is the source of truth.
- GitHub Actions publishes selected build artifacts to a separate public repository.
- Public-facing files can differ from the private repository, including documentation.

## Build

### Rust bot

```bash
cd rust/tgbot
cargo build --release
```

### Go installer

```bash
cd go/installer
go build -o installer .
```

### SNI tester

```bash
cd sni_tester
go build .
```

## Release Flow

The workflow at [public-release.yml](/home/ub/Dark/Wuthering_Waves_Private_Server_source_code/.github/workflows/public-release.yml) builds release artifacts from this private repository and pushes binaries plus a public README to the public repository.

## Notes

- Do not treat the public repository as the canonical source tree.
- Runtime-visible naming and public-facing materials are managed separately from the private implementation.
