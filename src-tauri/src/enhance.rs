// Synapse "Audio Enhancement" (playback) and "Mic Enhancement" rebuilt as PipeWire filter-chains.
//
// Synapse runs these on the PC with THX v4 software (not in the headset). On Linux each side is ONE fixed filter-chain
// (62-razerctl-enhance.conf) whose stages are all neutral when off; picking a mode/level only changes live control
// values (pw-cli set-param ... Props), so nothing restarts while you click or drag. The numbers are fitted to
// measurements of THX on Joe's PC (2026-11-03); the reference model + fit notes are in dsp/enh.py, the dynamics plugin
// (expander / gate / compressor) is dsp/razer_dyn.c, compiled by build.rs and shipped inside this binary.
//
// Routing: turning a side on makes the "(Enhanced)" node the default and moves the headset's streams onto it;
// turning it off moves them back. Like Synapse, only one mode per side is active at a time.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, time::Duration};

static DYN_SO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/razer_dyn.so"));

pub const GAME_SINK: &str = "razerctl_game_enhanced";
const GAME_OUT: &str = "effect_output.razerctl-enhance";
pub const MIC_SRC: &str = "razerctl_mic_enhanced";
const MIC_IN: &str = "effect_input.razerctl-mic";

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Side {
    #[serde(default)]
    pub on: bool,
    #[serde(default)]
    pub mode: String,
    /// last level per mode (Synapse keeps one slider value per tab)
    #[serde(default)]
    pub levels: BTreeMap<String, f64>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct State {
    #[serde(default)]
    pub audio: Side,
    #[serde(default)]
    pub mic: Side,
}

fn state_path() -> PathBuf { dirs::config_dir().unwrap_or_default().join("razerctl/enhance.json") }
fn conf_path() -> PathBuf { dirs::config_dir().unwrap_or_default().join("pipewire/pipewire.conf.d/62-razerctl-enhance.conf") }
fn so_path() -> PathBuf { dirs::data_local_dir().unwrap_or_default().join("razerctl/razer_dyn.so") }

pub fn load() -> State {
    fs::read_to_string(state_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
fn save(st: &State) {
    let p = state_path();
    let _ = fs::create_dir_all(p.parent().unwrap());
    let _ = fs::write(p, serde_json::to_string_pretty(st).unwrap_or_default());
}

/// Synapse defaults per mode (getDefaultProfile): 55, mic noise 60, voice gate step 2 (Medium).
pub fn default_level(mode: &str) -> f64 {
    match mode { "noise" => 60.0, "gate" => 2.0, _ => 55.0 }
}

// ---------------------------------------------------------------- DSP model (port of dsp/enh.py)
fn interp(l: f64, pts: &[(f64, f64)]) -> f64 {
    for i in 0..pts.len() - 1 {
        let ((x0, y0), (x1, y1)) = (pts[i], pts[i + 1]);
        if l <= x1 || i == pts.len() - 2 {
            return y0 + (y1 - y0) * (l - x0) / (x1 - x0);
        }
    }
    pts[0].1
}
fn i3(l: f64, a: f64, b: f64, c: f64) -> f64 { interp(l, &[(0.25, a), (0.55, b), (1.0, c)]) }
fn n3(l: f64, a: f64, b: f64, c: f64) -> f64 { interp(l, &[(0.25, a), (0.60, b), (1.0, c)]) }

#[derive(Clone, Copy, PartialEq)]
enum Kind { Dyn, LowShelf, HighShelf, Peaking }

fn graph_nodes(side: &str) -> &'static [(&'static str, Kind)] {
    use Kind::*;
    if side == "mic" {
        &[("nc", Dyn), ("gate", Dyn), ("vc_low", LowShelf), ("vc_mid", Peaking), ("vc_air", HighShelf), ("norm", Dyn), ("lim", Dyn)]
    } else {
        &[("bass", LowShelf), ("vc_shelf", LowShelf), ("vc_dip", Peaking), ("vc_pres", Peaking), ("norm", Dyn), ("lim", Dyn)]
    }
}

const DYN_PORTS: [(&str, f64); 12] = [
    ("Input gain (dB)", 0.0), ("Expander threshold (dB)", -120.0), ("Expander ratio", 1.0), ("Expander floor (dB)", -200.0),
    ("Mute threshold (dB)", -200.0), ("Compressor threshold (dB)", 0.0), ("Compressor ratio", 1.0), ("Knee (dB)", 0.0),
    ("Attack (ms)", 10.0), ("Release (ms)", 100.0), ("Peak detect", 0.0), ("Detector window (ms)", 10.0),
];

fn bq_default(name: &str) -> f64 {
    match name { "bass" => 105.0, "vc_shelf" => 169.0, "vc_dip" => 151.0, "vc_pres" => 2535.0, "vc_low" => 334.0, "vc_mid" => 819.0, _ => 8000.0 }
}

type Params = Vec<(String, f64)>;

struct P(Params);
impl P {
    fn set(&mut self, key: String, v: f64) {
        if let Some(e) = self.0.iter_mut().find(|(k, _)| *k == key) { e.1 = v } else { self.0.push((key, v)) }
    }
    fn bq(&mut self, n: &str, f: f64, q: f64, g: f64) {
        self.set(format!("{n}:Freq"), f);
        self.set(format!("{n}:Q"), q);
        self.set(format!("{n}:Gain"), g);
    }
    fn dyn_(&mut self, n: &str, kv: &[(&str, f64)]) {
        for (k, v) in kv {
            let port = match *k {
                "gain" => "Input gain (dB)", "exp_t" => "Expander threshold (dB)", "exp_r" => "Expander ratio",
                "floor" => "Expander floor (dB)", "mute" => "Mute threshold (dB)", "comp_t" => "Compressor threshold (dB)",
                "comp_r" => "Compressor ratio", "knee" => "Knee (dB)", "attack" => "Attack (ms)", "release" => "Release (ms)",
                "peak" => "Peak detect", "window" => "Detector window (ms)", other => other,
            };
            self.set(format!("{n}:{port}"), *v);
        }
    }
}

fn neutral(side: &str) -> P {
    let mut p = P(vec![]);
    for (name, kind) in graph_nodes(side) {
        if *kind == Kind::Dyn {
            for (k, v) in DYN_PORTS { p.set(format!("{name}:{k}"), v) }
        } else {
            p.bq(name, bq_default(name), 0.707, 0.0);
        }
    }
    p
}

/// Full control set for one Synapse setting. mode None = off (every stage neutral).
/// Attack/Release are one-pole time constants in dB = THX's measured 90 % settle time / 2.3.
pub fn params(side: &str, mode: Option<&str>, level: f64) -> Params {
    let mut p = neutral(side);
    let l = level / 100.0;
    let Some(mode) = mode else { return p.0 };
    if side != "mic" {
        match mode {
            // one low shelf, 105 Hz, Q 0.75, +9 dB x slider (fit error < 0.01 dB)
            "bass" => p.bq("bass", 105.0, 0.75, 9.0 * l),
            // -6 dB low shelf (corner rises with slider) + low-mid dip + 2.5 kHz presence peak (<= 0.11 dB)
            "clarity" => {
                p.bq("vc_shelf", i3(l, 123.6, 169.1, 227.4), i3(l, 0.995, 0.964, 0.914), -6.0);
                p.bq("vc_dip", i3(l, 107.4, 151.1, 217.7), i3(l, 1.187, 1.235, 1.299), i3(l, -3.28, -3.78, -5.01));
                p.bq("vc_pres", 2535.0, i3(l, 0.954, 1.002, 1.009), i3(l, 1.538, 3.229, 6.142));
            }
            // fixed makeup gain, then a soft-knee limiter near 0 dBFS peak (static curve within 0.03/0.01/0.23 dB)
            "normalization" => p.dyn_("norm", &[
                ("gain", i3(l, 6.5, 9.5, 16.0)), ("comp_t", i3(l, -2.75, -2.75, -5.25)), ("comp_r", i3(l, 100.0, 50.0, 6.0)),
                ("knee", i3(l, 6.0, 8.0, 8.0)), ("attack", 1.0), ("release", i3(l, 200.0, 313.0, 500.0)), ("window", 10.0),
            ]),
            _ => {}
        }
        p.dyn_("lim", &[("comp_t", -0.1), ("comp_r", 100.0), ("attack", 0.5), ("release", 80.0), ("peak", 1.0), ("window", 20.0)]);
    } else {
        match mode {
            // broadband downward expander (+ hard mute from ~45 up); refitted on drift-corrected THX data
            "noise" => p.dyn_("nc", &[
                ("exp_t", n3(l, -47.5, -36.5, -19.5)), ("exp_r", n3(l, 2.5, 2.8, 3.2)),
                ("floor", if l < 0.45 { -60.0 } else { -200.0 }),
                ("mute", if l >= 0.45 { n3(l, -92.0, -77.7, -61.6) } else { -200.0 }),
                ("attack", 4.0), ("release", n3(l, 83.0, 117.0, 117.0)), ("window", 10.0),
            ]),
            // Synapse Voice Gate Low / Medium / High = level 1 / 2 / 3
            "gate" => match level.round() as i64 {
                i64::MIN..=1 => p.dyn_("gate", &[("exp_t", -57.0), ("exp_r", 2.0), ("floor", -32.0), ("attack", 4.0), ("release", 52.0)]),
                2 => p.dyn_("gate", &[("mute", -55.9), ("exp_t", -34.0), ("exp_r", 2.1), ("attack", 4.0), ("release", 43.0)]),
                _ => p.dyn_("gate", &[("mute", -33.5), ("exp_t", -6.5), ("exp_r", 1.9), ("attack", 4.0), ("release", 43.0)]),
            },
            // pure EQ: big low cut, broad ~1 kHz boost, a little air (rms error 0.6/1.6/2.3 dB vs acoustic THX data)
            "clarity" => {
                p.bq("vc_low", i3(l, 308.0, 337.0, 184.0), i3(l, 0.85, 1.55, 1.16), i3(l, -7.35, -11.7, -17.0));
                p.bq("vc_mid", i3(l, 532.0, 858.0, 1340.0), i3(l, 0.122, 0.253, 0.569), i3(l, 4.82, 8.7, 13.6));
                p.bq("vc_air", 8000.0, 0.707, i3(l, 2.37, 1.38, 1.82));
            }
            // fixed makeup gain + compression near the top (static curve; THX's slow loudness AGC is not modelled)
            "normalization" => p.dyn_("norm", &[
                ("gain", i3(l, 7.2, 11.9, 18.9)), ("comp_t", i3(l, -13.0, -13.0, -9.5)), ("comp_r", i3(l, 3.0, 8.0, 20.0)),
                ("knee", i3(l, 0.0, 0.0, 11.0)), ("attack", 5.0), ("release", 300.0), ("window", 10.0),
            ]),
            _ => {}
        }
        p.dyn_("lim", &[("comp_t", -1.0), ("comp_r", 100.0), ("attack", 0.5), ("release", 80.0), ("peak", 1.0), ("window", 20.0)]);
    }
    p.0
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.into() }
}

