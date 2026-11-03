// Game Profiles (Synapse-style): when a listed game's window is in the foreground, switch the
// headset's THX / EQ to that game's settings; when you leave it, put back what you had.
//
// Wayland gives apps no way to see the active window, so a tiny KWin script reports every window
// activation to us over D-Bus (org.razerctl.App /gameprof).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::Duration,
};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Rule {
    /// matched against the window class, the program file name, or a Windows .exe in its command line
    pub key: String,
    pub name: String,
    /// None = leave as is
    pub thx: Option<bool>,
    pub eq: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Seen {
    pub key: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    rules: Vec<Rule>,
    /// windows seen recently, offered in the Add picker
    seen: Vec<Seen>,
}

#[derive(Clone, Default)]
struct Win {
    class: String,
    caption: String,
    exe: String,
    exes: Vec<String>,
}

struct Baseline {
    thx: Option<bool>,
    eq: Option<u32>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
static BASE: Mutex<Option<Baseline>> = Mutex::new(None);
static ACTIVE_RULE: Mutex<Option<String>> = Mutex::new(None);
static GEN: AtomicU64 = AtomicU64::new(0);
static HEADSET: OnceLock<Arc<dyn Fn() -> Option<(String, String)> + Send + Sync>> = OnceLock::new();
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("razerctl/gameprofiles.json")
}
fn store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| Mutex::new(fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()))
}
fn save(s: &Store) {
    let p = path();
    let _ = fs::create_dir_all(p.parent().unwrap());
    let _ = fs::write(p, serde_json::to_string_pretty(s).unwrap_or_default());
}

fn base(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

fn win_info(class: &str, caption: &str, pid: i32) -> Win {
    let mut w = Win { class: class.to_string(), caption: caption.trim().to_string(), ..Default::default() };
    if pid > 0 {
        w.exe = fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| base(&p.to_string_lossy())).unwrap_or_default();
        if let Ok(c) = fs::read(format!("/proc/{pid}/cmdline")) {
            w.exes = c
                .split(|b| *b == 0)
                .map(|a| String::from_utf8_lossy(a).to_string())
                .filter(|a| a.to_lowercase().ends_with(".exe"))
                .map(|a| base(&a))
                .collect();
        }
    }
    w
}

/// The most specific stable name for a window: Steam's per-game class, a Windows .exe, the program.
fn best_key(w: &Win) -> String {
    if w.class.starts_with("steam_app_") {
        return w.class.clone();
    }
    if let Some(e) = w.exes.iter().find(|e| !e.eq_ignore_ascii_case("steam.exe") && !e.to_lowercase().contains("proton")) {
        return e.clone();
    }
    if !w.class.is_empty() {
        return w.class.clone();
    }
    w.exe.clone()
}

fn matches(r: &Rule, w: &Win) -> bool {
    let k = r.key.to_lowercase();
    !k.is_empty() && (w.class.to_lowercase() == k || w.exe.to_lowercase() == k || w.exes.iter().any(|e| e.to_lowercase() == k))
}

/// Windows that shouldn't count as leaving the game (the panel/tray, our own windows, the KDE shell).
fn ignored(w: &Win) -> bool {
    let c = w.class.to_lowercase();
    c.is_empty() || c.contains("plasmashell") || c.contains("razerctl") || c == "org.kde.krunner" || w.caption == "razerctl OSD" || c.contains("xwaylandvideobridge")
}

// ---------- headset reads/writes ----------
fn rd(dir: &str, a: &str) -> Option<u32> {
    fs::read_to_string(Path::new(dir).join(a)).ok().and_then(|s| s.split_whitespace().next().and_then(|t| t.parse().ok()))
}
fn apply(thx: Option<bool>, eq: Option<u32>) {
    let Some((dir, needle)) = HEADSET.get().and_then(|f| f()) else { return };
    if let Some(e) = eq {
        if rd(&dir, "v3pro_headphone_eq") != Some(e) {
            let _ = fs::write(Path::new(&dir).join("v3pro_headphone_eq"), e.to_string());
        }
    }
    if let Some(on) = thx {
        // without the PipeWire THX filter there is nothing to route to, so leave it alone
        if crate::thx_available() && rd(&dir, "v3pro_thx_spatial_audio").map(|v| v == 1) != Some(on) {
            let _ = fs::write(Path::new(&dir).join("v3pro_thx_spatial_audio"), if on { "1" } else { "0" });
            let _ = crate::thx_route_sync(on, &needle);
        }
    }
}
fn current() -> Baseline {
    let Some((dir, _)) = HEADSET.get().and_then(|f| f()) else { return Baseline { thx: None, eq: None } };
    Baseline { thx: rd(&dir, "v3pro_thx_spatial_audio").map(|v| v == 1), eq: rd(&dir, "v3pro_headphone_eq").filter(|v| *v < 5) }
}

