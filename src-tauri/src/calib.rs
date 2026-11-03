// Sensitivity matcher: counts raw motion (device counts, before any pointer acceleration)
// from every pointer device while the user moves one mouse a fixed physical distance.
// Comparing the counts of the other mouse and the Razer mouse over the same distance gives
// the other mouse's DPI relative to the Razer's current DPI.

use evdev::{Device, EventType, RelativeAxisCode};
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    time::Duration,
};

#[derive(Serialize, Clone, Default)]
pub struct Reading {
    pub path: String,
    pub name: String,
    pub distance: f64,
}

struct Session {
    stop: Arc<AtomicBool>,
    totals: Arc<Mutex<HashMap<String, Reading>>>,
}

static SESSION: LazyLock<Mutex<Option<Session>>> = LazyLock::new(|| Mutex::new(None));

pub fn stop() {
    if let Some(s) = SESSION.lock().take() {
        s.stop.store(true, Ordering::Relaxed);
    }
}

/// Start (or restart) measuring: totals reset to zero.
pub fn start() -> Result<usize, String> {
    stop();
    let mut devs = vec![];
    for (path, dev) in evdev::enumerate() {
        let name = dev.name().unwrap_or("").to_string();
        if name == "razerctl macro" || name == "razerctl virtual keyboard" {
            continue;
        }
        let ok = dev.supported_relative_axes().is_some_and(|r| r.contains(RelativeAxisCode::REL_X) && r.contains(RelativeAxisCode::REL_Y));
        if ok && dev.set_nonblocking(true).is_ok() {
            devs.push((path.display().to_string(), name, dev));
        }
    }
    if devs.is_empty() {
        return Err("No mice found (are you in the input group?)".into());
    }
    let n = devs.len();
    let stop = Arc::new(AtomicBool::new(false));
    let totals: Arc<Mutex<HashMap<String, Reading>>> = Arc::new(Mutex::new(
        devs.iter().map(|(p, nm, _)| (p.clone(), Reading { path: p.clone(), name: nm.clone(), distance: 0.0 })).collect(),
    ));
    let (st, tt) = (stop.clone(), totals.clone());
    std::thread::spawn(move || {
        let mut devs: Vec<(String, Device)> = devs.into_iter().map(|(p, _, d)| (p, d)).collect();
        let mut pending: HashMap<String, (i32, i32)> = HashMap::new();
        while !st.load(Ordering::Relaxed) {
            devs.retain_mut(|(p, d)| {
                match d.fetch_events() {
                    Ok(evs) => {
                        for ev in evs {
                            let e = pending.entry(p.clone()).or_default();
                            match ev.event_type() {
                                EventType::RELATIVE if ev.code() == RelativeAxisCode::REL_X.0 => e.0 += ev.value(),
                                EventType::RELATIVE if ev.code() == RelativeAxisCode::REL_Y.0 => e.1 += ev.value(),
                                EventType::SYNCHRONIZATION => {
                                    let (x, y) = std::mem::take(e);
                                    if x != 0 || y != 0 {
                                        if let Some(r) = tt.lock().get_mut(p.as_str()) {
                                            r.distance += ((x as f64).powi(2) + (y as f64).powi(2)).sqrt();
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        true
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                    Err(_) => false, // unplugged
                }
            });
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    *SESSION.lock() = Some(Session { stop, totals });
    Ok(n)
}

/// Current totals, largest first.
pub fn read() -> Vec<Reading> {
    let mut v: Vec<Reading> = SESSION.lock().as_ref().map(|s| s.totals.lock().values().cloned().collect()).unwrap_or_default();
    v.sort_by(|a, b| b.distance.total_cmp(&a.distance));
    v
}
