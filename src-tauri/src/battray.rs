// Optional per-device system-tray icons showing the battery percentage.
// The icon is drawn here: big digits, a small device glyph and a level bar.

use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::LazyLock,
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter,
};

pub struct DevInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub battery: Option<u8>,
    pub charging: bool,
    pub active: bool,
}

static ENABLED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| {
    Mutex::new(fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
});
// Last real reading per device, so the icon stays put while the device sleeps.
static LAST: LazyLock<Mutex<HashMap<String, (u8, bool, String, String)>>> = LazyLock::new(Default::default);
// device id -> (tray id, state key, visible).
// Trays are created once and then only shown/hidden: removing a libappindicator tray never
// unregisters it (it just goes Passive), so a re-created one with the same id was ignored.
static SHOWN: LazyLock<Mutex<HashMap<String, (String, String, bool)>>> = LazyLock::new(Default::default);

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("razerctl/battray.json")
}

pub fn enabled() -> Vec<String> {
    ENABLED.lock().iter().cloned().collect()
}

pub fn set(id: &str, on: bool) {
    let mut e = ENABLED.lock();
    if on {
        e.insert(id.to_string());
    } else {
        e.remove(id);
    }
    if let Some(d) = path().parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(path(), serde_json::to_string(&*e).unwrap());
}

fn hide(app: &AppHandle, id: &str) {
    if let Some(e) = SHOWN.lock().get_mut(id) {
        if e.2 {
            if let Some(t) = app.tray_by_id(&e.0) {
                let _ = t.set_visible(false);
            }
            e.2 = false;
        }
    }
}

/// Create, update or remove the battery trays to match the devices. Runs on the main thread.
pub fn sync(app: &AppHandle, devs: Vec<DevInfo>) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let app = &app2;
        let en = ENABLED.lock().clone();
        let mut shown = SHOWN.lock();
        let mut keep = HashSet::new();
        for d in &devs {
            if !en.contains(&d.id) {
                continue;
            }
            // A sleeping/idle wireless device reports no battery; keep showing the last reading.
            let (pct, charging, asleep) = match d.battery.filter(|_| d.active) {
                Some(p) => {
                    LAST.lock().insert(d.id.clone(), (p, d.charging, d.name.clone(), d.kind.clone()));
                    (Some(p), d.charging, false)
                }
                None => match LAST.lock().get(&d.id) {
                    Some(&(p, ..)) => (Some(p), false, true),
                    None => (None, false, true),
                },
            };
            keep.insert(d.id.clone());
            let tip = match pct {
                Some(p) if asleep => format!("{}: {}% (asleep, last reading)", d.name, p),
                Some(p) => format!("{}: {}%{}", d.name, p, if charging { " (charging)" } else { "" }),
                None => format!("{}: battery unknown (asleep or off)", d.name),
            };
            let key = format!("{pct:?}{charging}{asleep}");
            let img = Image::new_owned(render(pct, charging, &d.kind), SIZE, SIZE);
            if let Some((tid, k, vis)) = shown.get(&d.id).cloned() {
                if let Some(t) = app.tray_by_id(&tid) {
                    if k != key || !vis {
                        let _ = t.set_icon(Some(img));
                        let _ = t.set_tooltip(Some(&tip));
                        let _ = t.set_title(Some(&tip)); // Linux: hover text (tooltip is a no-op there)
                    }
                    if !vis {
                        let _ = t.set_visible(true);
                    }
                    shown.insert(d.id.clone(), (tid, key, true));
                    continue;
                }
            }
            let tid = format!("bat:{}", d.id);
            let id = d.id.clone();
            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, format!("batopen:{id}"), format!("Open {}", d.name), true, None::<&str>).unwrap(),
                    &MenuItem::with_id(app, format!("bathide:{id}"), "Hide this battery icon", true, None::<&str>).unwrap(),
                ],
            );
            let mut b = TrayIconBuilder::with_id(tid.clone()).icon(img).tooltip(&tip).title(&tip).show_menu_on_left_click(false);
            if let Ok(m) = menu {
                b = b.menu(&m);
            }
            let built = b
                .on_menu_event(|app, ev| {
                    let s = ev.id().as_ref().to_string();
                    if let Some(id) = s.strip_prefix("batopen:") {
                        crate::show_main(app);
                        let _ = app.emit("select-device", id.to_string());
                    } else if let Some(id) = s.strip_prefix("bathide:") {
                        set(id, false);
                        hide(app, id);
                        let _ = app.emit("battray-changed", ());
                    }
                })
                .on_tray_icon_event(move |tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        crate::show_main(tray.app_handle());
                        let _ = tray.app_handle().emit("select-device", id.clone());
                    }
                })
                .build(app);
            match built {
                Ok(_) => {
                    shown.insert(d.id.clone(), (tid, key, true));
                }
                Err(e) => eprintln!("battery tray: {e}"),
            }
        }
        let stale: Vec<String> = shown.iter().filter(|(k, v)| v.2 && !keep.contains(*k)).map(|(k, _)| k.clone()).collect();
        drop(shown);
        for k in stale {
            hide(app, &k);
        }
    });
}

