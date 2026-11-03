// razerctl: Synapse-style control app for Razer devices on Linux.
// Backend = device discovery (sysfs, hotplug by polling), generic attr read/write,
// PipeWire volume via pactl, small JSON state store, tray icon.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod remap;
mod calib;
mod smartpoll;
mod battray;
mod quick;
mod osd;
mod gameprof;
mod enhance;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};

const DRIVERS: &[(&str, &str)] = &[
    ("razermouse", "mouse"),
    ("razerkraken", "headset"),
    ("razerkbd", "keyboard"),
    ("razeraccessory", "accessory"),
    ("razercore", "accessory"),
    ("razermousemat", "accessory"),
];

#[derive(Serialize, Clone, PartialEq, Debug)]
struct Iface {
    path: String,
    pid: String,
    conn: String, // "wired" | "wireless" | ""
    alive: bool,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
struct Device {
    id: String,
    name: String,
    kind: String,
    ifaces: Vec<Iface>,
    active: Option<String>,
    conn: String,
    attrs: Vec<String>,
    battery: Option<u8>,
    charging: bool,
    firmware: String,
    serial: String,
    /// one of the devices razerctl has full Synapse-style pages for; everything else gets the basic GENERAL page
    focused: bool,
    /// false = Razer USB device with no openrazer driver bound (only name + kernel battery are known)
    driver: bool,
}

/// Devices with full pages: Viper V3 Pro (wired 00c0 / dongle 00c1), BlackShark V3 Pro (dongle 0577 / wired 0576).
const FOCUSED_PIDS: &[&str] = &["00c0", "00c1", "0577", "0576"];

impl Device {
    /// sysfs dir of the focused device of this kind (the one the Viper/BlackShark-specific features drive)
    pub(crate) fn focused_active(devs: &[Device], kind: &str) -> Option<String> {
        devs.iter().find(|d| d.kind == kind && d.focused).and_then(|d| d.active.clone())
    }
}

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) devices: Mutex<Vec<Device>>,
    tray: Mutex<Option<TrayIcon>>,
    demo: Mutex<Option<std::process::Child>>,
    remap: Arc<remap::Remapper>,
}

/// sysfs dir of the connected Razer mouse (for DPI actions).
fn mouse_sysfs_fn(st: Arc<AppState>) -> Arc<dyn Fn() -> Option<String> + Send + Sync> {
    Arc::new(move || Device::focused_active(&st.devices.lock(), "mouse"))
}

#[tauri::command]
fn remap_get(state: tauri::State<'_, Arc<AppState>>) -> serde_json::Value {
    serde_json::json!({ "config": *state.remap.config.lock(), "status": *state.remap.status.lock() })
}

fn bat_infos(devs: &[Device]) -> Vec<battray::DevInfo> {
    devs.iter()
        .map(|d| battray::DevInfo { id: d.id.clone(), name: d.name.clone(), kind: d.kind.clone(), battery: d.battery, charging: d.charging, active: d.active.is_some() })
        .collect()
}

#[tauri::command]
fn battray_get() -> Vec<String> {
    battray::enabled()
}

#[tauri::command]
fn battray_set(app: AppHandle, state: tauri::State<'_, Arc<AppState>>, id: String, on: bool) {
    battray::set(&id, on);
    battray::sync(&app, bat_infos(&state.devices.lock()));
}

#[tauri::command]
fn smartpoll_get() -> serde_json::Value {
    smartpoll::status()
}

#[tauri::command]
fn smartpoll_set(config: smartpoll::Config) -> Result<(), String> {
    smartpoll::set(config)
}

#[tauri::command]
fn calib_start() -> Result<usize, String> {
    calib::start()
}

#[tauri::command]
fn calib_read() -> Vec<calib::Reading> {
    calib::read()
}

#[tauri::command]
fn calib_stop() {
    calib::stop()
}

#[tauri::command]
fn macro_play(state: tauri::State<'_, Arc<AppState>>, id: String) {
    state.remap.spawn_play("test".into(), id, Some(1));
}

#[tauri::command]
fn macro_stop(state: tauri::State<'_, Arc<AppState>>) {
    state.remap.stop_play("test");
}