fn graph(side: &str, p: &Params, so: &str) -> String {
    let nodes = graph_nodes(side);
    let port = |n: &str, input: bool| {
        let dyn_ = nodes.iter().any(|(m, k)| *m == n && *k == Kind::Dyn);
        match (dyn_, input) { (true, true) => format!("{n}:Input"), (true, false) => format!("{n}:Output"), (false, true) => format!("{n}:In"), (false, false) => format!("{n}:Out") }
    };
    let mut out = String::from("            nodes = [\n");
    for (name, kind) in nodes {
        let ctl: Vec<String> = p.iter().filter(|(k, _)| k.split(':').next() == Some(name))
            .map(|(k, v)| format!("\"{}\" = {}", k.split_once(':').unwrap().1, fmt(*v))).collect();
        let head = match kind {
            Kind::Dyn => format!("type = ladspa name = {name} plugin = \"{so}\" label = razer_dyn"),
            Kind::LowShelf => format!("type = builtin name = {name} label = bq_lowshelf"),
            Kind::HighShelf => format!("type = builtin name = {name} label = bq_highshelf"),
            Kind::Peaking => format!("type = builtin name = {name} label = bq_peaking"),
        };
        out += &format!("                {{ {head} control = {{ {} }} }}\n", ctl.join(" "));
    }
    out += "            ]\n            links = [\n";
    for w in nodes.windows(2) {
        out += &format!("                {{ output = \"{}\" input = \"{}\" }}\n", port(w[0].0, false), port(w[1].0, true));
    }
    out += &format!("            ]\n            inputs = [ \"{}\" ]\n            outputs = [ \"{}\" ]\n", port(nodes[0].0, true), port(nodes[nodes.len() - 1].0, false));
    out
}

