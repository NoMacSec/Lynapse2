// Headset quick settings in the tray menus (ported from Lynapse's tray): submenus that open to the
// right with the current value marked ●. Item ids are "q:<what>:<value>".

use std::{fs, path::Path};
use tauri::{
    menu::{IsMenuItem, MenuItem, Submenu},
    AppHandle, Manager, Runtime,
};

const EQ: [&str; 5] = ["Default", "Game", "Movie", "Music", "Esports"];
const MIC: [&str; 4] = ["Default", "Flat", "Broadcast", "Esports"]; // slot 1 = driver "MicBoost", labelled Flat in Synapse
const FN: [&str; 4] = ["Game/Chat", "Mic Sidetone", "Footsteps", "Bluetooth Volume"];
const SLEEP: [(u32, &str); 5] = [(0, "Never"), (15, "15 min"), (30, "30 min"), (45, "45 min"), (60, "60 min")];

fn exists(dir: &str, attr: &str) -> bool {
    Path::new(dir).join(attr).exists()
}
fn attr_name<'a>(dir: &str, attr: &'a str) -> &'a str {
    if exists(dir, attr) { return attr }
    match attr {
        "v3pro_sidetone" if exists(dir, "sidetone") => "sidetone",
        "v3pro_thx_spatial_audio" if exists(dir, "thx_spatial_audio") => "thx_spatial_audio",
        "v3pro_ultra_low_latency" if exists(dir, "ultra_low_latency") => "ultra_low_latency",
        "v3pro_power_save" if exists(dir, "wireless_power_save") => "wireless_power_save",
        _ => attr,
    }
}
fn has(dir: &str, attr: &str) -> bool {
    exists(dir, attr_name(dir, attr))
}
fn rd(dir: &str, attr: &str) -> Option<String> {
    fs::read_to_string(Path::new(dir).join(attr_name(dir, attr))).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "-1")
}
fn num(dir: &str, attr: &str) -> Option<i64> {
    rd(dir, attr).and_then(|s| s.split_whitespace().next().and_then(|t| t.parse().ok()))
}
fn sel(label: &str, on: bool) -> String {
    if on { format!("● {label}") } else { format!("   {label}") }
}
fn gc_label(v: i64) -> String {
    match v - 10 {
        0 => "Even".into(),
        n if n > 0 => format!("Game +{n}"),
        n => format!("Chat +{}", -n),
    }
}

/// Padding after a submenu label so KDE's ▸ arrow isn't jammed against the text (em spaces,
/// which Qt doesn't trim).
/// The quick settings sit straight under the headset's line in the main menu, indented under it.
const INDENT: &str = "\u{2003}";
pub const SUB_GAP: &str = "\u{2003}\u{2003}";

type Items<R> = Vec<Box<dyn IsMenuItem<R>>>;

fn item<R: Runtime>(app: &AppHandle<R>, id: String, label: String) -> Option<Box<dyn IsMenuItem<R>>> {
    MenuItem::with_id(app, id, format!("{INDENT}{label}"), true, None::<&str>).ok().map(|m| Box::new(m) as _)
}
fn sub<R: Runtime>(app: &AppHandle<R>, label: String, kids: Vec<(String, String)>) -> Option<Box<dyn IsMenuItem<R>>> {
    let built: Vec<MenuItem<R>> = kids.into_iter().filter_map(|(id, l)| MenuItem::with_id(app, id, l, true, None::<&str>).ok()).collect();
    let refs: Vec<&dyn IsMenuItem<R>> = built.iter().map(|m| m as &dyn IsMenuItem<R>).collect();
    Submenu::with_items(app, format!("{INDENT}{label}{SUB_GAP}"), true, &refs).ok().map(|s| Box::new(s) as _)
}

