#!/usr/bin/env bash
# Lynapse 2.0 installer (per user, no root needed except for installing missing system packages).
#
#   ./install.sh --spatial       app + Spatial Audio (virtual surround filter + free KEMAR HRIR)   = install-with-spatial.sh
#   ./install.sh --no-spatial    app only                                                          = install-without-spatial.sh
#   extra flags:  --no-deps (don't install system packages)  --no-autostart  --hrir FILE (use your own 14-ch HeSuVi WAV)
#
# Installs:  ~/.local/bin/razerctl, menu entry + icon, autostart entry (starts minimised to the tray),
#            and with --spatial: ~/.local/share/razerctl/thx/<hrir>.wav + ~/.config/pipewire/pipewire.conf.d/60-razerctl-thx.conf
# Undo with ./uninstall.sh
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(pwd)

SPATIAL="" DEPS=1 AUTOSTART=1 HRIR_SRC="$ROOT/assets/spatial/lynapse-kemar.wav"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --spatial) SPATIAL=1 ;;
        --no-spatial) SPATIAL=0 ;;
        --no-deps) DEPS=0 ;;
        --no-autostart) AUTOSTART=0 ;;
        --hrir) HRIR_SRC=$(realpath "$2"); shift ;;
        -h|--help) sed -n '2,11p' "$0"; exit 0 ;;
        *) echo "unknown option: $1 (see --help)"; exit 2 ;;
    esac
    shift
done
if [[ -z "$SPATIAL" ]]; then
    read -r -p "Install Spatial Audio (virtual surround for games)? [Y/n] " a
    [[ "$a" =~ ^[Nn] ]] && SPATIAL=0 || SPATIAL=1
fi
[[ $EUID -eq 0 ]] && { echo "Run this as your normal user, not root (it installs into your home folder)."; exit 1; }

say() { printf '\n\033[1;32m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m!! %s\033[0m\n' "$*"; }

# ---------------------------------------------------------------- 1. system packages
say "Checking build and runtime dependencies"
. /etc/os-release 2>/dev/null || true
like=" ${ID:-} ${ID_LIKE:-} "
if [[ "$like" == *" arch "* || "$like" == *" cachyos "* || "$like" == *" manjaro "* || "$like" == *" endeavouros "* ]]; then
    PKGS=(base-devel rust webkit2gtk-4.1 gtk3 libayatana-appindicator ladspa pipewire pipewire-pulse wireplumber libpulse)
    have() { pacman -Q "$1" >/dev/null 2>&1 || { [[ "$1" == rust ]] && command -v cargo >/dev/null; }; }
    INSTALL=(sudo pacman -S --needed --noconfirm)
elif [[ "$like" == *" fedora "* || "$like" == *" rhel "* ]]; then
    PKGS=(gcc cargo rust webkit2gtk4.1-devel gtk3-devel libayatana-appindicator-gtk3-devel ladspa-devel pipewire-utils pulseaudio-utils wireplumber)
    have() { rpm -q "$1" >/dev/null 2>&1 || { [[ "$1" == cargo || "$1" == rust ]] && command -v cargo >/dev/null; }; }
    INSTALL=(sudo dnf install -y)
elif [[ "$like" == *" debian "* || "$like" == *" ubuntu "* ]]; then
    PKGS=(build-essential pkg-config cargo libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev ladspa-sdk pipewire-bin pulseaudio-utils wireplumber)
    have() { dpkg -s "$1" >/dev/null 2>&1 || { [[ "$1" == cargo ]] && command -v cargo >/dev/null; }; }
    INSTALL=(sudo apt-get install -y)
else
    PKGS=(); warn "Unknown distro '${ID:-?}': install Rust (cargo), a C compiler, webkit2gtk-4.1, gtk3, ayatana-appindicator, the LADSPA SDK header, PipeWire (+ pactl, pw-cli) yourself."