fn side_params(s: &Side, side: &str) -> Params {
    let mode = (s.on && !s.mode.is_empty()).then_some(s.mode.as_str());
    let lvl = s.levels.get(&s.mode).copied().unwrap_or_else(|| default_level(&s.mode));
    params(side, mode, lvl)
}

/// The whole drop-in. The "# targets:" line decides whether PipeWire must be restarted (device names changed);
/// control values alone are applied live, and are written here only so a PipeWire restart starts from them.
fn conf(game: &str, mic: &str, st: &State, so: &str) -> String {
    format!(
        "# Generated by razerctl (Sound > Enhancement, Mic > Mic Enhancement). Edited by the app; changes here are overwritten.
# targets: {game} {mic}
context.modules = [
{{   name = libpipewire-module-filter-chain
    flags = [ nofail ]
    args = {{
        node.description = \"BlackShark V3 Pro Game (Enhanced)\"
        media.name       = \"BlackShark V3 Pro Game (Enhanced)\"
        filter.graph = {{
{ag}        }}
        audio.rate = 48000
        audio.channels = 2
        audio.position = [ FL FR ]
        capture.props = {{
            node.name = \"{GAME_SINK}\"
            media.class = Audio/Sink
        }}
        playback.props = {{
            node.name = \"{GAME_OUT}\"
            node.passive = true
            target.object = \"{game}\"
            stream.dont-remix = true
        }}
    }}
}}
{{   name = libpipewire-module-filter-chain
    flags = [ nofail ]
    args = {{
        node.description = \"BlackShark V3 Pro Mic (Enhanced)\"
        media.name       = \"BlackShark V3 Pro Mic (Enhanced)\"
        filter.graph = {{
{mg}        }}
        audio.rate = 48000
        audio.channels = 1
        audio.position = [ MONO ]
        capture.props = {{
            node.name = \"{MIC_IN}\"
            node.passive = true
            target.object = \"{mic}\"
            stream.dont-remix = true
        }}
        playback.props = {{
            node.name = \"{MIC_SRC}\"
            media.class = Audio/Source
        }}
    }}
}}
]
",
        ag = graph("audio", &side_params(&st.audio, "audio"), so),
        mg = graph("mic", &side_params(&st.mic, "mic"), so),
    )
}

