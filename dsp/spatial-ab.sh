#!/usr/bin/env bash
# A/B the spatial-audio HRIR: ./spatial-ab.sh kemar | atmos   (rewrites the filter's IR file and restarts PipeWire)
set -euo pipefail
conf="$HOME/.config/pipewire/pipewire.conf.d/60-razerctl-thx.conf"
dir="$HOME/.local/share/razerctl/thx"
case "${1:-}" in
  kemar) to="$dir/lynapse-kemar.wav" ;;
  atmos) to="$dir/atmos.wav" ;;
  *) echo "usage: $0 kemar|atmos   (now: $(grep -o 'filename = "[^"]*"' "$conf" | head -1))"; exit 1 ;;
esac
[[ -f "$to" ]] || { echo "missing $to"; exit 1; }
sed -i -E "s#filename = \"[^\"]*\"#filename = \"$to\"#g" "$conf"
systemctl --user restart pipewire pipewire-pulse wireplumber
echo "spatial audio now uses $(basename "$to")"
