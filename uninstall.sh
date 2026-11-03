#!/usr/bin/env bash
# Remove Lynapse 2.0. Your settings (~/.config/razerctl) are kept unless you pass --purge.
set -euo pipefail
pkill -9 -x razerctl 2>/dev/null || true
rm -f "$HOME/.local/bin/razerctl" "$HOME/.local/share/applications/razerctl.desktop" "$HOME/.config/autostart/razerctl.desktop" \
      "$HOME/.local/share/icons/hicolor/256x256/apps/razerctl.png"
pw=0
for f in 60-razerctl-thx.conf 62-razerctl-enhance.conf; do
    p="$HOME/.config/pipewire/pipewire.conf.d/$f"
    [[ -f "$p" ]] && rm -f "$p" && pw=1
done
# KWin helper scripts the app may have loaded
for s in razerctl-gameprof razerctl-smartpoll razerctl-osd; do
    command -v qdbus >/dev/null && qdbus org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "$s" >/dev/null 2>&1 || true
done
if [[ "${1:-}" == "--purge" ]]; then
    rm -rf "$HOME/.config/razerctl" "$HOME/.local/share/razerctl" "$HOME/.cache/razerctl"
    echo "Settings removed."
fi
[[ $pw == 1 ]] && systemctl --user restart pipewire pipewire-pulse wireplumber || true
echo "Lynapse 2.0 removed."