// ---------------------------------------------------------------- PipeWire plumbing
fn pw_nodes() -> Vec<(u64, String)> {
    let out = Command::new("pw-dump").output().map(|o| o.stdout).unwrap_or_default();
    let v: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
    v.as_array().into_iter().flatten()
        .filter(|o| o["type"] == "PipeWire:Interface:Node")
        .filter_map(|o| Some((o["id"].as_u64()?, o["info"]["props"]["node.name"].as_str()?.to_string())))
        .collect()
}
fn node_id(name: &str) -> Option<u64> { pw_nodes().into_iter().find(|(_, n)| n == name).map(|(i, _)| i) }

/// Push a control set to a running chain (both nodes of the filter; only the one carrying the controls takes it).
fn push(names: &[&str], p: &Params) -> Result<(), String> {
    let body: Vec<String> = p.iter().map(|(k, v)| format!("\"{k}\" {}", fmt(*v))).collect();
    let pod = format!("{{ params = [ {} ] }}", body.join(" "));
    let mut any = false;
    for n in names {
        if let Some(id) = node_id(n) {
            any = true;
            let _ = Command::new("pw-cli").args(["set-param", &id.to_string(), "Props", &pod]).output();
        }
    }
    if any { Ok(()) } else { Err("enhancement filter is not loaded in PipeWire".into()) }
}

fn pactl_json(args: &[&str]) -> Value {
    let out = Command::new("pactl").arg("-f").arg("json").args(args).output().map(|o| o.stdout).unwrap_or_default();
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}
fn index_of(kind: &str, name: &str) -> Option<u64> {
    pactl_json(&["list", "short", kind]).as_array()?.iter().find(|s| s["name"] == name).and_then(|s| s["index"].as_u64())
}
fn default_of(what: &str) -> String {
    let out = Command::new("pactl").arg(what).output().map(|o| o.stdout).unwrap_or_default();
    String::from_utf8_lossy(&out).trim().to_string()
}

