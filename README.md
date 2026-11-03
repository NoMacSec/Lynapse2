# Lynapse 2.0

**Razer device settings for Linux, laid out like Razer Synapse.** A small tray app (Rust + Tauri) that talks to your
devices through the [openrazer](https://openrazer.github.io) kernel driver and to your audio through PipeWire.

Built and tested with the **Razer BlackShark V3 Pro** (firmware 1.3.3) and **Razer Viper V3 Pro** (firmware 1.14).
Every other Razer mouse, headset or keyboard gets a basic page with whatever its driver supports.

> Not affiliated with, endorsed by or connected to Razer Inc., THX Ltd. or Dolby Laboratories.
> "Razer" and "Synapse" are trademarks of Razer Inc.

## Screenshots

| Dashboard | Macros |
|---|---|
| ![Dashboard](docs/screenshots/dashboard.png) | ![Macros](docs/screenshots/macros.png) |
| **BlackShark V3 Pro: Sound** | **BlackShark V3 Pro: EQ, game profiles, volume** |
| ![BlackShark V3 Pro Sound tab](docs/screenshots/headset-sound.png) | ![BlackShark V3 Pro EQ](docs/screenshots/headset-eq.png) |

**System tray:** hover the Lynapse icon for every device's battery; optional per-device battery icons (86, 100).

![Tray hover text and battery icons](docs/screenshots/tray.png)

## Features

**Razer BlackShark V3 Pro (full support)**
- **Sound:** 10-band headphone EQ with presets (stored on the headset), game/chat balance, volume, **Spatial Audio**
  (virtual surround, optional, see below)
- **Enhancement:** Audio Enhancement (Sound Normalization, Bass Boost, Voice Clarity), Ultra-Low Latency, ANC/Ambient + level,
  in-call audio mix
- **Mic:** mic volume, Mic Enhancement (Volume Normalization, Vocal Clarity, Mic Noise Cancellation, Voice Gate),
  mic EQ presets + 10 bands (Default / Esports / Broadcast / Flat), sidetone, audio prompts
- **Power:** sleep timer, dongle LED mode. **Demo:** a 7.1 test clip to hear Spatial Audio
- Tray quick settings, scroll-wheel pop-ups (game/chat balance, sidetone), per-game profiles (Spatial Audio + EQ while a game is focused)

**Razer Viper V3 Pro (full support)**
- DPI stages, polling rate up to 8000 Hz on the dongle, smart polling-rate switching for full-screen games
- Button remapping + Hypershift layer + macros (software, via evdev/uinput), sensitivity matcher
- Sleep timer, low-power threshold, dongle LED

**Any other Razer device (basic support)**
- Its own tab with name, connection, firmware, battery
- Mouse: DPI, polling rate, sleep timer, low-battery level
- Headset: volume and mic volume (PipeWire)
- Keyboard / mouse lighting: brightness and Off / Static (colour) / Spectrum / Wave / Breathing, per zone
- Only what the device's driver offers is shown. Want more specific support? Contact [Stealthee](https://github.com/Stealthee) (open an issue).

**Everywhere**
- Tray icon whose hover text lists every connected device's battery; optional per-device battery icons in the tray
- Profiles per device, autostart minimised to the tray

### About Audio / Mic Enhancement

In Synapse these run as THX software on the PC, not in the headset. Lynapse rebuilds them as PipeWire filter-chains whose
settings were **fitted to measurements of the real thing** (playback within ~0.4 dB, mic within ~2 dB). Like Synapse, one
mode per side is active at a time. Switching a side on routes your game audio / mic through
"BlackShark V3 Pro Game (Enhanced)" / "BlackShark V3 Pro Mic (Enhanced)". The first time, PipeWire restarts once.
The dynamics run in a small bundled LADSPA plugin (`dsp/razer_dyn.c`), compiled automatically during the build.

### About Spatial Audio

Spatial Audio is a PipeWire convolution filter that makes stereo game sound feel like it comes from around you. It
needs an **HRIR** (head-related impulse response) file. Lynapse ships a free one, `assets/spatial/lynapse-kemar.wav`,
built from the MIT KEMAR dummy-head measurements (see [credits](assets/spatial/CREDITS.md)).

It is **not** Razer's THX Spatial Audio (that is proprietary Windows software). You can use any other 14-channel
HeSuVi-format HRIR you are allowed to use: `./install-with-spatial.sh --hrir /path/to/file.wav`.

## Install

Requirements: a Linux desktop with **PipeWire** (+ WirePlumber), and the **openrazer kernel driver** for your devices.
Made on KDE Plasma (Wayland). The tray and window work elsewhere; the per-game features use KWin scripts.

1. **openrazer driver** (not installed by the script):
   - most Razer devices: install openrazer for your distro (for example `openrazer-driver-dkms` on Arch)
   - **BlackShark V3 Pro**: it needs the patched driver from
     [Stealthee/openrazer-v3pro-custom-driver](https://github.com/Stealthee/openrazer-v3pro-custom-driver)
2. **Lynapse:**
   ```sh
   git clone https://github.com/Stealthee/Lynapse2.git
   cd Lynapse2
   ./install-with-spatial.sh       # app + Spatial Audio (recommended for headsets)
   # or
   ./install-without-spatial.sh    # app only
   ```
   The script installs the build dependencies it is missing (Arch/CachyOS, Fedora, Debian/Ubuntu; it asks for your password
   for that), builds the app, installs it to `~/.local/bin/razerctl` with an app-menu entry and autostart, and with
   Spatial Audio also sets up the PipeWire filter. Connect the headset first so the filter is pointed at it.
   Options: `--no-deps`, `--no-autostart`, `--hrir FILE`. `./install.sh --help` lists them.

**Update:** `git pull` and run the same install script again.
**Uninstall:** `./uninstall.sh` (add `--purge` to also delete your settings).

## Troubleshooting

- **No device tab:** check the driver is loaded: `ls /sys/bus/hid/drivers/ | grep razer`.
- **Spatial Audio is greyed out:** the filter isn't loaded. Re-run `./install-with-spatial.sh` and check
  `journalctl --user -u pipewire -b | tail`.
- **Enhancement doesn't switch on:** `journalctl --user -u pipewire -b | tail` usually shows why.
- The app's internal name is still `razerctl` (binary, `~/.config/razerctl`, PipeWire node names).

## Building by hand

```sh
cd src-tauri && cargo build --release      # needs Rust, a C compiler, webkit2gtk-4.1, gtk3, ayatana-appindicator, ladspa.h
./target/release/razerctl
```
`dsp/enh.py` is the reference model for the enhancement chains and `dsp/make_hrir.py` regenerates the KEMAR HRIR.
`src-tauri/vendor/tray-icon` is tray-icon 0.25.1 with a one-line patch so the tray hover text works on Linux.

## Credits

- Created by **ShenaniganJoe** (GitHub: [Stealthee](https://github.com/Stealthee)). Want another device supported? Open an
  issue or email j71rivera@gmail.com with screenshots of the device's pages in Synapse. If you like Lynapse, you can
  donate to his Cash App: **$j71rivera**.
- Successor to **Lynapse** ([mehmetbayoglu/blackshark-control](https://github.com/mehmetbayoglu/blackshark-control),
  GPL-2.0-or-later), which some parts (mic EQ tables, the spatial filter) come from.
- [openrazer](https://github.com/openrazer/openrazer) for the device drivers.
- HRTF data: Bill Gardner and Keith Martin, MIT Media Lab, "HRTF Measurements of a KEMAR Dummy-Head Microphone" (1994).

## License

GPL-2.0-or-later. See [LICENSE](LICENSE). The bundled tray-icon patch keeps its MIT/Apache-2.0 license.
