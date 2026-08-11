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

- `rust/aegis`: Telegram-based management bot
- `go/installer`: bootstrap installer and update entrypoint
- `go/cf-preferred-ip`: client-side Cloudflare preferred-IP utility for XHTTP TLS domains
- `rust/version-sync`: release/version synchronization helper
- `sni_tester`: standalone SNI and connectivity testing utility

## Quick Start

Recommended environment:

- Ubuntu 20.04+
- Debian 10+
- `amd64` or `arm64` Linux VPS

Install with:

**Interactive (simplest):** SSH into your server and run:

```bash
wget -O /root/installer "https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest/download/installer" && chmod +x /root/installer && ./installer
```

Follow the prompts to enter your Telegram Bot Token and Admin ID. Optionally configure Matrix for sensitive notification routing.

To update an existing installation, just run the same command again — the installer handles upgrades automatically.

**Headless (automation via SSH pipe):** download once, then pipe config via heredoc.

JSON format (best for simple ASCII values):
```bash
ssh root@YOUR_SERVER_IP "wget -qO /root/installer 'https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest/download/installer' && chmod +x /root/installer"
ssh -T root@YOUR_SERVER_IP /root/installer --setup-stdin <<'JSONEOF'
{"token":"YOUR_BOT_TOKEN","admin_id":"YOUR_ADMIN_ID","totp_secret":"","matrix_homeserver":"https://matrix.org","matrix_username":"@bot:matrix.org","matrix_password":"YOUR_PASSWORD","matrix_room_id":"!room:matrix.org"}
JSONEOF
```

Key=value format (avoids JSON quoting, recommended for passwords with special/non-ASCII characters):
```bash
ssh root@YOUR_SERVER_IP "wget -qO /root/installer 'https://github.com/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest/download/installer' && chmod +x /root/installer"
ssh -T root@YOUR_SERVER_IP /root/installer --setup-keyval <<'KVEOF'
token=YOUR_BOT_TOKEN
admin_id=YOUR_ADMIN_ID
totp_secret=
matrix_homeserver=https://matrix.org
matrix_username=@bot:matrix.org
matrix_password=YOUR_PASSWORD
matrix_room_id=!room:matrix.org
KVEOF
```

> Fields `matrix_*` are optional. Set `totp_secret` to empty string to auto-generate. Using a heredoc (`<<'EOF'`) avoids shell injection — your password can contain any characters safely. The key=value format passes raw bytes without JSON escaping, making it the easier choice when passwords contain special or non-ASCII characters.

## Repository Contents

Releases include:

- `installer`: bootstrap program
- `aegis`: management-side executable
- `aegis-cf-win-amd64.zip` and `aegis-cf-linux-amd64.tar.gz`: Cloudflare preferred-IP utility bundles

Use the installer first. It will prepare the runtime environment and deploy the required components automatically.

### Cloudflare Preferred IP Utility

For Cloudflare-proxied XHTTP TLS domains, this client-side utility runs the bundled CloudflareSpeedTest from the subscriber's network, then maps each supplied domain to a preferred Cloudflare edge IP in an owned hosts-file block. The domain remains unchanged for Xray SNI and Host routing.

Download the matching archive from the release page, extract it, then run:

```bash
# Linux
sudo ./run.sh example.com another.example.com

# Windows (run Command Prompt as Administrator)
run.bat example.com another.example.com
```

Use `--restore` to remove only the utility-owned hosts-file block without running a test:

```bash
sudo ./run.sh --restore
```

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
cd rust/aegis
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

The CI/CD workflows build release artifacts from this repository and create releases with `aegis` and `installer` binaries.

Supported CI/CD platforms:

- GitHub Actions (`.github/workflows/public-release.yml`)
- GitLab CI (`.gitlab-ci.yml`)
- Azure Pipelines (`azure-pipelines.yml`)
- Bitbucket Pipelines (`bitbucket-pipelines.yml`)
- SourceHut Builds (`.build.yml`)

Trigger by bumping version in `rust/aegis/Cargo.toml` and pushing to default branch, or run manually with version input.

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
