// Pop-up when the headset's scroll wheel moves: watches the driver's cached game/chat balance and
// sidetone level (pure cache reads, no USB traffic).
//
// Where it shows (~/.config/razerctl/osd.json, "screen"):
//   "tray"  KDE notification, which KDE always puts near the tray (default)
//   "DP-3"  razerctl's own pop-up window, placed by a KWin script in the bottom-right of that output
//   "off"   nothing

use parking_lot::Mutex;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder};
use zbus::zvariant::Value;

const WIN_TITLE: &str = "razerctl OSD";
const SCRIPT_NAME: &str = "razerctl-osd";
const SCRIPT: &str = r#"// razerctl: keep the scroll-wheel pop-up window in the bottom-right of the chosen screen.
const TARGET = "%TARGET%";
function place(w) {
    if (!w || w.caption !== "razerctl OSD") return;
    const out = workspace.screens.find(s => s.name === TARGET) || workspace.activeScreen;
    const a = workspace.clientArea(KWin.PlacementArea, out, workspace.currentDesktop);
    const g = w.frameGeometry;
    w.frameGeometry = { x: a.x + a.width - g.width - 12, y: a.y + a.height - g.height - 12, width: g.width, height: g.height };
    w.keepAbove = true;
    w.skipTaskbar = true;
    w.skipPager = true;
    w.skipSwitcher = true;
    w.onAllDesktops = true;
}
workspace.windowAdded.connect(place);
workspace.windowList().forEach(place);
"#;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Cfg {
    screen: String,
}
impl Default for Cfg {
    fn default() -> Self {
        Cfg { screen: "tray".into() }
    }
}

fn cfg_path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("razerctl/osd.json")
}
static CFG: LazyLock<Mutex<Cfg>> =
    LazyLock::new(|| Mutex::new(fs::read_to_string(cfg_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()));
static APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(serde::Serialize)]
pub struct Screen {
    id: String,
    name: String,
    geometry: String,
}

/// Outputs as KDE numbers them (kscreen-doctor ids).
#[tauri::command]
pub fn osd_screens() -> Vec<Screen> {
    let out = Command::new("kscreen-doctor").arg("-o").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    // strip ANSI colour codes
    let mut clean = String::new();
    let mut esc = false;
    for c in out.chars() {
        if c == '\x1b' {
            esc = true;
        } else if esc {
            if c.is_ascii_alphabetic() {
                esc = false;
            }
        } else {
            clean.push(c);
        }
    }
    let mut v: Vec<Screen> = vec![];
    for line in clean.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Output: ") {
            let mut it = rest.split_whitespace();
            if let (Some(id), Some(name)) = (it.next(), it.next()) {
                v.push(Screen { id: id.into(), name: name.into(), geometry: String::new() });
            }
        } else if let Some(g) = t.strip_prefix("Geometry: ") {
            if let Some(last) = v.last_mut() {
                last.geometry = g.to_string();
            }
        }
    }
    v
}

#[tauri::command]
pub fn osd_get() -> String {
    CFG.lock().screen.clone()
}

#[tauri::command]
pub fn osd_set(screen: String) -> Result<(), String> {
    let c = Cfg { screen };
    if let Some(d) = cfg_path().parent() {
        let _ = fs::create_dir_all(d);
    }
    fs::write(cfg_path(), serde_json::to_string(&c).unwrap()).map_err(|e| e.to_string())?;
    *CFG.lock() = c;
    apply_window();
    Ok(())
}

static READY: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicBool = AtomicBool::new(false);

fn send_preview() {
    std::thread::spawn(|| {
        // give KWin a moment to move the window if the screen just changed
        std::thread::sleep(Duration::from_millis(250));
        if let Some(app) = APP.get() {
            let _ = app.emit_to("osd", "osd", serde_json::json!({ "preview": true, "sticky": true, "title": "Scroll-wheel pop-up", "value": "Will be here" }));
        }
    });
}

static PREVIEW_NOTE: Mutex<u32> = Mutex::new(0);

fn end_preview_visuals() {
    PENDING.store(false, Ordering::SeqCst);
    let id = std::mem::take(&mut *PREVIEW_NOTE.lock());
    if id != 0 {
        if let Some(c) = session() {
            let _ = c.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "CloseNotification",
                &(id,),
            );
        }
    }
    if let Some(app) = APP.get() {
        let _ = app.emit_to("osd", "osd", serde_json::json!({ "hide": true }));
    }
}