/// Move every stream on `from` to `to` (sink-inputs or source-outputs), except filter streams named in `skip`.
fn move_streams(sink_side: bool, from: &str, to: &str, skip: &[&str]) {
    let (kind, list, field, cmd) = if sink_side { ("sinks", "sink-inputs", "sink", "move-sink-input") } else { ("sources", "source-outputs", "source", "move-source-output") };
    let Some(from_idx) = index_of(kind, from) else { return };
    for s in pactl_json(&["list", list]).as_array().into_iter().flatten() {
        if s[field].as_u64() != Some(from_idx) { continue }
        let name = s["properties"]["node.name"].as_str().unwrap_or("");
        if skip.iter().any(|k| name == *k) { continue }
        if let Some(i) = s["index"].as_u64() {
            let _ = Command::new("pactl").args([cmd, &i.to_string(), to]).status();
        }
    }
}

/// Write the plugin + drop-in. Restarts PipeWire only when the drop-in did not exist or its device targets changed.
fn ensure_loaded(game: &str, mic: &str, st: &State) -> Result<bool, String> {
    let so = so_path();
    if fs::read(&so).map(|b| b != DYN_SO).unwrap_or(true) {
        fs::create_dir_all(so.parent().unwrap()).map_err(|e| e.to_string())?;
        let tmp = so.with_extension("so.new");
        fs::write(&tmp, DYN_SO).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &so).map_err(|e| e.to_string())?;
    }
    let text = conf(game, mic, st, &so.to_string_lossy());
    let cp = conf_path();
    let old = fs::read_to_string(&cp).unwrap_or_default();
    let tline = |s: &str| s.lines().find(|l| l.starts_with("# targets:")).map(String::from);
    let structural = tline(&old) != tline(&text) || old.lines().filter(|l| l.contains("type =")).count() != text.lines().filter(|l| l.contains("type =")).count();
    if old != text {
        fs::create_dir_all(cp.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&cp, &text).map_err(|e| e.to_string())?;
    }
    let loaded = node_id(GAME_SINK).is_some() && node_id(MIC_SRC).is_some();
    if structural || !loaded {
        let _ = Command::new("systemctl").args(["--user", "restart", "pipewire", "pipewire-pulse", "wireplumber"]).status();
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(200));
            if node_id(GAME_SINK).is_some() && node_id(MIC_SRC).is_some() { break }
        }
        if node_id(GAME_SINK).is_none() || node_id(MIC_SRC).is_none() {
            return Err("the enhancement filter did not load (journalctl --user -u pipewire -b)".into());
        }
        return Ok(true);
    }
    Ok(false)
}

/// Raw headset node names: (game sink, mic source).
fn targets(needle: &str) -> Result<(String, String), String> {
    let game = crate::pactl_find("game", needle).ok_or("headset game output not found")?;
    let mic = crate::pactl_find("mic", needle).ok_or("headset mic not found")?;
    Ok((game, mic))
}

/// Make the running chains + routing match `st`. `which` = Some("audio"/"mic") routes only that side.
pub fn apply(st: &State, needle: &str, which: Option<&str>) -> Result<(), String> {
    let (game, mic) = targets(needle)?;
    let any_on = st.audio.on || st.mic.on;
    if !any_on && node_id(GAME_SINK).is_none() && node_id(MIC_SRC).is_none() {
        return Ok(()); // never used: don't install anything
    }
    ensure_loaded(&game, &mic, st)?;
    push(&[GAME_SINK, GAME_OUT], &side_params(&st.audio, "audio"))?;
    push(&[MIC_IN, MIC_SRC], &side_params(&st.mic, "mic"))?;
    if which.is_none_or(|w| w == "audio") { route_audio(st.audio.on, &game) }
    if which.is_none_or(|w| w == "mic") { route_mic(st.mic.on, &mic) }
    Ok(())
}

fn route_audio(on: bool, game: &str) {
    let def = default_of("get-default-sink");
    if on {
        if def == game { let _ = Command::new("pactl").args(["set-default-sink", GAME_SINK]).status(); }
        // apps + the THX filter's output go through the enhancement; our own output stays on the headset
        move_streams(true, game, GAME_SINK, &[GAME_OUT]);
    } else {
        if def == GAME_SINK { let _ = Command::new("pactl").args(["set-default-sink", game]).status(); }
        move_streams(true, GAME_SINK, game, &[]);
    }
}