#[tauri::command]
fn remap_set(state: tauri::State<'_, Arc<AppState>>, config: remap::Config) -> Result<(), String> {
    let f = dirs::config_dir().unwrap_or_default().join("razerctl/remap.json");
    fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&f, serde_json::to_vec_pretty(&config).unwrap()).map_err(|e| e.to_string())?;
    let st = state.inner().clone();
    st.remap.set_config(config, mouse_sysfs_fn(st.clone()));
    Ok(())
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn split_conn(name: &str) -> (String, String) {
    for (suffix, conn) in [
        (" (Wired)", "wired"),
        (" (Wireless)", "wireless"),
        (" (Receiver)", "wireless"),
        (" (USB)", "wired"),
    ] {
        if let Some(base) = name.strip_suffix(suffix) {
            return (base.to_string(), conn.to_string());
        }
    }
    (name.to_string(), String::new())
}

// ---- liveness / battery probing ----
// Reading charge_level etc. is a USB round-trip. A device that is enumerated but not answering
// (mouse switched away on the KVM, wireless mouse asleep) makes every read block ~5 s inside the
// driver, and those reads cannot be interrupted. So probes run on detached threads, at most one
// in flight per interface, and dead interfaces are retried only every 30 s. scan() itself never
// touches the USB link; it uses the cached probe results.
#[derive(Clone, Default)]
struct Probe {
    inflight: bool,
    alive: bool,
    last: Option<std::time::Instant>,
    // when this interface first appeared; a just-plugged device is retried every 2 s for a while
    // because its first reads often fail while the driver is still binding
    first: Option<std::time::Instant>,
    battery: Option<u8>,
    charging: bool,
    firmware: String,
    serial: String,
}

static PROBES: std::sync::LazyLock<Mutex<std::collections::HashMap<String, Probe>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn probe_async(path: &Path) {
    let key = path.to_string_lossy().to_string();
    {
        let mut m = PROBES.lock();
        let p = m.entry(key.clone()).or_default();
        let fresh = p.first.get_or_insert_with(std::time::Instant::now).elapsed() < Duration::from_secs(30);
        let every = if p.alive { Duration::from_secs(10) } else if fresh { Duration::from_secs(2) } else { Duration::from_secs(30) };
        if p.inflight || p.last.map(|t| t.elapsed() < every).unwrap_or(false) {
            return;
        }
        p.inflight = true;
        p.last = Some(std::time::Instant::now());
    }
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let probe = if path.join("charge_level").exists() { "charge_level" } else { "firmware_version" };
        let t0 = std::time::Instant::now();
        let first = fs::read_to_string(path.join(probe));
        // a read that only succeeded after a long stall counts as dead too (half-present device)
        let alive = first.is_ok() && t0.elapsed() < Duration::from_secs(2);
        let mut r = Probe { alive, ..Default::default() };
        if alive {
            if let Some(v) = read_trim(&path.join("charge_level")).and_then(|s| s.parse::<u32>().ok()) {
                r.battery = Some(((v * 100 + 127) / 255).min(100) as u8);
            }
            r.charging = read_trim(&path.join("charge_status")).as_deref() == Some("1");
            r.firmware = read_trim(&path.join("firmware_version")).unwrap_or_default();
            // razerkraken has no firmware query for the BlackShark V3 family and just reports a
            // fixed "v1.0" placeholder, so don't show it.
            if r.firmware == "v1.0" && path.to_string_lossy().contains("razerkraken") {
                r.firmware.clear();
            }
            r.serial = read_trim(&path.join("device_serial"))
                .filter(|s| !s.is_empty() && s != "-1" && !s.chars().all(|c| c == '0'))
                .unwrap_or_default();
        }
        let mut m = PROBES.lock();
        let p = m.entry(path.to_string_lossy().to_string()).or_default();
        let (last, first) = (p.last, p.first);
        *p = r;
        p.last = last;
        p.first = first;
    });
}