/// Settings preview: `Some(screen)` shows a "Will be here" sample where that choice would put the
/// pop-up and keeps it there; `None` removes it and goes back to the saved setting.
#[tauri::command]
pub fn osd_preview(screen: Option<String>) {
    end_preview_visuals();
    let Some(screen) = screen else {
        apply_window();
        return;
    };
    match screen.as_str() {
        "off" => apply_window(),
        "tray" => {
            apply_window();
            *PREVIEW_NOTE.lock() = notify_t(0, "Scroll-wheel pop-up", "Will be here", 50, 0);
        }
        _ => {
            apply_window_for(screen);
            if READY.load(Ordering::SeqCst) {
                send_preview();
            } else {
                // the pop-up window is still loading; it gets sent when the page reports ready
                PENDING.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn session() -> Option<&'static zbus::blocking::Connection> {
    static C: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    C.get_or_init(|| zbus::blocking::Connection::session().ok()).as_ref()
}

fn kwin(method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<zbus::message::Message> {
    let c = session().ok_or(zbus::Error::Failure("no session bus".into()))?;
    c.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), method, body)
}

/// Create/destroy the pop-up window and (re)load the KWin script that places it.
fn apply_window() {
    apply_window_for(CFG.lock().screen.clone());
}

fn apply_window_for(screen: String) {
    let Some(app) = APP.get().cloned() else { return };
    let own = screen != "tray" && screen != "off";
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let app = &app2;
        let existing = app.get_webview_window("osd");
        if !own {
            if let Some(w) = existing {
                let _ = w.destroy();
            }
            READY.store(false, Ordering::SeqCst);
            let _ = kwin("unloadScript", &(SCRIPT_NAME,));
            return;
        }
        // load the placement script first so it catches the window as it appears
        let p = dirs::cache_dir().unwrap_or_default().join("razerctl/osd.js");
        let _ = fs::create_dir_all(p.parent().unwrap());
        let safe: String = screen.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let _ = fs::write(&p, SCRIPT.replace("%TARGET%", &safe));
        let _ = kwin("unloadScript", &(SCRIPT_NAME,));
        let _ = kwin("loadScript", &(p.display().to_string(), SCRIPT_NAME));
        let _ = kwin("start", &());
        if existing.is_none() {
            // Mapped once and kept: fully transparent and click-through between pop-ups, so it
            // never has to be shown again (which could pull focus away from a game).
            let w = WebviewWindowBuilder::new(app, "osd", WebviewUrl::App("osd.html".into()))
                .title(WIN_TITLE)
                .inner_size(360.0, 110.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .focusable(false)
                .visible(true)
                .build();
            if let Ok(w) = w {
                let _ = w.set_ignore_cursor_events(true);
            }
        }
    });
}

fn num(dir: &str, attr: &str) -> Option<i64> {
    fs::read_to_string(Path::new(dir).join(attr)).ok().and_then(|s| s.trim().split_whitespace().next()?.parse().ok())
}

fn bar(pos: i64, max: i64) -> String {
    let n = 10;
    let filled = ((pos.clamp(0, max) * n + max / 2) / max) as usize;
    format!("{}{}", "▰".repeat(filled), "▱".repeat(n as usize - filled))
}

/// Bar that fills outward from the centre mark: off = -10 (all chat) ..= +10 (all game).
/// 5 cells a side (2 steps each, any step off centre lights one) so it fits on one line.
fn center_bar(off: i64) -> String {
    let off = off.clamp(-10, 10);
    let n = (off.abs() + 1) / 2;
    let left: String = (0..5).map(|i| if off < 0 && i >= 5 - n { '▰' } else { '▱' }).collect();
    let right: String = (0..5).map(|i| if off > 0 && i < n { '▰' } else { '▱' }).collect();
    format!("{left} ┃ {right}")
}

/// Show or update the single scroll-wheel notification. Returns its id for in-place updates.
fn notify(replaces: u32, summary: &str, body: &str, value: i64) -> u32 {
    notify_t(replaces, summary, body, value, 2000)
}

fn notify_t(replaces: u32, summary: &str, body: &str, value: i64, timeout: i32) -> u32 {
    let Some(c) = session() else { return 0 };
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("value", Value::from(value.clamp(0, 100) as i32));
    hints.insert("transient", Value::from(true)); // keep it out of the notification history
    hints.insert("urgency", Value::from(0u8));
    hints.insert("x-kde-display-appname", Value::from("Lynapse 2.0"));
    let r = c.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("Lynapse 2.0", replaces, "audio-headset", summary, body, Vec::<&str>::new(), hints, timeout),
    );
    if let Err(e) = &r {
        eprintln!("osd: notify failed: {e}");
    }
    r.ok().and_then(|m| m.body().deserialize::<u32>().ok()).unwrap_or(0)
}

