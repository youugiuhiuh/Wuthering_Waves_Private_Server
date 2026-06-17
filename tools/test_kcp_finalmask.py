#!/usr/bin/env python3
"""Test KCP FinalMask JSON format against xray-core v26.6.1"""
import json
import subprocess
import sys
import uuid
import os

PASS = 0
FAIL = 1

def generate_config(masks):
    return {
        "inbounds": [{
            "listen": "127.0.0.1",
            "port": 9999,
            "protocol": "vless",
            "settings": {
                "clients": [{"id": str(uuid.uuid4())}],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "kcp",
                "security": "none",
                "finalmask": {"udp": masks},
                "kcpSettings": {
                    "mtu": 1350, "tti": 50,
                    "uplinkCapacity": 5, "downlinkCapacity": 20
                }
            }
        }]
    }

MASKS = {
    "mkcp-legacy": [
        {"type": "mkcp-legacy", "settings": {"header": "dns", "value": "example.com"}},
        {"type": "mkcp-legacy", "settings": {"header": "wechat", "value": "123456"}},
        {"type": "mkcp-legacy", "settings": {"value": "pwd"}},
        {"type": "mkcp-legacy"},
    ],
    "noise": [
        {"type": "noise"},
    ],
    "salamander": [
        {"type": "salamander", "settings": {"password": "test", "packetSize": "512-1200"}},
        {"type": "salamander", "settings": {"password": "test"}},
    ],
    "sudoku": [
        {"type": "sudoku", "settings": {"password": "test"}},
    ],
    "xdns": [
        {"type": "xdns", "settings": {"domains": ["example.com"], "resolvers": ["example.com+udp://8.8.8.8:53"]}},
    ],
    "xicmp": [
        {"type": "xicmp", "settings": {"dgram": True, "ips": ["1.2.3.4", "5.6.7.8"]}},
        {"type": "xicmp"},
    ],
    "realm": [
        {"type": "realm", "settings": {"url": "realm://token@example.com:443/id", "stunServers": ["stun.l.google.com:19302"]}},
    ],
    "combined": [
        {"type": "mkcp-legacy", "settings": {"header": "dns", "value": "example.com"}},
        {"type": "noise"},
        {"type": "salamander", "settings": {"password": "test", "packetSize": "512-1200"}},
        {"type": "sudoku", "settings": {"password": "test"}},
        {"type": "xicmp", "settings": {"dgram": True, "ips": ["1.2.3.4"]}},
        {"type": "realm", "settings": {"url": "realm://token@example.com:443/id", "stunServers": ["stun.l.google.com:19302"]}},
    ],
}


def test_mask(name, masks):
    config = generate_config(masks)
    config_path = f"/tmp/kcp_test_{name}.json"
    with open(config_path, "w") as f:
        json.dump(config, f, indent=2)

    result = subprocess.run(
        ["xray", "convert", "pb", "-outpbfile", "/dev/null", config_path],
        capture_output=True, text=True, timeout=10
    )

    if result.returncode == 0:
        print(f"  ✓ {name}: accepted")
        return PASS
    else:
        err = result.stderr.strip().split("\n")[-1][:150] if result.stderr else ""
        print(f"  ✗ {name}: REJECTED - {err}")
        return FAIL


def main():
    xray_path = subprocess.run(["which", "xray"], capture_output=True, text=True).stdout.strip()
    if not xray_path:
        print("xray not found in PATH. Aborting.")
        return

    version = subprocess.run(["xray", "version"], capture_output=True, text=True).stdout.strip().split("\n")[0]
    print(f"Xray: {version}")
    print(f"Path: {xray_path}")
    print(f"Worktree: {os.getcwd()}")
    print()

    total = 0
    passed = 0
    for name, masks in MASKS.items():
        total += 1
        if test_mask(name, masks) == PASS:
            passed += 1

    print(f"\n{'='*40}")
    print(f"Result: {passed}/{total} passed")
    if passed == total:
        print("All mask types validated successfully!")
        sys.exit(0)
    else:
        print("Some tests failed!")
        sys.exit(1)


if __name__ == "__main__":
    main()