// ---------------- icon drawing ----------------
// Same look as Joe's original tray icons (viper-tray / Lynapse): a device outline with the
// percentage inside, coloured by state: green charging, red <20, orange <40, blue otherwise.

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub const SIZE: u32 = 256;
const K: f32 = 0.552_284_8; // cubic circle constant

static FONT: LazyLock<Option<fontdue::Font>> = LazyLock::new(|| {
    ["/usr/share/fonts/TTF/DejaVuSans-Bold.ttf", "/usr/share/fonts/noto/NotoSans-Bold.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf"]
        .iter()
        .find_map(|p| fs::read(p).ok())
        .and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok())
});

fn state_color(pct: Option<u8>, charging: bool) -> [u8; 3] {
    match pct {
        None => [127, 140, 141],
        Some(_) if charging => [39, 174, 96],
        Some(p) if p < 20 => [231, 76, 60],
        Some(p) if p < 40 => [243, 156, 18],
        _ => [61, 174, 233],
    }
}

/// Clockwise quarter-ellipse arc from the current point to (ex, ey), bulging away from (cx, cy).
fn corner(pb: &mut PathBuilder, sx: f32, sy: f32, ex: f32, ey: f32, horizontal_first: bool) {
    if horizontal_first {
        pb.cubic_to(sx + (ex - sx) * K, sy, ex, ey - (ey - sy) * K, ex, ey);
    } else {
        pb.cubic_to(sx, sy + (ey - sy) * K, ex - (ex - sx) * K, ey, ex, ey);
    }
}

fn circle(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32) {
    if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
        pb.push_path(&p);
    }
}

fn rrect(pb: &mut PathBuilder, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
    pb.move_to(x0 + r, y0);
    pb.line_to(x1 - r, y0);
    corner(pb, x1 - r, y0, x1, y0 + r, true);
    pb.line_to(x1, y1 - r);
    corner(pb, x1, y1 - r, x1 - r, y1, false);
    pb.line_to(x0 + r, y1);
    corner(pb, x0 + r, y1, x0, y1 - r, true);
    pb.line_to(x0, y0 + r);
    corner(pb, x0, y0 + r, x0 + r, y0, false);
    pb.close();
}

fn stroke(pm: &mut Pixmap, pb: PathBuilder, w: f32, paint: &Paint) {
    if let Some(path) = pb.finish() {
        let st = Stroke { width: w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, paint, &st, Transform::identity(), None);
    }
}

fn fill(pm: &mut Pixmap, pb: PathBuilder, paint: &Paint) {
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, paint, FillRule::Winding, Transform::identity(), None);
    }
}