fn scan() -> Vec<Device> {
    let mut map: BTreeMap<String, Device> = BTreeMap::new();
    for (drv, kind) in DRIVERS {
        let dir = Path::new("/sys/bus/hid/drivers").join(drv);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            // 0003:1532:00C0.001A
            let parts: Vec<&str> = fname.split(':').collect();
            if parts.len() != 3 || parts[1] != "1532" {
                continue;
            }
            let p = e.path();
            let Some(dtype) = read_trim(&p.join("device_type")) else { continue };
            if dtype.is_empty() {
                continue;
            }
            let pid = parts[2].split('.').next().unwrap_or("").to_lowercase();
            let (base, conn) = split_conn(&dtype);
            probe_async(&p);
            let alive = PROBES.lock().get(&*p.to_string_lossy()).map(|x| x.alive).unwrap_or(false);
            let id = format!("{}:{}", kind, base);
            let dev = map.entry(id.clone()).or_insert_with(|| Device {
                id,
                name: base.clone(),
                kind: kind.to_string(),
                ifaces: vec![],
                active: None,
                conn: String::new(),
                attrs: vec![],
                battery: None,
                charging: false,
                firmware: String::new(),
                serial: String::new(),
                focused: false,
                driver: true,
            });
            // RAZERCTL_GENERIC=1: preview the basic GENERAL page on the focused devices too
            dev.focused |= FOCUSED_PIDS.contains(&pid.as_str()) && std::env::var_os("RAZERCTL_GENERIC").is_none();
            dev.ifaces.push(Iface { path: p.to_string_lossy().to_string(), pid, conn, alive });
        }
    }
    let mut out = vec![];
    for (_, mut d) in map {
        d.ifaces.sort_by(|a, b| a.path.cmp(&b.path));
        // Prefer an alive wired interface, then alive wireless.
        let pick = d
            .ifaces
            .iter()
            .filter(|i| i.alive)
            .min_by_key(|i| if i.conn == "wired" { 0 } else { 1 })
            .cloned();
        if let Some(i) = pick {
            let p = PathBuf::from(&i.path);
            d.active = Some(i.path.clone());
            d.conn = i.conn.clone();
            if let Ok(rd) = fs::read_dir(&p) {
                let mut a: Vec<String> = rd
                    .flatten()
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                a.sort();
                d.attrs = a;
            }
            if let Some(pr) = PROBES.lock().get(&i.path) {
                d.battery = pr.battery;
                d.charging = pr.charging;
                d.firmware = pr.firmware.clone();
                d.serial = pr.serial.clone();
            }
        }
        out.push(d);
    }
    usb_only_devices(&mut out);
    out
}

/// Razer (1532) USB devices that no openrazer driver has claimed: a tab with the basics (name, kernel battery).
fn usb_only_devices(out: &mut Vec<Device>) {
    let known: Vec<String> = out.iter().flat_map(|d| d.ifaces.iter().map(|i| i.pid.clone())).collect();
    let Ok(rd) = fs::read_dir("/sys/bus/usb/devices") else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.contains(':') || read_trim(&p.join("idVendor")).as_deref() != Some("1532") { continue }
        let pid = read_trim(&p.join("idProduct")).unwrap_or_default().to_lowercase();
        if known.contains(&pid) { continue }
        // kind from the interfaces: audio class = headset, HID boot mouse/keyboard
        let mut kind = "accessory";
        for i in fs::read_dir(&p).into_iter().flatten().flatten() {
            let ip = i.path();
            if !i.file_name().to_string_lossy().contains(':') { continue }
            let class = read_trim(&ip.join("bInterfaceClass")).unwrap_or_default();
            let proto = read_trim(&ip.join("bInterfaceProtocol")).unwrap_or_default();
            if class == "01" { kind = "headset"; break }
            if class == "03" && proto == "02" && kind == "accessory" { kind = "mouse" }
            if class == "03" && proto == "01" && kind == "accessory" { kind = "keyboard" }
        }
        let product = read_trim(&p.join("product")).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Razer device {pid}"));
        let product = if product.starts_with("Razer") { product } else { format!("Razer {product}") };
        let (battery, charging) = kernel_battery(&p);
        let path = p.to_string_lossy().to_string();
        out.push(Device {
            id: format!("{kind}:{product}"),
            name: product,
            kind: kind.into(),
            ifaces: vec![Iface { path: path.clone(), pid: pid.clone(), conn: String::new(), alive: true }],
            active: Some(path),
            conn: String::new(),
            attrs: vec![],
            battery,
            charging,
            firmware: read_trim(&p.join("bcdDevice")).map(|v| format!("bcd {v}")).unwrap_or_default(),
            serial: read_trim(&p.join("serial")).filter(|s| !s.chars().all(|c| c == '0')).unwrap_or_default(),
            focused: false,
            driver: false,
        });
    }
}

