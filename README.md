# WWPS Control Stack

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Platform](https://img.shields.io/badge/Platform-Linux%20x86__64-blue)](#quick-start)
[![Status](https://img.shields.io/badge/Status-Experimental-orange)](#disclaimer)

## Overview

This is the control plane used to deploy, initialize, update, and operate the WWPS environment on Linux VPS hosts.

An experimental self-hosted server emulator project for Wuthering Waves, focused on quick deployment, simple management, and stable runtime behavior on Linux VPS environments.

## Features

- One-command bootstrap installer
- Remote management workflow after deployment
- User profile and runtime configuration management
- Deployment asset rotation and maintenance utilities
- Network tuning and recovery helpers
- Suitable for lightweight VPS environments

## Components

- `rust/tgbot`: Telegram-based management bot
- `go/installer`: bootstrap installer and update entrypoint
- `rust/version-sync`: release/version synchronization helper
- `sni_tester`: standalone SNI and connectivity testing utility

## Quick Start

Recommended environment:

- Ubuntu 20.04+
- Debian 10+
- `amd64` or `arm64` Linux VPS

Install with:

```bash
wget -O /root/installer "https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest/download/installer" && chmod +x /root/installer && /root/installer
```

## Repository Contents

Releases include:

- `installer`: bootstrap program
- `tgbot`: management-side executable

Use the installer first. It will prepare the runtime environment and deploy the required components automatically.

## Operations

After installation, the management interface can be used for:

- service status checks
- user and configuration maintenance
- deployment updates
- runtime diagnostics
- security and recovery operations

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

The CI/CD workflows build release artifacts from this repository and create releases with `tgbot` and `installer` binaries.

Supported CI/CD platforms:

- GitHub Actions (`.github/workflows/public-release.yml`)
- GitLab CI (`.gitlab-ci.yml`)
- Azure Pipelines (`azure-pipelines.yml`)
- Bitbucket Pipelines (`bitbucket-pipelines.yml`)
- SourceHut Builds (`.build.yml`)

Trigger by bumping version in `rust/tgbot/Cargo.toml` and pushing to default branch, or run manually with version input.

## Scope

The project focuses on:

- host bootstrap and secure initialization
- Telegram-side operations and maintenance workflows
- inbound configuration generation and lifecycle management
- routing, geo data, certificate, and kernel/network maintenance helpers

## Disclaimer

This project is intended for educational and technical research purposes only.
Do not use it for commercial activities.

Wuthering Waves is a trademark of its respective owner. Please support the official game.