/// Largest text that fits the box, centred in it; `bold` thickens like PIL's stroke_width.
fn text(pm: &mut Pixmap, s: &str, color: [u8; 3], bx: f32, by: f32, bw: f32, bh: f32, max: f32, bold_div: f32) {
    let Some(font) = FONT.as_ref() else { return };
    let mut size = max;
    let measure = |size: f32| {
        let mut x = 0.0f32;
        let (mut l, mut r, mut top, mut bot) = (f32::MAX, f32::MIN, f32::MIN, f32::MAX);
        let mut glyphs = vec![];
        for ch in s.chars() {
            let (m, bm) = font.rasterize(ch, size);
            l = l.min(x + m.xmin as f32);
            r = r.max(x + m.xmin as f32 + m.width as f32);
            top = top.max(m.ymin as f32 + m.height as f32);
            bot = bot.min(m.ymin as f32);
            glyphs.push((x, m, bm));
            x += m.advance_width;
        }
        (l, r, top, bot, glyphs)
    };
    let (l, r, top, bot, glyphs) = loop {
        let m = measure(size);
        if (m.1 - m.0 <= bw && m.2 - m.3 <= bh) || size <= 14.0 {
            break m;
        }
        size -= 2.0;
    };
    let (tw, th) = (r - l, top - bot);
    let ox = bx + (bw - tw) / 2.0 - l; // pen origin x
    let base = by + (bh - th) / 2.0 + top; // baseline y
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let mut cov = vec![0u8; (w * h) as usize];
    for (gx, m, bm) in glyphs {
        let x0 = (ox + gx + m.xmin as f32).round() as i32;
        let y0 = (base - m.ymin as f32 - m.height as f32).round() as i32;
        for yy in 0..m.height as i32 {
            for xx in 0..m.width as i32 {
                let (x, y) = (x0 + xx, y0 + yy);
                if (0..w).contains(&x) && (0..h).contains(&y) {
                    let c = &mut cov[(y * w + x) as usize];
                    *c = (*c).max(bm[(yy * m.width as i32 + xx) as usize]);
                }
            }
        }
    }
    let rad = (size / bold_div).round().max(1.0) as i32;
    let mut out = cov.clone();
    for y in 0..h {
        for x in 0..w {
            let mut v = 0u8;
            for dy in -rad..=rad {
                for dx in -rad..=rad {
                    if dx * dx + dy * dy > rad * rad {
                        continue;
                    }
                    let (nx, ny) = (x + dx, y + dy);
                    if (0..w).contains(&nx) && (0..h).contains(&ny) {
                        v = v.max(cov[(ny * w + nx) as usize]);
                    }
                }
            }
            out[(y * w + x) as usize] = v;
        }
    }
    let data = pm.data_mut();
    for (i, a) in out.iter().enumerate() {
        if *a == 0 {
            continue;
        }
        let a = *a as u32;
        let px = &mut data[i * 4..i * 4 + 4];
        for c in 0..3 {
            px[c] = ((color[c] as u32 * a + px[c] as u32 * (255 - a)) / 255) as u8;
        }
        px[3] = (a + px[3] as u32 * (255 - a) / 255) as u8;
    }
}

fn draw_mouse(pm: &mut Pixmap, paint: &Paint, label: &str, color: [u8; 3]) {
    let lw = 20.0;
    let (x0, x1, y0, y1, r, notch) = (28.0, 228.0, 4.0 + lw / 2.0, 252.0 - lw / 2.0, 40.0, 26.0);
    let cx = 128.0;
    let y_dip = y0 + 40.0;
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + r, y0);
    pb.line_to(cx - notch, y0);
    pb.line_to(cx, y_dip);
    pb.line_to(cx + notch, y0);
    pb.line_to(x1 - r, y0);
    corner(&mut pb, x1 - r, y0, x1, y0 + r, true);
    pb.line_to(x1, y1 - r);
    corner(&mut pb, x1, y1 - r, x1 - r, y1, false);
    pb.line_to(x0 + r, y1);
    corner(&mut pb, x0 + r, y1, x0, y1 - r, true);
    pb.line_to(x0, y0 + r);
    corner(&mut pb, x0, y0 + r, x0 + r, y0, false);
    pb.close();
    // button split line
    let split = y0 + 88.0;
    pb.move_to(cx, y_dip);
    pb.line_to(cx, split);
    stroke(pm, pb, lw, paint);
    // scroll wheel
    let mut w = PathBuilder::new();
    rrect(&mut w, cx - 14.0, y0 + 8.0, cx + 14.0, y0 + 64.0, 11.0);
    fill(pm, w, paint);
    let (iw, ih) = (x1 - x0 - lw * 2.0, y1 - split - lw);
    text(pm, label, color, x0 + lw, split + lw / 2.0, iw, ih, 140.0, 28.0);
}