/// Battery from a kernel power_supply whose device sits under this USB device (hid-*-battery etc.).
fn kernel_battery(usb: &Path) -> (Option<u8>, bool) {
    let Ok(base) = fs::canonicalize(usb) else { return (None, false) };
    for e in fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten() {
        let ps = e.path();
        let Ok(dev) = fs::canonicalize(ps.join("device")) else { continue };
        if !dev.starts_with(&base) { continue }
        let cap = read_trim(&ps.join("capacity")).and_then(|v| v.parse::<u8>().ok());
        if cap.is_some() {
            return (cap.map(|c| c.min(100)), read_trim(&ps.join("status")).as_deref() == Some("Charging"));
        }
    }
    (None, false)
}

fn valid_target(state: &AppState, path: &str, attr: &str) -> Result<PathBuf, String> {
    if attr.is_empty() || !attr.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err("bad attribute name".into());
    }
    let ok = state.devices.lock().iter().any(|d| d.ifaces.iter().any(|i| i.path == path));
    if !ok {
        return Err("unknown device".into());
    }
    Ok(Path::new(path).join(attr))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn devices(state: tauri::State<'_, Arc<AppState>>) -> Vec<Device> {
    state.devices.lock().clone()
}

#[tauri::command]
async fn read_attr(state: tauri::State<'_, Arc<AppState>>, path: String, attr: String) -> Result<String, String> {
    let p = valid_target(&state, &path, &attr)?;
    blocking(move || fs::read_to_string(&p).map(|s| s.trim().to_string()).map_err(|e| format!("{attr}: {e}"))).await
}

#[tauri::command]
async fn read_attr_bytes(state: tauri::State<'_, Arc<AppState>>, path: String, attr: String) -> Result<Vec<u8>, String> {
    let p = valid_target(&state, &path, &attr)?;
    blocking(move || fs::read(&p).map_err(|e| format!("{attr}: {e}"))).await
}

#[tauri::command]
async fn write_attr(state: tauri::State<'_, Arc<AppState>>, path: String, attr: String, value: String) -> Result<(), String> {
    let p = valid_target(&state, &path, &attr)?;
    blocking(move || {
        let mut f = fs::OpenOptions::new().write(true).open(&p).map_err(|e| format!("{attr}: {e}"))?;
        f.write_all(value.as_bytes()).map_err(|e| format!("{attr}: {e}"))
    })
    .await
}

#[tauri::command]
async fn write_attr_bytes(state: tauri::State<'_, Arc<AppState>>, path: String, attr: String, value: Vec<u8>) -> Result<(), String> {
    let p = valid_target(&state, &path, &attr)?;
    blocking(move || {
        let mut f = fs::OpenOptions::new().write(true).open(&p).map_err(|e| format!("{attr}: {e}"))?;
        f.write_all(&value).map_err(|e| format!("{attr}: {e}"))
    })
    .await
}

// ---------- PipeWire volume via pactl ----------
pub(crate) fn pactl_find(kind: &str, needle: &str) -> Option<String> {
    let what = if kind == "mic" { "sources" } else { "sinks" };
    let out = Command::new("pactl").args(["list", "short", what]).output().ok()?;
    let txt = String::from_utf8_lossy(&out.stdout);
    let needle = needle.to_lowercase();
    txt.lines()
        .filter_map(|l| l.split('\t').nth(1))
        .filter(|n| n.starts_with("alsa_") && !n.ends_with(".monitor") && n.to_lowercase().contains(&needle))
        .find(|n| match kind {
            "chat" => n.contains("chat") && !n.starts_with("alsa_input"),
            "game" => !n.contains("chat"),
            _ => true,
        })
        .map(|s| s.to_string())
}

#[tauri::command]
async fn audio_get(kind: String, needle: String) -> Result<Option<u32>, String> {
    blocking(move || {
        let Some(name) = pactl_find(&kind, &needle) else { return Ok(None) };
        let sub = if kind == "mic" { "get-source-volume" } else { "get-sink-volume" };
        let out = Command::new("pactl").args([sub, &name]).output().map_err(|e| e.to_string())?;
        let t = String::from_utf8_lossy(&out.stdout);
        let pct = t.split('/').nth(1).and_then(|s| s.trim().trim_end_matches('%').trim().parse::<u32>().ok());
        Ok(pct)
    })
    .await
}

