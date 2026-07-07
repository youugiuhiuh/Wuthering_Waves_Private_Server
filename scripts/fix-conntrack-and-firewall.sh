#!/usr/bin/env bash
# fix-conntrack-and-firewall.sh
# One-time VPS repair: apply sysctl and remove stale firewall port rules
# Usage: sudo bash scripts/fix-conntrack-and-firewall.sh
set -euo pipefail

echo "=== WWPS Conntrack & Firewall Repair Script ==="
echo ""

# --- Step 1: Apply sysctl immediately ---
echo "[1/3] Applying sysctl parameters..."
sysctl -w net.netfilter.nf_conntrack_max=262144
sysctl -w net.ipv4.tcp_max_tw_buckets=32768
echo "  sysctl applied (persisted via /etc/sysctl.d/90-wwps-bbr3-optimize.conf on next bot deploy)"
echo ""

# --- Step 2: Collect required ports from config files ---
echo "[2/3] Scanning config files for required ports..."
REQUIRED_PORTS=""

# Xray configs
XRAY_DIR="/etc/wwps/wwps-core/conf"
if [ -d "$XRAY_DIR" ]; then
  for f in "$XRAY_DIR"/*_inbounds.json; do
    [ -f "$f" ] || continue
    PORTS=$(python3 -c "
import json, sys
try:
    with open('$f') as fh:
        data = json.load(fh)
    inbounds = data if isinstance(data, list) else data.get('inbounds', [])
    for ib in (inbounds if isinstance(inbounds, list) else []):
        p = ib.get('port')
        if p:
            print(p)
except:
    pass
" 2>/dev/null || true)
    REQUIRED_PORTS="$REQUIRED_PORTS $PORTS"
  done
  echo "  Found xray configs in $XRAY_DIR"
else
  echo "  No xray config directory found"
fi

# Singbox configs
SB_DIR="/etc/wwps/wwps-box/conf"
if [ -d "$SB_DIR" ]; then
  for f in "$SB_DIR"/*.json; do
    [ -f "$f" ] || continue
    basename_f=$(basename "$f")
    [[ $basename_f == 00_* ]] && continue
    [[ $basename_f == 01_* ]] && continue
    PORTS=$(python3 -c "
import json
def extract(obj):
    if isinstance(obj, dict):
        if 'listen_port' in obj:
            p = obj['listen_port']
            print(p)
            if obj.get('type') == 'hysteria2':
                for hp in range(p+1, p+100):
                    print(hp)
        for v in obj.values():
            extract(v)
    elif isinstance(obj, list):
        for v in obj:
            extract(v)
try:
    with open('$f') as fh:
        data = json.load(fh)
    extract(data)
except:
    pass
" 2>/dev/null || true)
    REQUIRED_PORTS="$REQUIRED_PORTS $PORTS"
  done
  echo "  Found singbox configs in $SB_DIR"
else
  echo "  No singbox config directory found"
fi

# Always include SSH
REQUIRED_PORTS="$REQUIRED_PORTS 22"

# Normalize and deduplicate
REQUIRED_SORTED=$(echo $REQUIRED_PORTS | tr ' ' '\n' | sort -n | uniq | tr '\n' ' ')
echo "  Required ports: $REQUIRED_SORTED"
echo ""

# --- Step 3: Remove stale firewall rules ---
echo "[3/3] Checking firewalld for stale port rules..."

if command -v firewall-cmd &>/dev/null && systemctl is-active --quiet firewalld 2>/dev/null; then
  ZONE=$(firewall-cmd --get-default-zone 2>/dev/null || echo "public")
  CURRENT=$(firewall-cmd --zone="$ZONE" --list-ports 2>/dev/null || echo "")

  echo "  Current firewalld ports: $CURRENT"
  echo ""

  STALE=""
  for entry in $CURRENT; do
    PORT=$(echo "$entry" | cut -d'/' -f1)
    if ! echo "$REQUIRED_SORTED" | grep -qw "$PORT"; then
      STALE="$STALE $entry"
    fi
  done

  if [ -z "$STALE" ]; then
    echo "  No stale firewall rules found."
  else
    echo "  Found stale rules:$STALE"
    echo ""
    read -p "  Remove these stale rules? [y/N] " -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
      for entry in $STALE; do
        PORT=$(echo "$entry" | cut -d'/' -f1)
        PROTO=$(echo "$entry" | cut -d'/' -f2)
        echo "  Removing port $PORT/$PROTO ..."
        firewall-cmd --zone="$ZONE" --remove-port="$PORT/$PROTO" 2>/dev/null || true
        firewall-cmd --zone="$ZONE" --permanent --remove-port="$PORT/$PROTO" 2>/dev/null || true
      done
      firewall-cmd --reload 2>/dev/null || true
      echo "  Stale rules removed and firewalld reloaded."
    else
      echo "  Skipped removal."
    fi
  fi
elif command -v ufw &>/dev/null && ufw status | grep -q "active" 2>/dev/null; then
  echo "  UFW detected. Listing current rules:"
  ufw status numbered | head -30
  echo ""
  echo "  UFW port reconciliation is best handled by the aegis service."
  echo "  Run the bot and it will auto-sync on next reload."
else
  echo "  No supported firewall backend detected (firewalld or ufw)."
fi

echo ""
echo "=== Repair complete ==="
echo "Verify: cat /proc/sys/net/netfilter/nf_conntrack_max"
echo "Verify: firewall-cmd --list-ports (or ufw status)"