fn on_active(w: Win) {
    if ignored(&w) {
        return;
    }
    // remember it for the Add picker (most recent first)
    {
        let key = best_key(&w);
        if !key.is_empty() {
            let mut s = store().lock();
            s.seen.retain(|x| x.key != key);
            s.seen.insert(0, Seen { key, name: if w.caption.is_empty() { w.class.clone() } else { w.caption.clone() } });
            s.seen.truncate(25);
            save(&s);
        }
    }
    let rule = store().lock().rules.iter().find(|r| matches(r, &w)).cloned();
    // settle for a moment so a quick alt-tab doesn't flip settings back and forth
    let g = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        if GEN.load(Ordering::SeqCst) != g {
            return;
        }
        let mut active = ACTIVE_RULE.lock();
        match rule {
            Some(r) => {
                if active.as_deref() == Some(r.key.as_str()) {
                    return;
                }
                let mut b = BASE.lock();
                if b.is_none() {
                    *b = Some(current());
                }
                drop(b);
                apply(r.thx, r.eq);
                *active = Some(r.key.clone());
            }
            None => {
                if active.take().is_some() {
                    if let Some(b) = BASE.lock().take() {
                        apply(b.thx, b.eq);
                    }
                }
            }
        }
    });
}

struct Iface;

#[zbus::interface(name = "org.razerctl.GameProf")]
impl Iface {
    fn active(&self, class: String, caption: String, pid: i32) {
        on_active(win_info(&class, &caption, pid));
    }
}

const SCRIPT: &str = r#"
function report(w) {
    if (!w) return;
    callDBus("org.razerctl.App", "/gameprof", "org.razerctl.GameProf", "Active",
             String(w.resourceClass || ""), String(w.caption || ""), w.pid || 0);
}
workspace.windowActivated.connect(report);
report(workspace.activeWindow);
"#;
const SCRIPT_NAME: &str = "razerctl-gameprof";

fn kwin(method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) {
    if let Some(c) = CONN.get() {
        let _ = c.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), method, body);
    }
}

/// `headset` returns (sysfs dir, PipeWire name needle) of the connected headset.
pub fn start(headset: Arc<dyn Fn() -> Option<(String, String)> + Send + Sync>) {
    let _ = HEADSET.set(headset);
    std::thread::spawn(|| {
        let conn = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name("org.razerctl.App"))
            .and_then(|b| b.serve_at("/gameprof", Iface))
            .and_then(|b| b.build());
        let conn = match conn {
            Ok(c) => c,
            Err(e) => {
                eprintln!("gameprof: d-bus: {e}");
                return;
            }
        };
        let _ = CONN.set(conn);
        let p = dirs::cache_dir().unwrap_or_default().join("razerctl/gameprof.js");
        let _ = fs::create_dir_all(p.parent().unwrap());
        let _ = fs::write(&p, SCRIPT);
        kwin("unloadScript", &(SCRIPT_NAME,));
        kwin("loadScript", &(p.display().to_string(), SCRIPT_NAME));
        kwin("start", &());
    });
}

// ---------- commands for the Game Profiles card ----------
#[tauri::command]
pub fn gp_list() -> serde_json::Value {
    let s = store().lock();
    serde_json::json!({ "rules": s.rules, "seen": s.seen })
}

#[tauri::command]
pub fn gp_save(rules: Vec<Rule>) {
    let mut s = store().lock();
    s.rules = rules.into_iter().filter(|r| !r.key.trim().is_empty()).collect();
    save(&s);
    // a changed rule takes effect the next time the game's window comes to the front
}
