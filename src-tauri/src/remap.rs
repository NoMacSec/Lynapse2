// Software button remapping + Hypershift for Razer mice (evdev grab -> uinput).
//
// Only active while at least one button has a non-default action. The physical
// mouse's event node is grabbed (exclusive) and its events are re-emitted on two
// virtual devices: a mouse that copies the real one's name/vendor/product (so
// KDE's per-device pointer settings still apply) and a keyboard for key actions.
// If razerctl exits or crashes the kernel drops the grab and the real mouse works
// normally again.

use evdev::{
    uinput::VirtualDevice, AttributeSet, Device, EventType, InputEvent, KeyCode, RelativeAxisCode,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    #[default]
    Default,
    Disabled,
    Mouse { button: String },
    Key { keys: Vec<String> },
    Hypershift,
    Dpi { op: String },
    Launch { cmd: String },
    /// Play a stored macro. mode: "once" (default), "times" (count), "held" (repeat while held), "toggle".
    Macro {
        id: String,
        #[serde(default)]
        mode: String,
        #[serde(default)]
        count: u32,
    },
    /// Type a piece of text (US layout).
    Text { text: String },
}

/// One macro step. Key/mouse `act` is "tap" (default), "down" or "up".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Step {
    Key {
        key: String,
        #[serde(default)]
        act: String,
    },
    Mouse {
        button: String,
        #[serde(default)]
        act: String,
    },
    Delay { ms: u64 },
    Text { text: String },
    Launch { cmd: String },
    Macro { id: String },
    Loop { count: u32 },
    Endloop,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Macro {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub standard: HashMap<String, Action>,
    #[serde(default)]
    pub hypershift: HashMap<String, Action>,
    #[serde(default)]
    pub macros: Vec<Macro>,
}

impl Config {
    fn active(&self) -> bool {
        self.standard.values().chain(self.hypershift.values()).any(|a| *a != Action::Default)
    }
}

fn btn_name(code: u16) -> Option<&'static str> {
    Some(match code {
        c if c == KeyCode::BTN_LEFT.0 => "left",
        c if c == KeyCode::BTN_RIGHT.0 => "right",
        c if c == KeyCode::BTN_MIDDLE.0 => "middle",
        c if c == KeyCode::BTN_SIDE.0 => "side",
        c if c == KeyCode::BTN_EXTRA.0 => "extra",
        _ => return None,
    })
}

fn btn_code(name: &str) -> Option<u16> {
    Some(match name {
        "left" => KeyCode::BTN_LEFT.0,
        "right" => KeyCode::BTN_RIGHT.0,
        "middle" => KeyCode::BTN_MIDDLE.0,
        "side" => KeyCode::BTN_SIDE.0,
        "extra" => KeyCode::BTN_EXTRA.0,
        _ => return None,
    })
}