#[tauri::command]
async fn audio_set(kind: String, needle: String, value: u32) -> Result<(), String> {
    blocking(move || {
        let name = pactl_find(&kind, &needle).ok_or("audio device not found")?;
        let sub = if kind == "mic" { "set-source-volume" } else { "set-sink-volume" };
        Command::new("pactl").args([sub, &name, &format!("{}%", value.min(150))]).status().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

// ---------- THX (software HRIR sinks set up by Lynapse / PipeWire) + demo player ----------
fn sink_list() -> Vec<String> {
    Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.split('\t').nth(1).map(String::from)).collect())
        .unwrap_or_default()
}

#[derive(Serialize)]
struct Engine {
    id: String,
    sink: String,
    label: String,
}

const THX_SINK: &str = "effect_input.razerctl-thx";
const THX_SINK_OLD: &str = "effect_input.lynapse-thx";

fn engines() -> Vec<Engine> {
    let mut v = vec![];
    for s in sink_list() {
        // razerctl-thx is the current name; lynapse-thx is the same filter before it moved here
        if s == THX_SINK || (s == THX_SINK_OLD && !v.iter().any(|e: &Engine| e.id == "lynapse")) {
            v.insert(0, Engine { id: "lynapse".into(), sink: s, label: "Spatial Audio (HRIR)".into() });
        } else if s.starts_with("effect_input.") && s.contains("virtual-surround") {
            v.push(Engine { id: "vs71".into(), sink: s, label: "Virtual Surround 7.1".into() });
        }
    }
    v
}

/// True when the PipeWire THX convolver sink (60-razerctl-thx.conf, or Lynapse's older one) exists.
pub(crate) fn thx_available() -> bool {
    let sinks = sink_list();
    [THX_SINK, THX_SINK_OLD].iter().any(|n| sinks.iter().any(|s| s == n))
}

#[tauri::command]
async fn thx_ok() -> Result<bool, String> {
    blocking(|| Ok(thx_available())).await
}

#[tauri::command]
async fn thx_engines() -> Result<Vec<Engine>, String> {
    blocking(|| Ok(engines())).await
}

/// sink-input ids currently playing on `sink` (by name), optionally only those of process `pid`.
fn sink_inputs(sink: Option<&str>, pid: Option<u32>) -> Vec<String> {
    let sinks = Command::new("pactl").args(["list", "short", "sinks"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let sink_id = sink.and_then(|n| sinks.lines().find(|l| l.split('\t').nth(1) == Some(n)).and_then(|l| l.split('\t').next()).map(String::from));
    let out = Command::new("pactl").args(["list", "sink-inputs"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let mut res = vec![];
    for block in out.split("Sink Input #").skip(1) {
        let id = block.lines().next().unwrap_or("").trim().to_string();
        let on_sink = block.lines().find_map(|l| l.trim().strip_prefix("Sink: ")).map(|s| s.trim().to_string());
        if let Some(sid) = &sink_id {
            if on_sink.as_deref() != Some(sid.as_str()) { continue; }
        }
        if block.contains("node.name = \"effect_output.") {
            continue; // a filter-chain's own output stream (THX / virtual surround) - never move it
        }
        if let Some(p) = pid {
            if !block.contains(&format!("application.process.id = \"{}\"", p)) { continue; }
        }
        res.push(id);
    }
    res
}

/// Software THX like Lynapse: move game-sink streams onto the THX convolver sink (on) or back (off).
#[tauri::command]
async fn thx_route(on: bool, needle: String) -> Result<(), String> {
    blocking(move || thx_route_sync(on, &needle)).await
}

pub(crate) fn thx_route_sync(on: bool, needle: &str) -> Result<(), String> {
    {
        let game = pactl_find("game", needle).ok_or("headset sink not found")?;
        // with playback enhancement on, apps sit on the enhanced sink (and THX's output is fed into it)
        let game = enhance::game_front(&game);
        let sinks = sink_list();
        let Some(thx) = [THX_SINK, THX_SINK_OLD].into_iter().find(|n| sinks.iter().any(|s| s == n)) else {
            return Ok(());
        };
        let (from, to) = if on { (game.as_str(), thx) } else { (thx, game.as_str()) };
        for id in sink_inputs(Some(from), None) {
            // never move the THX filter's own output stream back onto itself
            let _ = Command::new("pactl").args(["move-sink-input", &id, to]).status();
        }
        Ok(())
    }
}

static DEMO_FLAC: &[u8] = include_bytes!("../assets/demo.flac");

#[tauri::command]
async fn demo_play(state: tauri::State<'_, Arc<AppState>>, sink: String) -> Result<f64, String> {
    let st = state.inner().clone();
    blocking(move || {
        if let Some(mut c) = st.demo.lock().take() { let _ = c.kill(); let _ = c.wait(); }
        let f = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("razerctl/demo.flac");
        if fs::metadata(&f).map(|m| m.len() as usize != DEMO_FLAC.len()).unwrap_or(true) {
            fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::write(&f, DEMO_FLAC).map_err(|e| e.to_string())?;
        }
        let child = Command::new("pw-play")
            .args(["--target", &sink, "--media-role", "Game", "-P", "{ media.name = \"razerctl demo\" }"])
            .arg(&f)
            .spawn()
            .map_err(|e| format!("pw-play: {e}"))?;
        *st.demo.lock() = Some(child);
        Ok(25.9)
    })
    .await
}

#[tauri::command]
async fn demo_move(state: tauri::State<'_, Arc<AppState>>, sink: String) -> Result<(), String> {
    let pid = state.demo.lock().as_ref().map(|c| c.id());
    let Some(pid) = pid else { return Ok(()) };
    blocking(move || {
        for id in sink_inputs(None, Some(pid)) {
            Command::new("pactl").args(["move-sink-input", &id, &sink]).status().map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
fn demo_stop(state: tauri::State<'_, Arc<AppState>>) {
    if let Some(mut c) = state.demo.lock().take() { let _ = c.kill(); let _ = c.wait(); }
}

#[tauri::command]
fn game_sink(needle: String) -> Option<String> {
    pactl_find("game", &needle)
}

#[tauri::command]
fn lynapse_config() -> serde_json::Value {
    // razerctl keeps its own copy (~/.config/razerctl/lynapse-import.json) so this works without Lynapse
    let c = dirs::config_dir().unwrap_or_default();
    fs::read_to_string(c.join("razerctl/lynapse-import.json"))
        .or_else(|_| fs::read_to_string(c.join("lynapse.json")))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
fn open_settings(which: String) -> Result<(), String> {
    let (cmd, args): (&str, Vec<&str>) = match which.as_str() {
        "mouse" => ("systemsettings", vec!["kcm_mouse"]),
        "sound" => ("systemsettings", vec!["kcm_pulseaudio"]),
        "mixer" => ("pavucontrol", vec![]),
        _ => return Err("unknown".into()),
    };
    Command::new(cmd).args(args).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

// Settings the driver can write but not read back. The device may forget them on a replug or
// power-cycle, so the last value chosen in the app is re-sent whenever the device (re)connects.
const WRITE_ONLY: &[&str] = &["hyperpolling_wireless_dongle_indicator_led_mode"];

fn reapply_write_only(devs: &[Device], seen: &mut std::collections::HashMap<String, String>) {
    for d in devs {
        let Some(path) = d.active.clone() else {
            seen.remove(&d.id);
            continue;
        };
        if seen.get(&d.id) == Some(&path) {
            continue;
        }
        seen.insert(d.id.clone(), path.clone());
        let st = state_load();
        for attr in WRITE_ONLY {
            let Some(v) = st.get(&d.id).and_then(|x| x.get(*attr)).and_then(|v| v.as_str().map(String::from)) else { continue };
            let f = PathBuf::from(&path).join(attr);
            if f.exists() {
                std::thread::spawn(move || {
                    // give the link a moment after connect
                    std::thread::sleep(Duration::from_millis(1500));
                    let _ = fs::write(&f, v);
                });
            }
        }
    }
}

// ---------- state store (~/.config/razerctl/state.json) ----------
fn state_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("razerctl/state.json")
}

#[tauri::command]
fn state_load() -> serde_json::Value {
    fs::read_to_string(state_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

#[tauri::command]
fn state_save(data: serde_json::Value) -> Result<(), String> {
    let f = state_file();
    fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = f.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&data).unwrap()).map_err(|e| e.to_string())?;
    fs::rename(tmp, f).map_err(|e| e.to_string())
}

// ---------- autostart ----------
fn autostart_file() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("autostart/razerctl.desktop")
}

#[tauri::command]
fn start_page() -> Option<String> {
    std::env::var("RAZERCTL_PAGE").ok()
}

#[tauri::command]
fn autostart_get() -> bool {
    autostart_file().exists()
}

#[tauri::command]
fn autostart_set(on: bool) -> Result<(), String> {
    let f = autostart_file();
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(
            &f,
            format!("[Desktop Entry]\nType=Application\nName=Lynapse 2.0\nExec={} --hidden\nIcon=razerctl\nX-GNOME-Autostart-enabled=true\n", exe.display()),
        )
        .map_err(|e| e.to_string())
    } else {
        let _ = fs::remove_file(f);
        Ok(())
    }
}

// ---------- tray ----------
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Hover text of the main tray icon: app name + battery of each connected device.
fn tray_title(devs: &[Device]) -> String {
    let mut s = format!("Lynapse 2.0 (v{})", env!("CARGO_PKG_VERSION"));
    for d in devs.iter().filter(|d| d.active.is_some()) {
        let name = d.name.trim_start_matches("Razer ");
        match d.battery {
            Some(b) => s += &format!("\n{name}: {b}%{}", if d.charging { " (charging)" } else { "" }),
            None => s += &format!("\n{name}: connected"),
        }
    }
    if !devs.iter().any(|d| d.active.is_some()) { s += "\nNo devices connected" }
    s
}

fn build_menu(app: &AppHandle, devs: &[Device]) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    if devs.is_empty() {
        menu.append(&MenuItem::with_id(app, "none", "No devices", false, None::<&str>)?)?;
    }
    for (i, d) in devs.iter().enumerate() {
        let status = match (d.active.is_some(), d.battery) {
            (false, _) => "offline".to_string(),
            (true, Some(b)) => format!("{}%{}", b, if d.charging { " ⚡" } else { "" }),
            (true, None) => "connected".to_string(),
        };
        let label = format!("{}   {}", d.name, status);
        menu.append(&MenuItem::with_id(app, format!("dev:{}", d.id), label, true, None::<&str>)?)?;
        if let (Some(dir), "headset") = (d.active.as_deref(), d.kind.as_str()) {
            // Lynapse-style quick settings listed straight under the headset's line: only one
            // submenu level, since KDE closes nested submenus easily when the pointer drifts.
            for it in quick::headset_items(app, dir) {
                menu.append(it.as_ref())?;
            }
            if i + 1 < devs.len() {
                menu.append(&PredefinedMenuItem::separator(app)?)?;
            }
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "show", "Open Lynapse 2.0", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?)?;
    Ok(menu)
}

fn main() {
    if let Ok(p) = std::env::var("RAZERCTL_BATICON") {
        battray::preview(&p);
        return;
    }
    let hidden = std::env::args().any(|a| a == "--hidden");
    let state = Arc::new(AppState::default());
    *state.devices.lock() = scan();
    if let Some(cfg) = fs::read_to_string(dirs::config_dir().unwrap_or_default().join("razerctl/remap.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<remap::Config>(&s).ok())
    {
        *state.remap.config.lock() = cfg;
    }
    state.remap.ensure(mouse_sysfs_fn(state.clone()));
    smartpoll::init(mouse_sysfs_fn(state.clone()));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            devices, read_attr, read_attr_bytes, write_attr, write_attr_bytes,
            audio_get, audio_set, open_settings, state_load, state_save, start_page, thx_engines, thx_ok, thx_route, demo_play, demo_move, demo_stop, game_sink, lynapse_config, remap_get, remap_set, macro_play, macro_stop, calib_start, calib_read, calib_stop, smartpoll_get, smartpoll_set, battray_get, battray_set, autostart_get, autostart_set, osd::osd_screens, osd::osd_get, osd::osd_set, osd::osd_preview, gameprof::gp_list, gameprof::gp_save, enhance::enh_get, enhance::enh_set
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let devs = state.devices.lock().clone();
            let tray = TrayIconBuilder::with_id("main")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .tooltip(concat!("Lynapse 2.0 (v", env!("CARGO_PKG_VERSION"), ")"))
                .title(tray_title(&devs))
                .menu(&build_menu(&handle, &devs)?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| {
                    let id = ev.id().as_ref().to_string();
                    if id == "quit" {
                        app.exit(0);
                    } else if id == "show" {
                        show_main(app);
                    } else if let Some(dev) = id.strip_prefix("dev:") {
                        show_main(app);
                        let _ = app.emit("select-device", dev.to_string());
                    } else if id.starts_with("q:") {
                        let st = app.state::<Arc<AppState>>();
                        let dir = Device::focused_active(&st.devices.lock(), "headset");
                        if let Some(dir) = dir {
                            quick::handle(app, &dir, &id);
                            // show the new value next time the menu opens
                            let devs = st.devices.lock().clone();
                            if let (Some(t), Ok(m)) = (st.tray.lock().as_ref(), build_menu(app, &devs)) {
                                let _ = t.set_menu(Some(m));
                            }
                        }
                    }
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                show_main(app);
                            }
                        }
                    }
                })
                .build(app)?;
            *state.tray.lock() = Some(tray);

            if !hidden {
                show_main(&handle);
            }
            battray::sync(&handle, bat_infos(&devs));

            // Audio/Mic Enhancement: put the saved state back once the headset is in PipeWire.
            {
                let st = state.clone();
                enhance::restore(move || {
                    st.devices.lock().iter().find(|d| d.kind == "headset" && d.focused).map(|d| d.name.trim_start_matches("Razer ").replace(' ', "_"))
                });
            }

            // Scroll-wheel pop-ups near the tray.
            {
                let st = state.clone();
                let h = handle.clone();
                let st2 = state.clone();
                gameprof::start(Arc::new(move || {
                    st2.devices.lock().iter().find(|d| d.kind == "headset" && d.focused).and_then(|d| {
                        d.active.clone().map(|dir| (dir, d.name.trim_start_matches("Razer ").replace(' ', "_")))
                    })
                }));
                osd::start(
                    handle.clone(),
                    Arc::new(move || Device::focused_active(&st.devices.lock(), "headset")),
                    // a hidden window can keep reporting focus on Wayland, so require it to be shown too
                    Arc::new(move || {
                        h.get_webview_window("main")
                            .map(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) && w.is_focused().unwrap_or(false))
                            .unwrap_or(false)
                    }),
                );
            }

            // Hotplug + battery poller.
            let st = state.clone();
            std::thread::spawn(move || {
                let mut tick: u32 = 0;
                let mut last_title = String::new();
                let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                reapply_write_only(&st.devices.lock().clone(), &mut seen);
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    tick += 1;
                    let new = scan();
                    st.remap.ensure(mouse_sysfs_fn(st.clone()));
                    let changed = {
                        let mut cur = st.devices.lock();
                        let ch = *cur != new;
                        if ch {
                            *cur = new.clone();
                        }
                        ch
                    };
                    battray::sync(&handle, bat_infos(&new));
                    let title = tray_title(&new);
                    if title != last_title {
                        if let Some(t) = st.tray.lock().as_ref() { let _ = t.set_title(Some(&title)); }
                        last_title = title;
                    }
                    reapply_write_only(&new, &mut seen);
                    if changed || tick % 15 == 0 {
                        let _ = handle.emit("devices", new.clone());
                        if let Some(t) = st.tray.lock().as_ref() {
                            if let Ok(m) = build_menu(&handle, &new) {
                                let _ = t.set_menu(Some(m));
                            }
                        }
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|w, ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running razerctl");
}

#[cfg(test)]
mod scan_dump {
    /// cargo test --release scan_dump -- --ignored --nocapture : print what scan() finds on this machine
    #[test]
    #[ignore]
    fn scan_dump() {
        for _ in 0..3 { super::scan(); std::thread::sleep(std::time::Duration::from_millis(1500)); }
        for d in super::scan() {
            println!("{} | {} | focused={} driver={} active={} battery={:?} attrs={}", d.id, d.kind, d.focused, d.driver, d.active.is_some(), d.battery, d.attrs.len());
        }
    }
}
