// Smart polling rate switcher: a KWin script reports the focused window (full screen or a
// borderless window covering its monitor) over D-Bus; while a game has focus the mouse runs
// at the chosen polling rate, and the previous rate is restored afterwards.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock, OnceLock},
};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Config {
    pub enabled: bool,
    pub rate: u32,
}

#[derive(Default)]
struct State {
    cfg: Config,
    in_game: bool,
    class: String,
    saved: Option<String>,
    watching: bool,
    error: String,
}

type MouseFn = Arc<dyn Fn() -> Option<String> + Send + Sync>;
static ST: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
static MOUSE: OnceLock<MouseFn> = OnceLock::new();
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();

const SCRIPT_NAME: &str = "razerctl-smartpoll";
// Not games even when full screen.
const IGNORE: &[&str] = &[
    "firefox", "chrom", "brave", "vivaldi", "opera", "mpv", "vlc", "haruna", "celluloid", "totem", "smplayer",
    "plasmashell", "krunner", "spectacle", "razerctl", "okular", "gwenview", "dolphin", "konsole",
];
const SCRIPT: &str = r#"(function () {
function report() {
    const w = workspace.activeWindow;
    let fs = false, cls = "";
    if (w && w.normalWindow) {
        cls = String(w.resourceClass);
        const g = w.frameGeometry, o = w.output ? w.output.geometry : null;
        fs = w.fullScreen || (w.noBorder && o && g.x <= o.x && g.y <= o.y && g.width >= o.width && g.height >= o.height);
    }
    callDBus("org.joe.razerctl", "/SmartPoll", "org.joe.razerctl.SmartPoll", "Window", fs ? "1" : "0", cls);
}
function hook(w) {
    if (!w) return;
    w.fullScreenChanged.connect(report);
    w.frameGeometryChanged.connect(() => { if (w === workspace.activeWindow) report(); });
}
workspace.windowList().forEach(hook);
workspace.windowAdded.connect(hook);
workspace.windowActivated.connect(report);
report();
})();
"#;

fn cfg_path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("razerctl/smartpoll.json")
}

struct Iface;

#[zbus::interface(name = "org.joe.razerctl.SmartPoll")]
impl Iface {
    fn window(&self, fullscreen: String, class: String) {
        on_window(fullscreen == "1", class);
    }
}

pub fn init(mouse: MouseFn) {
    let _ = MOUSE.set(mouse);
    let cfg: Config = fs::read_to_string(cfg_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let on = cfg.enabled;
    ST.lock().cfg = cfg;
    if on {
        std::thread::spawn(watch);
    }
}

pub fn set(cfg: Config) -> Result<(), String> {
    if let Some(d) = cfg_path().parent() {
        let _ = fs::create_dir_all(d);
    }
    fs::write(cfg_path(), serde_json::to_string(&cfg).unwrap()).map_err(|e| e.to_string())?;
    let (on, in_game) = {
        let mut st = ST.lock();
        st.cfg = cfg.clone();
        (cfg.enabled, st.in_game)
    };
    if on {
        if in_game {
            apply_game();
        }
        std::thread::spawn(watch);
    } else {
        restore();
        unwatch();
    }
    Ok(())
}

pub fn status() -> serde_json::Value {
    let st = ST.lock();
    serde_json::json!({ "enabled": st.cfg.enabled, "rate": st.cfg.rate, "in_game": st.in_game, "window": st.class,
        "watching": st.watching, "error": st.error, "restoring_to": st.saved })
}

fn kwin(conn: &zbus::blocking::Connection, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<zbus::message::Message> {
    conn.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), method, body)
}

fn watch() {
    let r = (|| -> Result<(), String> {
        if CONN.get().is_none() {
            let c = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.name("org.joe.razerctl"))
                .and_then(|b| b.serve_at("/SmartPoll", Iface))
                .and_then(|b| b.build())
                .map_err(|e| format!("D-Bus: {e}"))?;
            let _ = CONN.set(c);
        }
        let conn = CONN.get().unwrap();
        let p = dirs::cache_dir().unwrap_or_default().join("razerctl/smartpoll.js");
        let _ = fs::create_dir_all(p.parent().unwrap());
        fs::write(&p, SCRIPT).map_err(|e| e.to_string())?;
        let _ = kwin(conn, "unloadScript", &(SCRIPT_NAME,));
        kwin(conn, "loadScript", &(p.display().to_string(), SCRIPT_NAME)).map_err(|e| format!("KWin: {e}"))?;
        kwin(conn, "start", &()).map_err(|e| format!("KWin: {e}"))?;
        Ok(())
    })();
    let mut st = ST.lock();
    match r {
        Ok(()) => {
            st.watching = true;
            st.error.clear();
        }
        Err(e) => {
            eprintln!("smartpoll: {e}");
            st.watching = false;
            st.error = e;
        }
    }
}

fn unwatch() {
    if let Some(c) = CONN.get() {
        let _ = kwin(c, "unloadScript", &(SCRIPT_NAME,));
    }
    let mut st = ST.lock();
    st.watching = false;
    st.in_game = false;
}

fn on_window(fullscreen: bool, class: String) {
    let lc = class.to_lowercase();
    let game = fullscreen && !lc.is_empty() && !IGNORE.iter().any(|i| lc.contains(i));
    let (changed, on) = {
        let mut st = ST.lock();
        st.class = class;
        let changed = st.in_game != game;
        st.in_game = game;
        (changed, st.cfg.enabled)
    };
    if changed && on {
        if game {
            apply_game();
        } else {
            restore();
        }
    }
}

// Sysfs writes can stall for seconds (a mouse on a KVM), so they run off the D-Bus thread.
fn apply_game() {
    std::thread::spawn(|| {
        let Some(dir) = MOUSE.get().and_then(|f| f()) else { return };
        let p = PathBuf::from(dir).join("poll_rate");
        let rate = ST.lock().cfg.rate;
        if rate == 0 {
            return;
        }
        let cur = fs::read_to_string(&p).map(|s| s.trim().to_string()).unwrap_or_default();
        {
            let mut st = ST.lock();
            if st.saved.is_none() && !cur.is_empty() {
                st.saved = Some(cur.clone());
            }
        }
        if cur != rate.to_string() {
            let _ = fs::write(&p, rate.to_string());
        }
    });
}

fn restore() {
    let Some(prev) = ST.lock().saved.take() else { return };
    std::thread::spawn(move || {
        if let Some(dir) = MOUSE.get().and_then(|f| f()) {
            let _ = fs::write(PathBuf::from(dir).join("poll_rate"), prev);
        }
    });
}