fi
MISSING=()
for p in "${PKGS[@]}"; do have "$p" || MISSING+=("$p"); done
if [[ ${#MISSING[@]} -gt 0 ]]; then
    if [[ $DEPS == 1 ]]; then
        echo "Installing: ${MISSING[*]}"
        "${INSTALL[@]}" "${MISSING[@]}"
    else
        warn "Missing packages (not installing, --no-deps): ${MISSING[*]}"
    fi
else
    echo "All packages present."
fi
command -v cargo >/dev/null || { echo "cargo (Rust) is required. Install it (https://rustup.rs) and re-run."; exit 1; }

# ---------------------------------------------------------------- 2. openrazer driver (not installed by this script)
if ! ls -d /sys/bus/hid/drivers/razer* >/dev/null 2>&1; then
    warn "No openrazer kernel driver is loaded. Lynapse talks to devices through it."
    echo "   Most Razer devices: install openrazer (https://openrazer.github.io), e.g. 'openrazer-driver-dkms'."
    echo "   BlackShark V3 Pro: needs the patched driver from https://github.com/Stealthee/openrazer-v3pro-custom-driver"
    echo "   (Lynapse still installs; devices appear once the driver is loaded.)"
fi

# ---------------------------------------------------------------- 3. build
say "Building Lynapse 2.0 (first build takes a few minutes)"
( cd src-tauri && cargo build --release )

# ---------------------------------------------------------------- 4. install the app
say "Installing the app"
BIN="$HOME/.local/bin"; mkdir -p "$BIN"
pkill -9 -x razerctl 2>/dev/null || true
install -m755 src-tauri/target/release/razerctl "$BIN/razerctl.new" && mv "$BIN/razerctl.new" "$BIN/razerctl"
install -Dm644 src-tauri/icons/icon.png "$HOME/.local/share/icons/hicolor/256x256/apps/razerctl.png"
DESK="[Desktop Entry]
Type=Application
Name=Lynapse 2.0
Comment=Razer device settings for Linux (Synapse-style)
Exec=$BIN/razerctl
Icon=razerctl
Categories=Settings;HardwareSettings;
"
install -Dm644 /dev/stdin "$HOME/.local/share/applications/razerctl.desktop" <<< "$DESK"
if [[ $AUTOSTART == 1 ]]; then
    install -Dm644 /dev/stdin "$HOME/.config/autostart/razerctl.desktop" <<< "[Desktop Entry]
Type=Application
Name=Lynapse 2.0
Exec=$BIN/razerctl --hidden
Icon=razerctl
X-GNOME-Autostart-enabled=true
"
fi
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

# ---------------------------------------------------------------- 5. Spatial Audio (optional)
PWCONF="$HOME/.config/pipewire/pipewire.conf.d/60-razerctl-thx.conf"
RESTART_PW=0
if [[ $SPATIAL == 1 ]]; then
    say "Installing Spatial Audio"
    [[ -f "$HRIR_SRC" ]] || { echo "HRIR file not found: $HRIR_SRC"; exit 1; }
    HDIR="$HOME/.local/share/razerctl/thx"; mkdir -p "$HDIR"
    HRIR="$HDIR/$(basename "$HRIR_SRC")"
    [[ "$HRIR_SRC" -ef "$HRIR" ]] || cp "$HRIR_SRC" "$HRIR"
    # send the filter's output to the headset's game output if we can find it, else to the default output
    GAME=$(pactl list short sinks 2>/dev/null | awk '{print $2}' | grep -i '^alsa_output.*razer' | grep -vi chat | head -1 || true)
    if [[ -n "$GAME" ]]; then TL="target.object  = \"$GAME\""; echo "Output: $GAME"
    else TL="# target.object: no Razer headset found at install time, using the default output"; warn "No Razer headset output found; Spatial Audio will play to your default output. Re-run with the headset connected to pin it."; fi
    [[ -f "$PWCONF" ]] && cp "$PWCONF" "$PWCONF.bak" && echo "Old filter config saved as $PWCONF.bak"
    mkdir -p "$(dirname "$PWCONF")"
    sed -e "s#@HRIR@#$HRIR#g" -e "s#@TARGET_LINE@#$TL#" packaging/60-razerctl-thx.conf.in > "$PWCONF"
    RESTART_PW=1
else
    if [[ -f "$PWCONF" ]]; then echo "Keeping your existing Spatial Audio filter ($PWCONF)."; fi
fi

if [[ $RESTART_PW == 1 ]]; then
    say "Restarting PipeWire to load the filter (audio blips for a second)"
    systemctl --user restart pipewire pipewire-pulse wireplumber || warn "Could not restart PipeWire; log out and back in."
fi

# ---------------------------------------------------------------- 6. start
say "Starting Lynapse 2.0"
( setsid "$BIN/razerctl" >/dev/null 2>&1 & )
cat <<EOF

Done. Lynapse 2.0 is in your app menu and system tray$([[ $AUTOSTART == 1 ]] && echo ", and starts at login").
  Spatial Audio: $([[ $SPATIAL == 1 ]] && echo "installed (switch it on in the headset's SOUND tab)" || echo "not installed (re-run ./install-with-spatial.sh to add it)")
  Audio / Mic Enhancement: built in (headset ENHANCEMENT and MIC tabs; the first switch-on restarts PipeWire once)
  Uninstall: ./uninstall.sh
EOF
