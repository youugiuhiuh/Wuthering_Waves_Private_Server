# Wuthering Waves Private Server

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Platform](https://img.shields.io/badge/Platform-Linux%20x86__64-blue)](#quick-start)
[![Status](https://img.shields.io/badge/Status-Experimental-orange)](#disclaimer)

An experimental self-hosted server emulator project for Wuthering Waves, focused on quick deployment, simple management, and stable runtime behavior on Linux VPS environments.

## Features

- One-command bootstrap installer
- Remote management workflow after deployment
- User profile and runtime configuration management
- Deployment asset rotation and maintenance utilities
- Network tuning and recovery helpers
- Suitable for lightweight VPS environments

## Quick Start

Recommended environment:

- Ubuntu 20.04+
- Debian 10+
- `amd64` Linux VPS

Install with:

```bash
wget -O /root/installer "https://github.com/NicholasDewar/Wuthering_Waves_Private_Server/releases/latest/download/installer" && chmod +x /root/installer && /root/installer
```

## Repository Contents

Public releases include:

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

## Release Policy

- Public repository: release artifacts and public-facing documentation
- Private source repository: canonical implementation and build pipeline

## Disclaimer

This project is intended for educational and technical research purposes only.
Do not use it for commercial activities.

Wuthering Waves is a trademark of its respective owner. Please support the official game.
