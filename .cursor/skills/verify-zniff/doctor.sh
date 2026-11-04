#!/bin/bash
# Read-only. Exit 0 only when launching the zniff window cannot take the user's proxy.
set -euo pipefail
cfg="${HOME}/Library/Application Support/zniff/config.toml"
snap="${HOME}/Library/Application Support/zniff/proxy-snapshot.toml"
if pgrep -x zniff >/dev/null 2>&1; then
  echo "zniff process is already running"
  exit 1
fi
if [[ -f "$cfg" ]] && grep -E '^capturing *= *true' "$cfg" >/dev/null; then
  echo "config capturing is true; the window would press Start"
  exit 1
fi
if [[ -f "$cfg" ]] && grep -E '^use_system_proxy *= *true' "$cfg" >/dev/null; then
  echo "use_system_proxy is true"
  exit 1
fi
if [[ -f "$snap" ]] && grep -E '^active *= *true' "$snap" >/dev/null; then
  echo "proxy snapshot is active"
  exit 1
fi
echo "doctor ok: no zniff process, capturing off, system proxy off, snapshot inactive"
exit 0