fn draw_headset(pm: &mut Pixmap, paint: &Paint, label: &str, color: [u8; 3]) {
    let lw = 16.0;
    let (ax0, ax1, top, cy, arm_bot, ear_r) = (30.0, 226.0, 6.0 + lw / 2.0, 62.0, 162.0, 22.0);
    let ear_cy = arm_bot + ear_r;
    let (ccx, rx, ry) = (128.0, (ax1 - ax0) / 2.0, cy - top);
    let mut pb = PathBuilder::new();
    // arms + headband (top half of an ellipse)
    pb.move_to(ax0, arm_bot);
    pb.line_to(ax0, cy);
    pb.cubic_to(ax0, cy - ry * K, ccx - rx * K, top, ccx, top);
    pb.cubic_to(ccx + rx * K, top, ax1, cy - ry * K, ax1, cy);
    pb.line_to(ax1, arm_bot);
    circle(&mut pb, ax0, ear_cy, ear_r);
    circle(&mut pb, ax1, ear_cy, ear_r);
    // boom mic
    let (by0, by1, bx1) = (ear_cy + ear_r - 2.0, SIZE as f32 - 42.0, 138.0);
    pb.move_to(ax0, by0);
    pb.line_to(ax0, by1);
    pb.line_to(bx1, by1);
    stroke(pm, pb, lw, paint);
    let mut cap = PathBuilder::new();
    circle(&mut cap, bx1, by1, 16.0);
    fill(pm, cap, paint);
    let (iw, ih) = (ax1 - ax0 - lw * 2.0, arm_bot - cy - lw);
    text(pm, label, color, ax0 + lw, cy + lw / 2.0, iw, ih, 120.0, 50.0);
}

/// Straight (non-premultiplied) RGBA, SIZE x SIZE.
fn render(pct: Option<u8>, charging: bool, kind: &str) -> Vec<u8> {
    let mut pm = Pixmap::new(SIZE, SIZE).unwrap();
    let color = state_color(pct, charging);
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], 255);
    paint.anti_alias = true;
    let label = pct.map(|p| p.to_string()).unwrap_or_else(|| "?".into());
    if kind == "headset" {
        draw_headset(&mut pm, &paint, &label, color);
    } else {
        draw_mouse(&mut pm, &paint, &label, color);
    }
    pm.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect()
}

/// Write a preview PNG (dev aid): RAZERCTL_BATICON=out.png
pub fn preview(path: &str) {
    let set: [(Option<u8>, bool, &str); 6] =
        [(Some(81), false, "headset"), (Some(100), true, "mouse"), (Some(35), false, "mouse"), (Some(9), false, "headset"), (Some(100), true, "headset"), (None, false, "mouse")];
    let s = SIZE as usize;
    let (w, h) = ((s + 20) * set.len(), s);
    let mut img = vec![40u8; w * h * 4];
    for (i, (p, c, k)) in set.iter().enumerate() {
        let ic = render(*p, *c, k);
        for y in 0..s {
            for x in 0..s {
                let sp = &ic[(y * s + x) * 4..(y * s + x) * 4 + 4];
                let a = sp[3] as u32;
                let d = (y * w + i * (s + 20) + x) * 4;
                for ch in 0..3 {
                    img[d + ch] = ((sp[ch] as u32 * a + img[d + ch] as u32 * (255 - a)) / 255) as u8;
                }
                img[d + 3] = 255;
            }
        }
    }
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(f, w as u32, h as u32);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&img).unwrap();
}