#[derive(Default)]
pub struct Remapper {
    pub config: Mutex<Config>,
    pub status: Mutex<String>,
    running: Mutex<HashSet<PathBuf>>,
    playing: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

/// Event nodes of Razer mice ("…-event-mouse", interface 0 only).
fn mouse_nodes() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(rd) = fs::read_dir("/dev/input/by-id") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.contains("Razer") && n.ends_with("-event-mouse") && !n.contains("-if0") {
                if let Ok(p) = fs::canonicalize(e.path()) {
                    v.push(p);
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

impl Remapper {
    pub fn set_config(self: &Arc<Self>, cfg: Config, mouse_sysfs: Arc<dyn Fn() -> Option<String> + Send + Sync>) {
        // Running grab threads re-read the config on every event batch, and let go of the
        // mouse themselves once every button is back to default.
        *self.config.lock() = cfg;
        self.ensure(mouse_sysfs);
    }

    /// Start a grab thread for every present mouse node not already handled (called on hotplug too).
    pub fn ensure(self: &Arc<Self>, mouse_sysfs: Arc<dyn Fn() -> Option<String> + Send + Sync>) {
        if !self.config.lock().active() {
            *self.status.lock() = "off (all buttons default)".into();
            return;
        }
        let nodes = mouse_nodes();
        if nodes.is_empty() {
            *self.status.lock() = "waiting for mouse".into();
        }
        for node in nodes {
            if !self.running.lock().insert(node.clone()) {
                continue;
            }
            let me = self.clone();
            let ms = mouse_sysfs.clone();
            std::thread::spawn(move || {
                if let Err(e) = me.run(&node, ms) {
                    *me.status.lock() = format!("{}: {}", node.display(), e);
                    eprintln!("remap {}: {}", node.display(), e);
                }
                me.running.lock().remove(&node);
            });
        }
    }

    fn run(&self, node: &Path, mouse_sysfs: Arc<dyn Fn() -> Option<String> + Send + Sync>) -> std::io::Result<()> {
        let mut dev = Device::open(node)?;
        let name = dev.name().unwrap_or("Razer mouse").to_string();
        let id = dev.input_id();

        // Virtual mouse mirrors the real one (identity + capabilities).
        let mut mkeys = AttributeSet::<KeyCode>::new();
        if let Some(k) = dev.supported_keys() {
            for c in k.iter() {
                mkeys.insert(c);
            }
        }
        for c in [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE, KeyCode::BTN_SIDE, KeyCode::BTN_EXTRA] {
            mkeys.insert(c);
        }
        let mut rel = AttributeSet::<RelativeAxisCode>::new();
        if let Some(r) = dev.supported_relative_axes() {
            for c in r.iter() {
                rel.insert(c);
            }
        }
        for c in [RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_WHEEL_HI_RES] {
            rel.insert(c);
        }
        let mut vmouse = VirtualDevice::builder()?
            .name(name.as_str())
            .input_id(id)
            .with_keys(&mkeys)?
            .with_relative_axes(&rel)?
            .build()?;
        let mut kset = AttributeSet::<KeyCode>::new();
        for c in 1..=248u16 {
            kset.insert(KeyCode(c));
        }
        let mut vkbd = VirtualDevice::builder()?.name("razerctl virtual keyboard").with_keys(&kset)?.build()?;

        // Give the compositor a moment to pick up the virtual devices before taking the real one away.
        std::thread::sleep(Duration::from_millis(300));
        dev.grab()?;
        eprintln!("remap: grabbed {} ({})", node.display(), name);
        *self.status.lock() = format!("active on {}", name);

        let hyper = AtomicBool::new(false);
        let mut pressed: HashMap<String, Action> = HashMap::new(); // action resolved at press time
        let mut frame: Vec<InputEvent> = vec![];
        let mut kbd_frame: Vec<InputEvent> = vec![];

        loop {
            let events: Vec<InputEvent> = dev.fetch_events()?.collect();
            let cfg = self.config.lock().clone();
            if !cfg.active() && pressed.is_empty() {
                // everything back to default: hand the real mouse back (this batch passes through)
                for ev in events.iter().filter(|e| e.event_type() != EventType::SYNCHRONIZATION) {
                    frame.push(*ev);
                }
                if !frame.is_empty() {
                    vmouse.emit(&frame)?;
                }
                let _ = dev.ungrab();
                *self.status.lock() = "off (all buttons default)".into();
                return Ok(());
            }
            let resolve = |input: &str, hs: bool| -> Action {
                let std = cfg.standard.get(input).cloned().unwrap_or_default();
                if hs {
                    match cfg.hypershift.get(input) {
                        Some(a) if *a != Action::Default => a.clone(),
                        _ => std,
                    }
                } else {
                    std
                }
            };
            for ev in events {
                match ev.event_type() {
                    EventType::SYNCHRONIZATION => {
                        if !frame.is_empty() {
                            vmouse.emit(&frame)?;
                            frame.clear();
                        }
                        if !kbd_frame.is_empty() {
                            vkbd.emit(&kbd_frame)?;
                            kbd_frame.clear();
                        }
                    }
                    EventType::KEY => {
                        let Some(input) = btn_name(ev.code()) else {
                            frame.push(ev);
                            continue;
                        };
                        let down = ev.value() != 0;
                        let action = if down {
                            let a = resolve(input, hyper.load(Ordering::Relaxed));
                            pressed.insert(input.to_string(), a.clone());
                            a
                        } else {
                            pressed.remove(input).unwrap_or_default()
                        };
                        self.apply(input, &action, Some(ev.code()), down, true, &hyper, &mut frame, &mut kbd_frame, &mouse_sysfs);
                    }
                    EventType::RELATIVE
                        if ev.code() == RelativeAxisCode::REL_WHEEL.0 || ev.code() == RelativeAxisCode::REL_WHEEL_HI_RES.0 =>
                    {
                        let input = if ev.value() > 0 { "wheel_up" } else { "wheel_down" };
                        let action = resolve(input, hyper.load(Ordering::Relaxed));
                        if action == Action::Default {
                            frame.push(ev);
                        } else if ev.code() == RelativeAxisCode::REL_WHEEL.0 {
                            // one "click" of the wheel = press, sync, release of the action
                            self.apply(input, &action, None, true, false, &hyper, &mut frame, &mut kbd_frame, &mouse_sysfs);
                            for (v, f) in [(&mut vmouse, &mut frame), (&mut vkbd, &mut kbd_frame)] {
                                if !f.is_empty() {
                                    v.emit(f)?;
                                    f.clear();
                                }
                            }
                            self.apply(input, &action, None, false, false, &hyper, &mut frame, &mut kbd_frame, &mouse_sysfs);
                        } // hi-res companion event of a remapped wheel is dropped
                    }
                    _ => frame.push(ev),
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &self,
        input: &str,
        action: &Action,
        orig: Option<u16>,
        down: bool,
        is_button: bool,
        hyper: &AtomicBool,
        frame: &mut Vec<InputEvent>,
        kbd: &mut Vec<InputEvent>,
        mouse_sysfs: &Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) {
        let key = |c: u16, v: i32| InputEvent::new(EventType::KEY.0, c, v);
        match action {
            Action::Default => {
                if let Some(c) = orig {
                    frame.push(key(c, down as i32));
                }
            }
            Action::Disabled => {}
            Action::Hypershift => {
                if is_button {
                    hyper.store(down, Ordering::Relaxed);
                }
            }
            Action::Mouse { button } => match button.as_str() {
                "wheel_up" | "wheel_down" | "wheel_left" | "wheel_right" => {
                    if down {
                        let (axis, hi, v) = match button.as_str() {
                            "wheel_up" => (RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_WHEEL_HI_RES, 1),
                            "wheel_down" => (RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_WHEEL_HI_RES, -1),
                            "wheel_left" => (RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_HWHEEL_HI_RES, -1),
                            _ => (RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_HWHEEL_HI_RES, 1),
                        };
                        frame.push(InputEvent::new(EventType::RELATIVE.0, axis.0, v));
                        frame.push(InputEvent::new(EventType::RELATIVE.0, hi.0, v * 120));
                    }
                }
                b => {
                    if let Some(c) = btn_code(b) {
                        frame.push(key(c, down as i32));
                    }
                }
            },
            Action::Key { keys } => {
                let codes: Vec<u16> = keys.iter().filter_map(|k| k.parse::<KeyCode>().ok()).map(|k| k.0).collect();
                if down {
                    for c in &codes {
                        kbd.push(key(*c, 1));
                    }
                } else {
                    for c in codes.iter().rev() {
                        kbd.push(key(*c, 0));
                    }
                }
            }
            Action::Dpi { op } => {
                if down {
                    let op = op.clone();
                    let ms = mouse_sysfs.clone();
                    std::thread::spawn(move || {
                        if let Some(p) = ms() {
                            let _ = dpi_step(Path::new(&p), &op);
                        }
                    });
                }
            }
            Action::Launch { cmd } => {
                if down && !cmd.trim().is_empty() {
                    let _ = Command::new("sh").arg("-c").arg(cmd).spawn();
                }
            }
            Action::Macro { id, mode, count } => {
                let k = format!("btn:{input}");
                match mode.as_str() {
                    "held" if is_button => {
                        if down {
                            self.spawn_play(k, id.clone(), None);
                        } else {
                            self.stop_play(&k);
                        }
                    }
                    "toggle" => {
                        if down {
                            if self.is_playing(&k) {
                                self.stop_play(&k);
                            } else {
                                self.spawn_play(k, id.clone(), None);
                            }
                        }
                    }
                    m => {
                        if down {
                            self.spawn_play(k, id.clone(), Some(if m == "times" { (*count).max(1) } else { 1 }));
                        }
                    }
                }
            }
            Action::Text { text } => {
                if down {
                    self.spawn_steps(format!("btn:{input}"), vec![Step::Text { text: text.clone() }], vec![], Some(1));
                }
            }
        }
    }

    // ---------------- macro playback ----------------

    pub fn is_playing(&self, key: &str) -> bool {
        self.playing.lock().contains_key(key)
    }

    pub fn stop_play(&self, key: &str) {
        if let Some(f) = self.playing.lock().remove(key) {
            f.store(true, Ordering::Relaxed);
        }
    }

    /// Play macro `id` `reps` times (None = until stopped) on a background thread, keyed so a
    /// button can stop / restart its own playback.
    pub fn spawn_play(&self, key: String, id: String, reps: Option<u32>) {
        let macros = self.config.lock().macros.clone();
        let Some(m) = macros.iter().find(|m| m.id == id) else { return };
        let steps = m.steps.clone();
        self.spawn_steps(key, steps, macros, reps);
    }

    fn spawn_steps(&self, key: String, steps: Vec<Step>, macros: Vec<Macro>, reps: Option<u32>) {
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.playing.lock().insert(key.clone(), stop.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        let playing = self.playing.clone();
        std::thread::spawn(move || {
            let mut p = Player { stop: &stop, held: HashSet::new(), macros: &macros };
            let mut n = 0;
            while !steps.is_empty() {
                let t = Instant::now();
                if !p.run(&steps, 0) {
                    break;
                }
                n += 1;
                if reps.is_some_and(|r| n >= r) {
                    break;
                }
                if t.elapsed() < Duration::from_millis(5) {
                    std::thread::sleep(Duration::from_millis(5)); // repeating macro with no delays
                }
            }
            p.release_all();
            let mut pl = playing.lock();
            if pl.get(&key).is_some_and(|f| Arc::ptr_eq(f, &stop)) {
                pl.remove(&key);
            }
        });
    }
}

/// Separate virtual device used for macro output (keyboard + mouse buttons/wheel).
static MACRO_DEV: LazyLock<Mutex<Option<VirtualDevice>>> = LazyLock::new(|| Mutex::new(None));

fn build_macro_dev() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for c in 1..=248u16 {
        keys.insert(KeyCode(c));
    }
    for c in [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE, KeyCode::BTN_SIDE, KeyCode::BTN_EXTRA] {
        keys.insert(c);
    }
    let mut rel = AttributeSet::<RelativeAxisCode>::new();
    for c in [
        RelativeAxisCode::REL_X,
        RelativeAxisCode::REL_Y,
        RelativeAxisCode::REL_WHEEL,
        RelativeAxisCode::REL_HWHEEL,
        RelativeAxisCode::REL_WHEEL_HI_RES,
        RelativeAxisCode::REL_HWHEEL_HI_RES,
    ] {
        rel.insert(c);
    }
    VirtualDevice::builder()?.name("razerctl macro").with_keys(&keys)?.with_relative_axes(&rel)?.build()
}

fn vemit(evs: &[InputEvent]) {
    let mut g = MACRO_DEV.lock();
    if g.is_none() {
        match build_macro_dev() {
            Ok(d) => {
                *g = Some(d);
                drop(g);
                std::thread::sleep(Duration::from_millis(400)); // let the compositor pick it up
                g = MACRO_DEV.lock();
            }
            Err(e) => {
                eprintln!("macro device: {e}");
                return;
            }
        }
    }
    if let Some(d) = g.as_mut() {
        let _ = d.emit(evs);
    }
}

struct Player<'a> {
    stop: &'a AtomicBool,
    held: HashSet<u16>,
    macros: &'a [Macro],
}

impl Player<'_> {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn sleep(&self, ms: u64) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if self.stopped() {
                return false;
            }
            std::thread::sleep((end - Instant::now()).min(Duration::from_millis(10)));
        }
        !self.stopped()
    }

    fn key(&mut self, code: u16, v: i32) {
        vemit(&[InputEvent::new(EventType::KEY.0, code, v)]);
        if v == 1 {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
    }

    fn press(&mut self, code: u16, act: &str) {
        match act {
            "down" => self.key(code, 1),
            "up" => self.key(code, 0),
            _ => {
                self.key(code, 1);
                std::thread::sleep(Duration::from_millis(12));
                self.key(code, 0);
            }
        }
    }

    fn release_all(&mut self) {
        for c in std::mem::take(&mut self.held) {
            vemit(&[InputEvent::new(EventType::KEY.0, c, 0)]);
        }
    }

    fn type_text(&mut self, text: &str) -> bool {
        let shift = KeyCode::KEY_LEFTSHIFT.0;
        for ch in text.chars() {
            if self.stopped() {
                return false;
            }
            let Some((k, sh)) = char_key(ch) else { continue };
            if sh {
                self.key(shift, 1);
            }
            self.press(k, "tap");
            if sh {
                self.key(shift, 0);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        true
    }

    /// Runs a flat step list; Loop … Endloop pairs nest. Returns false when stopped.
    fn run(&mut self, steps: &[Step], depth: u32) -> bool {
        let mut i = 0;
        while i < steps.len() {
            if self.stopped() {
                return false;
            }
            match &steps[i] {
                Step::Key { key, act } => {
                    if let Ok(k) = key.parse::<KeyCode>() {
                        self.press(k.0, act);
                    }
                }
                Step::Mouse { button, act } => match button.as_str() {
                    "wheel_up" | "wheel_down" | "wheel_left" | "wheel_right" => {
                        let (a, h, v) = match button.as_str() {
                            "wheel_up" => (RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_WHEEL_HI_RES, 1),
                            "wheel_down" => (RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_WHEEL_HI_RES, -1),
                            "wheel_left" => (RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_HWHEEL_HI_RES, -1),
                            _ => (RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_HWHEEL_HI_RES, 1),
                        };
                        vemit(&[InputEvent::new(EventType::RELATIVE.0, a.0, v), InputEvent::new(EventType::RELATIVE.0, h.0, v * 120)]);
                    }
                    b => {
                        if let Some(c) = btn_code(b) {
                            self.press(c, act);
                        }
                    }
                },
                Step::Delay { ms } => {
                    if !self.sleep(*ms) {
                        return false;
                    }
                }
                Step::Text { text } => {
                    if !self.type_text(text) {
                        return false;
                    }
                }
                Step::Launch { cmd } => {
                    if !cmd.trim().is_empty() {
                        let _ = Command::new("sh").arg("-c").arg(cmd).spawn();
                    }
                }
                Step::Macro { id } => {
                    if depth < 8 {
                        if let Some(m) = self.macros.iter().find(|m| &m.id == id) {
                            let steps = m.steps.clone();
                            if !self.run(&steps, depth + 1) {
                                return false;
                            }
                        }
                    }
                }
                Step::Loop { count } => {
                    // find the matching Endloop (or run to the end)
                    let mut lvl = 0;
                    let mut j = i + 1;
                    while j < steps.len() {
                        match steps[j] {
                            Step::Loop { .. } => lvl += 1,
                            Step::Endloop if lvl == 0 => break,
                            Step::Endloop => lvl -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    for _ in 0..(*count).max(1) {
                        if !self.run(&steps[i + 1..j], depth) {
                            return false;
                        }
                    }
                    i = j;
                }
                Step::Endloop => {}
            }
            i += 1;
        }
        true
    }
}

/// US-layout key for a character: (code, needs shift).
fn char_key(c: char) -> Option<(u16, bool)> {
    let (name, sh): (String, bool) = if c.is_ascii_lowercase() || c.is_ascii_digit() {
        (c.to_ascii_uppercase().to_string(), false)
    } else if c.is_ascii_uppercase() {
        (c.to_string(), true)
    } else {
        let (n, s) = match c {
            ' ' => ("SPACE", false),
            '\n' => ("ENTER", false),
            '\t' => ("TAB", false),
            '-' => ("MINUS", false),
            '_' => ("MINUS", true),
            '=' => ("EQUAL", false),
            '+' => ("EQUAL", true),
            '[' => ("LEFTBRACE", false),
            '{' => ("LEFTBRACE", true),
            ']' => ("RIGHTBRACE", false),
            '}' => ("RIGHTBRACE", true),
            ';' => ("SEMICOLON", false),
            ':' => ("SEMICOLON", true),
            '\'' => ("APOSTROPHE", false),
            '"' => ("APOSTROPHE", true),
            '`' => ("GRAVE", false),
            '~' => ("GRAVE", true),
            '\\' => ("BACKSLASH", false),
            '|' => ("BACKSLASH", true),
            ',' => ("COMMA", false),
            '<' => ("COMMA", true),
            '.' => ("DOT", false),
            '>' => ("DOT", true),
            '/' => ("SLASH", false),
            '?' => ("SLASH", true),
            '!' => ("1", true),
            '@' => ("2", true),
            '#' => ("3", true),
            '$' => ("4", true),
            '%' => ("5", true),
            '^' => ("6", true),
            '&' => ("7", true),
            '*' => ("8", true),
            '(' => ("9", true),
            ')' => ("0", true),
            _ => return None,
        };
        (n.to_string(), s)
    };
    format!("KEY_{name}").parse::<KeyCode>().ok().map(|k| (k.0, sh))
}

/// Move the active DPI stage up/down/cycle via the driver's dpi_stages + dpi attributes.
fn dpi_step(dir: &Path, op: &str) -> std::io::Result<()> {
    let b = fs::read(dir.join("dpi_stages"))?;
    if b.len() < 5 {
        return Ok(());
    }
    let n = (b.len() - 1) / 4;
    let cur = b[0] as usize;
    let next = match op {
        "up" => (cur + 1).min(n),
        "down" => cur.saturating_sub(1).max(1),
        _ => cur % n + 1,
    };
    let mut out = b.clone();
    out[0] = next as u8;
    fs::write(dir.join("dpi_stages"), &out)?;
    let i = 1 + (next - 1) * 4;
    fs::write(dir.join("dpi"), &b[i..i + 4])?;
    Ok(())
}