/// Quick-settings entries for a connected headset (sysfs dir `dir`).
pub fn headset_items<R: Runtime>(app: &AppHandle<R>, dir: &str) -> Items<R> {
    let mut v: Items<R> = vec![];
    let mut push = |x: Option<Box<dyn IsMenuItem<R>>>| {
        if let Some(x) = x {
            v.push(x);
        }
    };
    if has(dir, "v3pro_headphone_eq") {
        let cur = num(dir, "v3pro_headphone_eq").unwrap_or(-1);
        let name = EQ.get(cur as usize).copied().unwrap_or(if cur >= 5 { "Custom" } else { "?" });
        push(sub(app, format!("Sound EQ: {name}"), EQ.iter().enumerate().map(|(i, n)| (format!("q:eq:{i}"), sel(n, i as i64 == cur))).collect()));
    }
    if has(dir, "mic_eq_preset") {
        let cur = num(dir, "mic_eq_preset").unwrap_or(-1);
        let name = MIC.get(cur as usize).copied().unwrap_or("?");
        push(sub(app, format!("Mic EQ: {name}"), MIC.iter().enumerate().map(|(i, n)| (format!("q:mic:{i}"), sel(n, i as i64 == cur))).collect()));
    }
    if has(dir, "v3pro_sidetone") {
        let cur = num(dir, "v3pro_sidetone").unwrap_or(-1);
        push(sub(app, format!("Sidetone: {}", if cur >= 0 { cur.to_string() } else { "?".into() }), (0..16).map(|i| (format!("q:st:{i}"), sel(&i.to_string(), i == cur))).collect()));
    }
    if has(dir, "v3pro_anc") {
        let s = rd(dir, "v3pro_anc").unwrap_or_default();
        let mut it = s.split_whitespace().filter_map(|t| t.parse::<i64>().ok());
        let (mode, lvl) = (it.next().unwrap_or(-1), it.next().unwrap_or(4));
        let cur = match mode {
            0 => "Off".to_string(),
            1 => format!("ANC {lvl}"),
            2 => "Ambient".into(),
            _ => "?".into(),
        };
        let mut kids = vec![("q:anc:0".to_string(), sel("Off", mode == 0))];
        for (n, l) in [(1, "Low"), (2, "Med"), (3, "High"), (4, "Max")] {
            kids.push((format!("q:anc:1:{n}"), sel(&format!("ANC: {l}"), mode == 1 && lvl == n)));
        }
        kids.push(("q:anc:2".to_string(), sel("Ambient", mode == 2)));
        push(sub(app, format!("ANC: {cur}"), kids));
    }
    if has(dir, "v3pro_ultra_low_latency") {
        let on = num(dir, "v3pro_ultra_low_latency") == Some(1);
        push(item(app, "q:ull:toggle".into(), format!("Turn {} HyperSpeed", if on { "Off" } else { "On" })));
    }
    if has(dir, "v3pro_thx_spatial_audio") {
        let on = num(dir, "v3pro_thx_spatial_audio") == Some(1);
        if crate::thx_available() {
            push(item(app, "q:thx:toggle".into(), format!("Turn {} Spatial Audio", if on { "Off" } else { "On" })));
        } else {
            // no PipeWire THX filter installed: the switch would do nothing, so show it greyed out
            push(MenuItem::with_id(app, "q:thx:na", format!("{INDENT}Spatial Audio (filter not installed)"), false, None::<&str>).ok().map(|m| Box::new(m) as _));
        }
    }
    let fnm = num(dir, "audio_function_button").unwrap_or(0);
    if has(dir, "audio_function_button") {
        push(sub(app, format!("Fn: {}", FN.get(fnm as usize).copied().unwrap_or("?")), FN.iter().enumerate().map(|(i, n)| (format!("q:fn:{i}"), sel(n, i as i64 == fnm))).collect()));
    }
    // The scroll wheel follows the Fn mode: sidetone in mode 1, game/chat balance otherwise.
    if fnm == 1 && has(dir, "v3pro_sidetone") {
        let cur = num(dir, "v3pro_sidetone").unwrap_or(-1);
        push(sub(app, format!("Sidetone Scroll: {cur}"), (0..16).map(|i| (format!("q:st:{i}"), sel(&i.to_string(), i == cur))).collect()));
    } else if has(dir, "game_chat_balance") {
        let cur = num(dir, "game_chat_balance").filter(|v| (0..=20).contains(v)).unwrap_or(-1);
        push(sub(app, format!("Game-Chat Scroll: {}", if cur >= 0 { gc_label(cur) } else { "?".into() }), (0..=20).step_by(2).map(|i| (format!("q:gc:{i}"), sel(&gc_label(i), i == cur))).collect()));
    }
    if exists(dir, "v3pro_power_save") {
        let cur = num(dir, "v3pro_power_save").unwrap_or(-1);
        let name = SLEEP.iter().find(|(m, _)| *m as i64 == cur).map(|x| x.1).unwrap_or("?");
        push(sub(app, format!("Sleep: {name}"), SLEEP.iter().map(|(m, l)| (format!("q:pwr:{m}"), sel(l, *m as i64 == cur))).collect()));
    } else if exists(dir, "wireless_power_save") {
        let cur = num(dir, "wireless_power_save").unwrap_or(-1);
        let choices = [(15, "15 min"), (30, "30 min"), (45, "45 min"), (60, "60 min")];
        push(sub(app, format!("Sleep: {}", choices.iter().find(|(m, _)| *m == cur).map(|x| x.1).unwrap_or("?")), choices.iter().map(|(m, l)| (format!("q:pwr:{m}"), sel(l, *m == cur))).collect()));
    }
    v
}