enum Kind {
    GameChat(i64),
    Sidetone(i64),
}

fn side(off: i64) -> String {
    match off {
        0 => "Even".to_string(),
        n if n > 0 => format!("Game +{n}"),
        n => format!("Chat +{}", -n),
    }
}

struct Shower {
    id: u32,
    shown_at: Instant,
}

impl Shower {
    fn show(&mut self, k: Kind) {
        let screen = CFG.lock().screen.clone();
        if screen == "off" {
            return;
        }
        if screen != "tray" {
            let Some(app) = APP.get() else { return };
            let p = match k {
                Kind::GameChat(gc) => {
                    let off = gc - 10;
                    serde_json::json!({ "title": "Game / Chat", "value": side(off), "left": "Chat", "right": "Game", "center": true, "off": off as f64 / 10.0 })
                }
                Kind::Sidetone(st) => {
                    serde_json::json!({ "title": "Mic Sidetone", "value": st.to_string(), "left": "0", "right": "15", "center": false, "pos": st as f64 / 15.0 })
                }
            };
            let _ = app.emit_to("osd", "osd", p);
            return;
        }
        // KDE drops updates to a notification that has already timed out instead of showing a
        // new one, so only reuse the id while the previous pop-up is still on screen.
        if self.shown_at.elapsed() > Duration::from_millis(1700) {
            self.id = 0;
        }
        self.shown_at = Instant::now();
        self.id = match k {
            Kind::GameChat(gc) => notify(self.id, &format!("Game / Chat: {}", side(gc - 10)), &format!("Chat  {}  Game", center_bar(gc - 10)), gc * 5),
            Kind::Sidetone(st) => notify(self.id, &format!("Mic Sidetone: {st}"), &format!("0  {}  15", bar(st, 15)), st * 100 / 15),
        };
    }
}

/// `headset_dir` returns the connected headset's sysfs dir; `app_focused` is true while the
/// razerctl window has focus (changes made from the window don't need a pop-up).
pub fn start(app: AppHandle, headset_dir: Arc<dyn Fn() -> Option<String> + Send + Sync>, app_focused: Arc<dyn Fn() -> bool + Send + Sync>) {
    app.listen("osd-ready", |_| {
        READY.store(true, Ordering::SeqCst);
        if PENDING.swap(false, Ordering::SeqCst) {
            send_preview();
        }
    });
    let _ = APP.set(app);
    apply_window();
    if std::env::var("RAZERCTL_OSD_PREVIEW").is_ok() {
        osd_preview(Some(CFG.lock().screen.clone())); // dev aid: show the "Will be here" sample at startup
    }
    std::thread::spawn(move || {
        let mut last: Option<(String, i64, i64)> = None; // (dir, game/chat, sidetone)
        let mut sh = Shower { id: 0, shown_at: Instant::now() - Duration::from_secs(60) };
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let Some(dir) = headset_dir() else {
                last = None;
                continue;
            };
            let gc = num(&dir, "game_chat_balance").filter(|v| (0..=20).contains(v)).unwrap_or(-1);
            let st = num(&dir, "v3pro_sidetone").filter(|v| (0..=15).contains(v)).unwrap_or(-1);
            let prev = last.replace((dir.clone(), gc, st));
            let Some((pdir, pgc, pst)) = prev else { continue };
            if pdir != dir || app_focused() {
                continue;
            }
            if gc != pgc && gc >= 0 && pgc >= 0 {
                sh.show(Kind::GameChat(gc));
            } else if st != pst && st >= 0 && pst >= 0 {
                sh.show(Kind::Sidetone(st));
            }
        }
    });
}