fn route_mic(on: bool, mic: &str) {
    let def = default_of("get-default-source");
    if on {
        if def == mic { let _ = Command::new("pactl").args(["set-default-source", MIC_SRC]).status(); }
        move_streams(false, mic, MIC_SRC, &[MIC_IN]);
    } else {
        if def == MIC_SRC { let _ = Command::new("pactl").args(["set-default-source", mic]).status(); }
        move_streams(false, MIC_SRC, mic, &[]);
    }
}

/// Where game audio currently enters: the enhanced sink while playback enhancement is on, else the headset.
pub fn game_front(game: &str) -> String {
    if load().audio.on && node_id(GAME_SINK).is_some() { GAME_SINK.into() } else { game.into() }
}

// ---------------------------------------------------------------- commands
#[tauri::command]
pub async fn enh_get() -> Result<State, String> { Ok(load()) }

/// side = "audio" | "mic"; mode = Synapse tab ("normalization", "bass", "clarity", "noise", "gate"); level = slider 0..100
/// (gate: 1..3). Changing only mode/level while on = live parameter change, no rerouting.
#[tauri::command]
pub async fn enh_set(side: String, on: bool, mode: String, level: f64, needle: String) -> Result<State, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut st = load();
        let s = if side == "mic" { &mut st.mic } else { &mut st.audio };
        let was_on = s.on;
        s.on = on;
        if !mode.is_empty() {
            s.mode = mode.clone();
            s.levels.insert(mode, level);
        }
        save(&st);
        let reroute = was_on != on;
        apply(&st, &needle, if reroute { Some(side.as_str()) } else { Some("none") })?;
        Ok(st)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// At startup: put the saved enhancement state back (waits for the headset to show up in PipeWire).
pub fn restore(needle_fn: impl Fn() -> Option<String> + Send + 'static) {
    std::thread::spawn(move || {
        let st = load();
        if !st.audio.on && !st.mic.on { return }
        for _ in 0..60 {
            if let Some(n) = needle_fn() {
                if apply(&st, &n, None).is_ok() { return }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_is_passthrough() {
        let p = params("audio", None, 55.0);
        assert!(p.iter().any(|(k, v)| k == "bass:Gain" && *v == 0.0));
        assert!(p.iter().any(|(k, v)| k == "lim:Compressor ratio" && *v == 1.0));
    }
    #[test]
    fn values_match_reference() {
        let get = |p: &Params, k: &str| p.iter().find(|(kk, _)| kk == k).unwrap().1;
        let p = params("audio", Some("bass"), 55.0);
        assert!((get(&p, "bass:Gain") - 4.95).abs() < 1e-9);
        let p = params("mic", Some("noise"), 60.0);
        assert!((get(&p, "nc:Expander threshold (dB)") + 36.5).abs() < 1e-9);
        assert!((get(&p, "nc:Mute threshold (dB)") + 77.7).abs() < 1e-9);
        let p = params("audio", Some("normalization"), 100.0);
        assert!((get(&p, "norm:Input gain (dB)") - 16.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod dump {
    use super::*;
    /// RAZERCTL_DUMP=dir cargo test dump -- --ignored : writes params grid + a sample conf for cross-checking with dsp/enh.py
    #[test]
    #[ignore]
    fn dump() {
        let dir = std::env::var("RAZERCTL_DUMP").unwrap();
        let mut grid = serde_json::Map::new();
        for (side, modes) in [("audio", vec!["normalization", "bass", "clarity"]), ("mic", vec!["normalization", "clarity", "noise", "gate"])] {
            for m in modes {
                let lv: Vec<f64> = if m == "gate" { vec![1.0, 2.0, 3.0] } else { vec![0.0, 10.0, 25.0, 40.0, 55.0, 60.0, 80.0, 100.0] };
                for l in lv {
                    let p = params(side, Some(m), l);
                    grid.insert(format!("{side}|{m}|{l}"), serde_json::to_value(p.into_iter().collect::<BTreeMap<_, _>>()).unwrap());
                }
            }
        }
        fs::write(format!("{dir}/rust_params.json"), serde_json::to_string(&grid).unwrap()).unwrap();
        let mut st = State::default();
        st.audio = Side { on: true, mode: "bass".into(), levels: [("bass".into(), 100.0)].into() };
        st.mic = Side { on: true, mode: "noise".into(), levels: [("noise".into(), 60.0)].into() };
        fs::write(format!("{dir}/conf.conf"), conf("GAME", "MIC", &st, "/tmp/razer_dyn.so")).unwrap();
    }
}