/// Handle a "q:..." menu id for the headset whose sysfs dir is `dir`. Returns true if handled.
pub fn handle<R: Runtime>(app: &AppHandle<R>, dir: &str, id: &str) -> bool {
    let Some(rest) = id.strip_prefix("q:") else { return false };
    let parts: Vec<&str> = rest.split(':').collect();
    let w = |attr: &str, val: String| {
        let _ = fs::write(Path::new(dir).join(attr_name(dir, attr)), val);
    };
    match parts.as_slice() {
        ["eq", n] => w("v3pro_headphone_eq", n.to_string()),
        ["mic", n] => w("mic_eq_preset", n.to_string()),
        ["st", n] => w("v3pro_sidetone", n.to_string()),
        ["gc", n] => w("game_chat_balance", n.to_string()),
        ["fn", n] => w("audio_function_button", n.to_string()),
        ["pwr", n] => w("v3pro_power_save", n.to_string()),
        ["anc", "0"] => {
            let lvl = rd(dir, "v3pro_anc").and_then(|s| s.split_whitespace().nth(1).map(String::from)).unwrap_or("4".into());
            w("v3pro_anc", format!("0 {lvl}"))
        }
        ["anc", "1", n] => w("v3pro_anc", format!("1 {n}")),
        ["anc", "2"] => {
            let lvl = rd(dir, "v3pro_anc").and_then(|s| s.split_whitespace().nth(1).map(String::from)).unwrap_or("4".into());
            w("v3pro_anc", format!("2 {lvl}"))
        }
        ["ull", "toggle"] => {
            let on = num(dir, "v3pro_ultra_low_latency") == Some(1);
            w("v3pro_ultra_low_latency", if on { "0" } else { "1" }.into())
        }
        ["thx", "toggle"] => {
            let on = num(dir, "v3pro_thx_spatial_audio") != Some(1);
            w("v3pro_thx_spatial_audio", if on { "1" } else { "0" }.into());
            // move the game streams onto/off the THX filter, like the app's THX switch
            let needle = app
                .try_state::<std::sync::Arc<crate::AppState>>()
                .and_then(|st| st.devices.lock().iter().find(|d| d.active.as_deref() == Some(dir)).map(|d| d.name.clone()))
                .unwrap_or_default()
                .trim_start_matches("Razer ")
                .replace(' ', "_");
            std::thread::spawn(move || {
                let _ = crate::thx_route_sync(on, &needle);
            });
        }
        _ => return true,
    }
    true
}
