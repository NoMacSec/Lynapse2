// razerctl front end. Vanilla JS; pages render to HTML strings, then binders attach handlers.
const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

let devices = [];
let store = {};                 // persisted: { [deviceId]: { attr: value, ... }, _ui: {...} }
let view = { tab: 'home', sub: {} };
let back = [], fwd = [];
let renderSeq = 0;

// ---------------- utilities ----------------
const $ = s => document.querySelector(s);
const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const timers = {};
function debounce(key, fn, ms = 250) { clearTimeout(timers[key]); timers[key] = setTimeout(fn, ms); }
function toast(msg, ok = false) {
  const t = $('#toast'); t.textContent = String(msg); t.className = ok ? 'ok' : ''; t.style.display = 'block';
  debounce('toast', () => t.style.display = 'none', 3500);
}
function saveStore() { debounce('store', () => invoke('state_save', { data: store }).catch(toast), 400); }
function mem(key, def) { const d = store[view.tab] || {}; return key in d ? d[key] : def; }
function remember(key, v) { (store[view.tab] ||= {})[key] = v; saveStore(); }

let binders = [], uid = 0;
function bind(fn) { const id = 'c' + (++uid); binders.push(() => { const el = document.getElementById(id); if (el) fn(el); }); return id; }

// ---------------- device I/O ----------------
const dev = () => devices.find(d => d.id === view.tab);
const has = a => !!dev()?.attrs.includes(a);
async function rd(attr) { return invoke('read_attr', { path: dev().active, attr }); }
// Values the current page was rendered from; polled so changes made on the device itself
// (headset dial, buttons, other apps) show up without reopening the page.
let watched = {}, lastWrite = 0, pointerDown = false;
async function getv(attr, fallback) {
  // Driver returns -1 for values it has not seen yet; fall back to what we last wrote.
  if (!has(attr)) return mem(attr, fallback);
  try { const v = await rd(attr); watched[attr] = v; if (v !== '-1' && v !== '') return v; } catch (e) { }
  return mem(attr, fallback);
}
async function audioGet(kind) {
  const v = await invoke('audio_get', { kind, needle: audioNeedle() }).catch(() => null);
  watched['audio:' + kind] = v; return v;
}
async function readWatched(key) {
  if (key.startsWith('audio:')) return invoke('audio_get', { kind: key.slice(6), needle: audioNeedle() }).catch(() => null);
  return rd(key).catch(() => watched[key]);
}
let polling = false;
setInterval(async () => {
  const keys = Object.keys(watched), tab = view.tab, sub = curSub();
  if (polling || !keys.length || !dev()?.active || document.visibilityState === 'hidden') return;
  if (pointerDown || $('#modal').style.display === 'grid' || document.activeElement?.matches?.('input,textarea,select,[tabindex]')) return;
  polling = true;
  try {
    const fresh = {};
    for (const k of keys) fresh[k] = await readWatched(k);
    if (tab !== view.tab || sub !== curSub()) return; // page changed meanwhile
    const changed = keys.some(k => String(fresh[k]) !== String(watched[k]));
    if (Date.now() - lastWrite < 2500) { Object.assign(watched, fresh); return; } // our own write settling
    if (changed) render();
  } finally { polling = false; }
}, 1500);
addEventListener('pointerdown', () => pointerDown = true, true);
addEventListener('pointerup', () => { pointerDown = false; lastWrite = Date.now(); }, true);
async function wr(attr, v) {
  const d = dev(); if (!d?.active) return toast('Device not connected');
  lastWrite = Date.now();
  try { await invoke('write_attr', { path: d.active, attr, value: String(v) }); remember(attr, String(v)); lastWrite = Date.now(); }
  catch (e) { toast(e); }
}
async function wrBytes(attr, bytes) {
  const d = dev(); if (!d?.active) return toast('Device not connected');
  lastWrite = Date.now();
  try { await invoke('write_attr_bytes', { path: d.active, attr, value: bytes }); }
  catch (e) { toast(e); }
}
const audioNeedle = () => dev().name.replace(/^Razer /, '').replace(/ /g, '_');

// ---------------- components ----------------
const NA = 'na';
// Hover text for the ? circles, keyed by card title.
const TIPS = {
  'Active Noise Cancellation': 'Uses the headset mics to cancel outside noise. Higher levels cancel more. Ambient mode lets outside sound in instead, so you can hear people around you.',
  'Audio Enhancement': 'Extra processing on what you hear, on top of the EQ. Rebuilt from measurements of Synapse\'s audio processing. One mode at a time, like Synapse. While on, game audio goes through "BlackShark V3 Pro Game (Enhanced)".',
  'Audio Prompts': 'The voice/beep the headset plays when you turn it on, change modes or the battery is low.',
  'Dongle Indicator LED': 'The light on the mouse\'s HyperPolling dongle. Lynapse remembers your choice and sends it again whenever the dongle reconnects, because the dongle can\'t report what it is set to.',
  'Dynamic Sensitivity': 'Changes the DPI with how fast you move the mouse: slow moves stay precise, fast flicks go further.',
  'Game Profiles': 'Add a game and pick its sound. While that game\'s window is in front, Lynapse switches Spatial Audio and the EQ to your picks, and puts your normal settings back when you leave it. Works with Steam/Proton games (steam_app_ID or the game .exe) and native programs.',
  'In-Call Audio Mix': 'How much game audio you still hear while you are in a call.',
  'Indicator LED': 'The status light on the device.',
  'Low Power Mode': 'When the battery drops to this level, the device saves power (dimmer lights, lower polling) to last longer.',
  'Mic Enhancement': 'Processing on your mic before others hear it. Rebuilt from measurements of Synapse\'s audio processing. One mode at a time, like Synapse. While on, apps record from "BlackShark V3 Pro Mic (Enhanced)" (made the default mic).',
  'Mic Monitoring (Side Tone)': 'Plays your own voice back into the headset so you don\'t shout. 0 is off; higher is louder.',
  'Microphone': 'Mic volume and mute.',
  'Mouse Properties': 'Opens the system mouse settings (pointer speed, acceleration, scroll speed).',
  'Rotation': 'Corrects the angle you hold the mouse at, so a sideways swipe on your desk moves the pointer straight across.',
  'Sensitivity': 'DPI: how far the pointer moves per inch of mouse movement. Higher is faster. The stages are what the DPI button cycles through.',
  'Sensitivity Matcher': 'Helps match the DPI of another mouse so the new one feels the same.',
  'Smart Tracking': 'Sets how high the mouse can be lifted before the sensor stops tracking (lift-off distance).',
  'Sound Properties': 'Opens the system sound settings.',
  'Spatial Audio Modes': 'Where the virtual speakers sit around you when Spatial Audio is on.',
  'Ultra-Low Latency': 'HyperSpeed low-latency mode: less audio delay over the dongle, at the cost of battery. If sound drops out, try turning it off and on.',
  'Wireless Power Saving': 'How long the device waits with no use before it goes to sleep.',
  'Wireless Polling Rate': 'How many times a second the mouse reports to the PC over the dongle. Higher is smoother and more responsive but uses more battery.',
  'Wired Polling Rate': 'How many times a second the mouse reports to the PC over the cable. Higher is smoother and more responsive.',
  'Audio Equalizer': 'Shapes the sound you hear. Pick a preset, or drag the bands to make your own (saved as Custom). Left is bass, right is treble.',
  'Volume': 'The game volume on the headset\'s output. The switch turns this section\'s controls on or off (it does not change the sound).',
  'Mic Equalizer': 'Shapes how your mic sounds to other people. Presets (hover for names): Default, Esports (forward and present), Broadcast (fuller), Flat (less bass, crisper). Saved on the headset.',
  'THX': 'Stereo plays games as normal. Spatial Audio sends game sound through the virtual-surround filter, so 7.1 surround sounds like it comes from around you. If Spatial Audio is greyed out, you are missing the PipeWire profile (THX filter) it needs.',
  'Remap': 'Standard is what the buttons do normally. Hypershift is a second layer, used while you hold the Hypershift button.',
};
function help(tip, style = '') { return `<div class="help" ${tip ? `data-tip="${esc(tip)}"` : ''} ${style ? `style="${style}"` : ''}>?</div>`; }
function card(title, body, o = {}) {
  return `<div class="card ${o.na ? NA : ''} ${o.cls || ''}">${help(TIPS[title])}<h3>${title}${o.toggle || ''}</h3>${body}</div>`;
}
function toggle(on, onChange, o = {}) {
  const id = bind(el => el.onclick = () => { el.classList.toggle('on'); onChange && onChange(el.classList.contains('on')); });
  return `<div id="${id}" class="toggle ${on ? 'on' : ''} ${o.na ? NA : ''}"></div>`;
}
function seg(opts, val, onChange, cls = '') {
  const id = bind(el => el.querySelectorAll('[data-v]').forEach(b => b.onclick = () => {
    el.querySelectorAll('[data-v]').forEach(x => x.classList.remove('active')); b.classList.add('active');
    if (cls.includes('hsw')) b.classList.toggle('hs', b.dataset.v === 'hs');
    onChange && onChange(b.dataset.v);
  }));
  return `<div class="seg ${cls}" id="${id}">${opts.map(o => `<div data-v="${o.v}" class="${String(o.v) === String(val) ? 'active' : ''} ${cls.includes('hsw') && o.v === 'hs' && String(o.v) === String(val) ? 'hs' : ''} ${o.na ? NA : ''}">${o.label}</div>`).join('')}</div>`;
}
function btns(opts, val, onChange, cls = '') {
  const id = bind(el => el.querySelectorAll('[data-v]').forEach(b => b.onclick = () => {
    el.querySelectorAll('[data-v]').forEach(x => x.classList.remove('active')); b.classList.add('active'); onChange && onChange(b.dataset.v);
  }));
  return `<div class="btns ${cls}" id="${id}">${opts.map(o => `<div data-v="${o.v}" class="${String(o.v) === String(val) ? 'active' : ''} ${o.na ? NA : ''}">${o.label}</div>`).join('')}</div>`;
}
function radios(opts, val, onChange, o = {}) {
  const id = bind(el => el.querySelectorAll('[data-v]').forEach(b => b.onclick = () => {
    el.querySelectorAll('[data-v]').forEach(x => x.classList.remove('on')); b.classList.add('on'); onChange && onChange(b.dataset.v);
  }));
  return `<div id="${id}" class="${o.na ? NA : ''}">${opts.map(r => `<div class="radio ${String(r.v) === String(val) ? 'on' : ''}" data-v="${r.v}"><div class="dot"></div><div><div class="t">${r.t}</div>${r.s ? `<div class="s">${r.s}</div>` : ''}</div></div>`).join('')}</div>`;
}
function check(label, on, onChange, o = {}) {
  const id = bind(el => el.onclick = () => { el.classList.toggle('on'); onChange && onChange(el.classList.contains('on')); });
  return `<div id="${id}" class="check ${on ? 'on' : ''} ${o.na ? NA : ''}"><div class="box"></div>${label}</div>`;
}
function slider(o) {
  const step = o.step || 1;
  const id = bind(el => {
    const inp = el.querySelector('input'), b = el.querySelector('.bubble');
    const upd = () => {
      const p = (inp.value - o.min) / (o.max - o.min) * 100;
      inp.style.setProperty('--p', p + '%');
      if (b) { b.style.left = `calc(${p}% + ${9 - p * 0.18}px)`; b.textContent = o.fmt ? o.fmt(+inp.value) : inp.value; }
    };
    upd();
    inp.oninput = () => { upd(); if (o.onInput) debounce(id, () => o.onInput(+inp.value), o.delay ?? 250); };
  });
  return `<div class="slider ${o.na ? NA : ''} ${o.dim ? 'dim' : ''}" id="${id}" ${o.nobubble ? 'style="padding-top:6px"' : ''}>
    ${o.nobubble ? '' : '<span class="bubble"></span>'}
    <input type="range" min="${o.min}" max="${o.max}" step="${step}" value="${o.val}">
    ${o.ends ? `<div class="ends"><span>${o.ends[0]}</span>${o.ends[2] ? `<span>${o.ends[2]}</span>` : ''}<span>${o.ends[1]}</span></div>` : ''}</div>`;
}
const FREQS = ['31Hz', '63Hz', '125Hz', '250Hz', '500Hz', '1kHz', '2kHz', '4kHz', '8kHz', '16kHz'];
function eqChart(bands, o = {}) {
  const W = 1400, H = 420, L = 60, R = 1290, top = 30, bot = 370;
  const xs = bands.map((_, i) => L + i * (R - L) / 9);
  const y = v => top + (6 - v) * (bot - top) / 12;
  const id = bind(el => {
    if (!o.onChange) return;
    const svg = el.querySelector('svg'); let drag = -1;
    const vals = bands.slice();
    const redraw = () => {
      el.querySelectorAll('circle.pt').forEach((c, i) => c.setAttribute('cy', y(vals[i])));
      const pl = el.querySelector('polyline'); if (pl) pl.setAttribute('points', vals.map((v, i) => `${xs[i]},${y(v)}`).join(' '));
    };
    el.querySelectorAll('circle.pt').forEach((c, i) => c.onpointerdown = e => { drag = i; c.setPointerCapture(e.pointerId); });
    svg.onpointermove = e => {
      if (drag < 0) return;
      const r = svg.getBoundingClientRect(); const sy = (e.clientY - r.top) * H / r.height;
      const v = Math.max(-6, Math.min(6, Math.round(6 - (sy - top) * 12 / (bot - top))));
      if (v !== vals[drag]) { vals[drag] = v; redraw(); debounce(id, () => o.onChange(vals.slice()), 200); }
    };
    svg.onpointerup = () => drag = -1;
  });
  const grid = xs.map(x => `<line x1="${x}" x2="${x}" y1="${top}" y2="${bot}" stroke="${o.bars ? '#1f5a17' : '#2a2a2a'}" stroke-width="${o.bars ? 4 : 2}"/>
     <circle cx="${x}" cy="${y(0)}" r="3" fill="#2f6a25"/>`).join('');
  const line = o.bars ? '' : `<polyline points="${bands.map((v, i) => `${xs[i]},${y(v)}`).join(' ')}" fill="none" stroke="#44d62c" stroke-width="3"/>`;
  const pts = bands.map((v, i) => `<circle class="pt" cx="${xs[i]}" cy="${y(v)}" r="12" fill="#44d62c" style="cursor:${o.onChange ? 'ns-resize' : 'default'}"/>`).join('');
  const lbls = FREQS.map((f, i) => `<text x="${xs[i]}" y="${bot + 38}" fill="#ddd" font-size="22" text-anchor="middle">${f}</text>`).join('');
  const db = [6, 3, 0, -3, -6].map(v => `<text x="${R + 60}" y="${y(v) + 7}" fill="#ddd" font-size="20">${v > 0 ? '+' : ''}${v}${v ? 'dB' : ''}</text>`).join('');
  return `<div id="${id}" class="${o.na ? NA : ''}"><svg viewBox="0 0 ${W} ${H}" width="100%">${grid}${line}${pts}${lbls}${db}</svg></div>`;
}
function batteryIcon(pct, charging) {
  const h = Math.round(18 * (pct ?? 0) / 100);
  return `<svg width="14" height="26" viewBox="0 0 14 26"><rect x="4" y="0" width="6" height="3" fill="#ddd"/><rect x="1" y="3" width="12" height="22" rx="1" fill="${charging ? '#44d62c' : '#ddd'}"/>
    ${charging ? '<path d="M8 6L4 15h3l-1 7 4-10H7z" fill="#000"/>' : `<rect x="3" y="5" width="8" height="${18 - h}" fill="#151515"/>`}</svg>`;
}
const naNote = t => `<p class="muted" style="font-size:13px">${t}</p>`;


// ---------------- profiles (per device; every setting on the device's tabs) ----------------
// store[dev]._profiles = { name: snapshot | null }, store[dev]._profile = active name.
// The live device holds the active profile; it is captured when you switch away from it.
const PROF_ATTRS = {
  headset: ['v3pro_power_save', 'indicator_led', 'v3pro_ultra_low_latency', 'v3pro_anc', 'in_call_audio_mix', 'audio_prompts', 'v3pro_sidetone', 'game_chat_balance', 'mic_eq_preset', 'v3pro_headphone_eq', 'v3pro_thx_spatial_audio'],
  mouse: ['poll_rate', 'device_idle_time', 'charge_low_threshold'],
};
const PROF_MEM = { headset: ['eq_custom', 'mic_eq', 'anc_sub', 'power_save_last'], mouse: ['sens_match'] };
const clone = o => JSON.parse(JSON.stringify(o));
function profState() {
  const ds = (store[view.tab] ||= {});
  ds._profiles ||= { Default: null };
  if (!(ds._profile in ds._profiles)) ds._profile = Object.keys(ds._profiles)[0];
  return ds;
}
async function profSnapshot() {
  const d = dev(), k = d.kind, p = { attrs: {}, mem: {} };
  for (const a of PROF_ATTRS[k] || []) {
    if (!has(a)) continue;
    let v = await getv(a, null);
    if (v == null || v === '') continue;
    if (a === 'v3pro_headphone_eq') v = String(parseInt(v) || 0);
    p.attrs[a] = String(v);
  }
  const ds = store[view.tab] || {};
  for (const m of PROF_MEM[k] || []) if (m in ds) p.mem[m] = clone(ds[m]);
  if (k === 'mouse') {
    if (has('dpi_stages')) try { p.dpi = Array.from(await invoke('read_attr_bytes', { path: d.active, attr: 'dpi_stages' })); } catch (e) { }
    await loadRemap();
    p.remap = { standard: clone(remapCfg.standard), hypershift: clone(remapCfg.hypershift) };
  }
  return p;
}
async function profApply(p) {
  const d = dev(), ds = store[view.tab];
  for (const [m, v] of Object.entries(p.mem || {})) ds[m] = clone(v);
  for (const [a, v] of Object.entries(p.attrs || {})) {
    if (!has(a)) continue;
    if (a === 'v3pro_thx_spatial_audio') await setThx(v);
    else if (a === 'v3pro_headphone_eq') { const c = (p.mem?.eq_custom || {})[v]; await wr(a, c ? `${v} ${c.join(' ')}` : v); remember(a, v); }
    else await wr(a, v);
  }
  if (p.mem?.mic_eq && has('mic_eq')) { // after mic_eq_preset, which resets the bands
    const b = typeof p.mem.mic_eq === 'string' ? p.mem.mic_eq.split(/\s+/).map(Number) : p.mem.mic_eq;
    await wr('mic_eq', b.join(' ')); remember('mic_eq', b);
  }
  if (p.dpi && p.dpi.length >= 5 && has('dpi_stages')) {
    await wrBytes('dpi_stages', p.dpi);
    const i = 1 + (p.dpi[0] - 1) * 4; if (p.dpi.length >= i + 4) await wrBytes('dpi', p.dpi.slice(i, i + 4));
  }
  if (p.remap) {
    await loadRemap();
    remapCfg.standard = clone(p.remap.standard || {}); remapCfg.hypershift = clone(p.remap.hypershift || {});
    remapEditing = null;
    await invoke('remap_set', { config: remapCfg }).catch(toast);
  }
  saveStore();
}
let profBusy = false;
async function profSwitch(name, { fresh = false } = {}) {
  const d = dev(); if (!d?.active) return toast('Connect the device to switch profiles');
  const ds = profState(); if (profBusy || (name === ds._profile && !fresh)) return;
  profBusy = true;
  try {
    ds._profiles[ds._profile] = await profSnapshot(); // keep the outgoing profile's live settings
    const target = ds._profiles[name];
    ds._profile = name;
    if (target && !fresh) { toast(`Loading profile "${name}"…`, true); await profApply(target); toast(`Profile "${name}" loaded`, true); }
    else ds._profiles[name] = await profSnapshot(); // new profile starts as a copy of the current settings
    saveStore();
  } finally { profBusy = false; }
  render();
}
function askName(title, val, ok) {
  $('#modal').innerHTML = `<div class="card"><h3>${esc(title)}</h3><input id="pname" value="${esc(val)}" maxlength="40" style="width:100%;background:#111;border:1px solid #555;color:#fff;padding:8px;font-size:17px">
    <div style="text-align:right;margin-top:20px;display:flex;gap:12px;justify-content:flex-end"><span class="flatbtn" id="pcancel">Cancel</span><button class="greenbtn" id="pok">OK</button></div></div>`;
  $('#modal').style.display = 'grid';
  const i = $('#pname'); i.focus(); i.select();
  const close = () => $('#modal').style.display = 'none';
  const go = () => { const v = i.value.trim(); if (!v) return; close(); ok(v); };
  $('#pok').onclick = go; $('#pcancel').onclick = close;
  i.onkeydown = e => { if (e.key === 'Enter') go(); if (e.key === 'Escape') close(); };
}
function confirmBox(title, ok) {
  $('#modal').innerHTML = `<div class="card"><h3>${esc(title)}</h3><div style="text-align:right;display:flex;gap:12px;justify-content:flex-end"><span class="flatbtn" id="pcancel">Cancel</span><button class="greenbtn" id="pok" style="background:var(--red);color:#fff">Delete</button></div></div>`;
  $('#modal').style.display = 'grid';
  const close = () => $('#modal').style.display = 'none';
  $('#pok').onclick = () => { close(); ok(); }; $('#pcancel').onclick = close;
}
function uniqueName(base) { const ps = profState()._profiles; let n = base, i = 2; while (n in ps) n = `${base} ${i++}`; return n; }
function profileBar(d) {
  const ds = profState();
  return `<span class="iconbtn ${NA}">${ART.layers}</span>
    <div class="select" id="psel" style="cursor:pointer;position:relative">${esc(ds._profile)}</div>
    <span class="iconbtn" id="pmenu" style="font-size:22px;cursor:pointer;position:relative">•••</span>`;
}
function popup(anchor, items) {
  document.querySelectorAll('.ppop').forEach(x => x.remove());
  const r = anchor.getBoundingClientRect();
  const el = document.createElement('div');
  el.className = 'ppop';
  el.style.cssText = `position:fixed;left:${r.left}px;top:${r.bottom + 4}px;min-width:${Math.max(220, r.width)}px;background:#111;border:1px solid #555;z-index:95;padding:4px 0;font-size:16px`;
  el.innerHTML = items.map((it, i) => it === '-' ? '<div style="border-top:1px solid #333;margin:4px 0"></div>'
    : `<div data-i="${i}" style="padding:8px 14px;cursor:${it.na ? 'not-allowed' : 'pointer'};${it.na ? 'opacity:.4;' : ''}${it.active ? 'color:var(--green);' : ''}">${it.active ? '✓ ' : ''}${esc(it.label)}</div>`).join('');
  document.body.appendChild(el);
  el.querySelectorAll('[data-i]').forEach(x => {
    const it = items[+x.dataset.i];
    x.onmouseenter = () => x.style.background = '#222'; x.onmouseleave = () => x.style.background = '';
    x.onclick = e => { e.stopPropagation(); if (it.na) return; el.remove(); it.fn(); };
  });
  setTimeout(() => document.addEventListener('click', function off() { el.remove(); document.removeEventListener('click', off); }), 0);
}
function bindProfileBar(d) {
  const ds = profState();
  $('#psel').onclick = e => popup(e.currentTarget, Object.keys(ds._profiles).map(n => ({ label: n, active: n === ds._profile, fn: () => profSwitch(n) })));
  $('#pmenu').onclick = e => popup(e.currentTarget, [
    { label: 'Add new profile', fn: () => askName('New profile (starts with the current settings)', uniqueName('Profile'), n => { if (n in ds._profiles) return toast('That name is taken'); ds._profiles[n] = null; profSwitch(n, { fresh: true }); }) },
    { label: 'Rename', fn: () => askName('Rename profile', ds._profile, n => { if (n === ds._profile) return; if (n in ds._profiles) return toast('That name is taken'); const o = {}; for (const [k, v] of Object.entries(ds._profiles)) o[k === ds._profile ? n : k] = v; ds._profiles = o; ds._profile = n; saveStore(); render(); }) },
    { label: 'Delete', na: Object.keys(ds._profiles).length < 2, fn: () => confirmBox(`Delete profile "${ds._profile}"?`, async () => { const gone = ds._profile, next = Object.keys(ds._profiles).find(k => k !== gone); await profSwitch(next); delete profState()._profiles[gone]; saveStore(); render(); }) },
    ...(d.kind === 'headset' ? ['-', { label: 'Import profiles from Lynapse', fn: importLynapse }] : []),
  ]);
}
async function importLynapse() {
  const ly = await invoke('lynapse_config').catch(() => null);
  const profs = ly?.profiles || {};
  if (!Object.keys(profs).length) return toast('No Lynapse profiles found');
  const ds = profState(); let n = 0;
  for (const [name, lp] of Object.entries(profs)) {
    const eqc = {};
    for (const [slot, b] of Object.entries(lp.eq_custom || {})) if (EQ_PRESETS[slot] && JSON.stringify(b) !== JSON.stringify(EQ_PRESETS[slot].bands)) eqc[slot] = b;
    const mi = lp.mic_target_idx ?? 0;
    const p = { attrs: {}, mem: { eq_custom: eqc } };
    const A = (k, v) => { if (v !== undefined && v !== null) p.attrs[k] = String(v); };
    A('v3pro_headphone_eq', lp.eq_target_profile);
    A('v3pro_thx_spatial_audio', lp.thx_on === undefined ? undefined : lp.thx_on ? 1 : 0);
    A('v3pro_ultra_low_latency', lp.ull_on === undefined ? undefined : lp.ull_on ? 1 : 0);
    if (lp.anc_mode !== undefined) A('v3pro_anc', `${lp.anc_mode} ${lp.anc_level ?? 4}`);
    A('game_chat_balance', lp.gc_balance); A('in_call_audio_mix', lp.in_call_mix);
    A('audio_prompts', lp.audio_prompts === undefined ? undefined : lp.audio_prompts ? 1 : 0);
    A('v3pro_sidetone', lp.sidetone_level); A('mic_eq_preset', mi);
    A('v3pro_power_save', lp.pwr_timeout); A('indicator_led', lp.indicator_led);
    const mb = lp.mic_eq_custom?.[mi]; if (mb) p.mem.mic_eq = mb;
    const nm = uniqueName(name + ' (Lynapse)'); ds._profiles[nm] = p; n++;
  }
  saveStore(); render(); toast(`Imported ${n} Lynapse profile${n > 1 ? 's' : ''}. Pick one from the profile list to load it.`, true);
}

// ---------------- chrome ----------------
const SUBTABS = {
  home: ['DASHBOARD'],
  macro: ['MY MACROS', 'KEY BINDS'],
  mouse: ['CUSTOMIZE', 'PERFORMANCE', 'POWER', 'CALIBRATION', 'ADVANCED'],
  headset: ['SOUND', 'ENHANCEMENT', 'MIC', 'POWER', 'DEMO'],
  keyboard: ['CUSTOMIZE', 'LIGHTING', 'PERFORMANCE'],
  accessory: ['CUSTOMIZE'],
};
const SUB_NA = { home: [], macro: ['KEY BINDS'], headset: [], mouse: [], keyboard: ['CUSTOMIZE', 'LIGHTING', 'PERFORMANCE'], accessory: ['CUSTOMIZE'] };
const kindOf = tab => tab === 'home' || tab === 'macro' ? tab : (devices.find(d => d.id === tab)?.kind || 'accessory');
const icon = kind => kind === 'mouse' ? ART.mouseIcon : kind === 'headset' ? ART.headsetIcon : ART.kbdIcon;

function renderTabs() {
  const t = [`<div class="tab ${view.tab === 'home' ? 'active' : ''}" data-tab="home"><span class="ic round">${ART.homeIcon}</span>Lynapse 2.0</div>`,
  `<div class="tab ${view.tab === 'macro' ? 'active' : ''}" data-tab="macro"><span class="ic round">M</span>Macro</div>`];
  for (const d of devices) t.push(`<div class="tab ${view.tab === d.id ? 'active' : ''} ${d.active ? '' : 'offline'}" data-tab="${esc(d.id)}"><span class="ic">${icon(d.kind)}</span>${esc(d.name.replace(/^Razer /, ''))}</div>`);
  $('#tabs').innerHTML = t.join('');
  $('#tabs').querySelectorAll('[data-tab]').forEach(el => el.onclick = () => go(el.dataset.tab));
}
function go(tab, sub) {
  if (tab !== view.tab || (sub && sub !== view.sub[tab])) { back.push({ tab: view.tab, sub: view.sub[view.tab] }); fwd = []; }
  view.tab = tab; if (sub) view.sub[tab] = sub;
  (store._ui ||= {}).last = tab; saveStore();
  render();
}
// Devices without full pages (anything but the Viper V3 Pro / BlackShark V3 Pro) get one basic GENERAL tab.
function subtabsOf(tab) { const d = devices.find(x => x.id === tab); return d && !d.focused ? ['GENERAL'] : SUBTABS[kindOf(tab)]; }
function curSub() { const t = subtabsOf(view.tab), s = view.sub[view.tab]; return t.includes(s) ? s : t[0]; }

async function render() {
  const seq = ++renderSeq;
  if (view.tab !== 'home' && view.tab !== 'macro' && !dev()) view.tab = 'home';
  const kind = kindOf(view.tab), sub = curSub(), d = dev();
  renderTabs();
  $('#title').textContent = view.tab === 'home' ? 'LYNAPSE 2.0' : view.tab === 'macro' ? 'MACRO' : d.name;
  $('#subtabs').innerHTML = subtabsOf(view.tab).map(s => `<div class="pill ${s === sub ? 'active' : ''} ${d && !d.focused ? '' : SUB_NA[kind]?.includes(s) ? NA : ''}" data-s="${s}">${s}</div>`).join('');
  $('#subtabs').querySelectorAll('[data-s]').forEach(el => el.onclick = () => go(view.tab, el.dataset.s));
  if (d) {
    $('#profile').innerHTML = profileBar(d);
    bindProfileBar(d);
    $('#status').innerHTML = `${d.battery != null ? `<span>${d.battery} %</span>${batteryIcon(d.battery, d.charging)}` : ''}<span class="${NA}">${ART.book}</span><span class="${NA}"><svg width="24" height="24" viewBox="0 0 24 24"><circle cx="12" cy="12" r="11" fill="#ddd"/><text x="12" y="17" font-size="15" text-anchor="middle" font-weight="700" fill="#1a1a1a">?</text></svg></span>`;
  } else if (kind === 'macro') {
    $('#profile').innerHTML = ''; $('#status').innerHTML = '';
  } else { $('#profile').innerHTML = ''; $('#status').innerHTML = ''; }

  if (demo.playing && !(kind === 'headset' && sub === 'DEMO')) demoStop();
  if (macroRec && !(kind === 'macro' && sub === 'MY MACROS')) { window.removeEventListener('keydown', macroRec.kd, true); window.removeEventListener('keyup', macroRec.ku, true); macroRec = null; saveMacros(); }
  binders = []; watched = {};
  let html;
  try {
    if (d && !d.active) html = `<div class="empty">${esc(d.name)} is not connected.<br><span style="font-size:14px">Turn it on or plug it in; the tab comes alive automatically.</span></div>`;
    else html = await (PAGES[kind]?.[sub] || (() => `<div class="empty ${NA}" style="opacity:1">This page is not added yet.</div>`))(d);
  } catch (e) { html = `<div class="empty">Error: ${esc(e)}</div>`; }
  if (seq !== renderSeq) return; // a newer render started while we were reading the device
  const keep = $('#content').scrollTop;
  $('#content').innerHTML = html;
  binders.forEach(b => b());
  if (keep && store._ui?.lastPage === view.tab + sub) $('#content').scrollTop = keep;
  store._ui.lastPage = view.tab + sub;
}

// ---------------- pages ----------------
const PAGES = { home: {}, macro: {}, mouse: {}, headset: {}, keyboard: {}, accessory: {} };

// "Show battery % in system tray" checkbox: offered for any device whose driver/kernel reports a battery.
function trayCheck(d, bt) {
  const on = bt.includes(d.id);
  return d.battery != null || on ? `<div class="check ${on ? 'on' : ''}" style="font-size:14px;margin-top:10px;gap:8px" id="${bind(el => el.onclick = async e => {
    e.stopPropagation();
    const v = !el.classList.contains('on'); el.classList.toggle('on', v);
    await invoke('battray_set', { id: d.id, on: v }).catch(toast);
  })}"><div class="box" style="width:18px;height:18px"></div>Show battery % in system tray</div>` : '';
}

// ---------------- GENERAL: basic page for any other Razer device (only what its driver reports) ----------------
// openrazer lighting zones: keyboards use matrix_*, mice logo_* / scroll_* (each zone only if the driver has it)
const LIGHT_ZONES = [
  { p: 'matrix_', br: 'matrix_brightness', title: 'Lighting' },
  { p: 'logo_matrix_', br: 'logo_led_brightness', title: 'Logo Lighting' },
  { p: 'scroll_matrix_', br: 'scroll_led_brightness', title: 'Scroll Wheel Lighting' },
  { p: 'backlight_matrix_', br: 'backlight_led_brightness', title: 'Backlight' },
];
const FX = [
  { v: 'none', label: 'Off' }, { v: 'static', label: 'Static' }, { v: 'spectrum', label: 'Spectrum' },
  { v: 'wave', label: 'Wave' }, { v: 'breath', label: 'Breathing' },
];
async function generalPage(d) {
  const bt = await invoke('battray_get').catch(() => []);
  const cards = [];
  const conn = d.conn ? d.conn[0].toUpperCase() + d.conn.slice(1) : 'Connected';
  cards.push(card('Device', `<div style="display:flex;gap:18px;align-items:center"><div style="color:#ddd;transform:scale(1.6);margin:0 12px">${icon(d.kind)}</div>
      <div style="flex:1"><div style="font-size:17px">${esc(d.name)}</div>
      <div class="muted" style="font-size:14px">${conn}${d.firmware ? ' · firmware ' + esc(d.firmware) : ''}${d.serial ? ' · S/N ' + esc(d.serial) : ''}</div>${trayCheck(d, bt)}</div>
      ${d.battery != null ? `<div style="display:flex;gap:10px;align-items:center;font-size:17px">${d.battery}% ${batteryIcon(d.battery, d.charging)}</div>` : ''}</div>
      ${naNote(d.driver ? 'Basic support: Lynapse shows the settings this device\'s openrazer driver offers.' : 'No openrazer driver is bound to this device, so Lynapse can only show it (and its battery, if the kernel reports one).')}`));
  if (d.kind === 'mouse') {
    if (has('dpi')) {
      const dpi = +(String(await getv('dpi', '800')).split(':')[0]) || 800;
      cards.push(card('Sensitivity', `<p>DPI (dots per inch)</p>${slider({ min: 100, max: 30000, step: 50, val: dpi, ends: ['100', '30000'],
        onInput: v => wrBytes('dpi', [v >> 8, v & 255, v >> 8, v & 255]) })}`));
    }
    if (has('poll_rate')) {
      const poll = await getv('poll_rate', '1000');
      cards.push(card(d.conn === 'wireless' ? 'Wireless Polling Rate' : 'Wired Polling Rate', `<p>The frequency (Hz) of data updates in a second.</p>
        ${btns([125, 500, 1000].map(r => ({ v: r, label: r })), poll, v => wr('poll_rate', v))}`));
    }
    if (has('device_idle_time')) {
      const idle = Math.round((+await getv('device_idle_time', 300)) / 60) || 5;
      cards.push(card('Wireless Power Saving', `<p>Enter sleep mode if idle for (minutes)</p>${slider({ min: 1, max: 15, val: Math.min(15, Math.max(1, idle)), ends: ['1', '15'], onInput: v => wr('device_idle_time', v * 60) })}`));
    }
    if (has('charge_low_threshold')) {
      const thr = Math.round((+await getv('charge_low_threshold', 77)) * 100 / 255) || 30;
      cards.push(card('Low Power Mode', `<p>Enter Low Power Mode if the battery level is below (%)</p>${slider({ min: 5, max: 100, val: Math.min(100, Math.max(5, thr)), ends: ['5%', '100%'], onInput: v => wr('charge_low_threshold', Math.round(v * 255 / 100)) })}`));
    }
  }
  if (d.kind === 'headset') {
    const vol = await audioGet('game'), mic = await audioGet('mic');
    if (vol != null) cards.push(card('Volume', slider({ min: 0, max: 100, val: vol, ends: ['0', '100'], onInput: v => (lastWrite = Date.now(), invoke('audio_set', { kind: 'game', needle: audioNeedle(), value: v }).catch(toast)) })));
    if (mic != null) cards.push(card('Microphone', slider({ min: 0, max: 100, val: mic, ends: ['0', '100'], onInput: v => (lastWrite = Date.now(), invoke('audio_set', { kind: 'mic', needle: audioNeedle(), value: v }).catch(toast)) })));
    if (vol == null && mic == null) cards.push(card('Volume', '<p class="muted">This headset was not found in PipeWire.</p>'));
  }
  for (const z of LIGHT_ZONES) {
    const fx = FX.filter(f => has(`${z.p}effect_${f.v}`));
    if (!has(z.br) && !fx.length) continue;
    const br = Math.round((+await getv(z.br, 255)) * 100 / 255);
    const cur = mem(z.p + 'fx', fx.some(f => f.v === 'spectrum') ? 'spectrum' : 'static'), col = mem(z.p + 'fx_color', '#44d62c');
    const hex = c => [1, 3, 5].map(i => parseInt(c.slice(i, i + 2), 16));
    const apply = (v, c) => {
      remember(z.p + 'fx', v);
      const attr = `${z.p}effect_${v}`;
      if (v === 'static') return wrBytes(attr, hex(c));
      if (v === 'breath') return wrBytes(attr, [1]);   // 1 byte = random colours
      return wr(attr, 1);                               // none / spectrum / wave (direction 1)
    };
    // picking a colour selects Static and applies it (like Synapse)
    const fxRow = bind(() => {});
    const picker = bind(el => el.oninput = () => {
      remember(z.p + 'fx_color', el.value);
      document.querySelectorAll(`#${fxRow} [data-v]`).forEach(b => b.classList.toggle('active', b.dataset.v === 'static'));
      debounce(z.p + 'c', () => apply('static', el.value), 150);
    });
    cards.push(card(z.title, `${has(z.br) ? `<p>Brightness</p>${slider({ min: 0, max: 100, val: br, ends: ['0%', '100%'], onInput: v => wr(z.br, Math.round(v * 255 / 100)) })}` : ''}
      ${fx.length ? `<p style="margin-top:20px">Effect</p><div id="${fxRow}">${btns(fx.map(f => ({ v: f.v, label: f.label })), cur, v => apply(v, mem(z.p + 'fx_color', col)), 'wide')}</div>` : ''}
      ${has(`${z.p}effect_static`) ? `<div style="display:flex;gap:12px;align-items:center;margin-top:18px">Static colour <input type="color" id="${picker}" value="${col}"></div>` : ''}
      ${naNote('The device cannot report its current effect, so this shows the last one picked here.')}`));
  }
  return `<div class="banner" style="padding:14px 20px;margin-bottom:20px"><div style="font-size:17px;color:#ddd">Basic support for this device. Want more specific support? Contact <span style="color:var(--green)">ShenaniganJoe</span>.</div></div>
    <div class="grid2"><div class="col">${cards.filter((_, i) => i % 2 === 0).join('')}</div><div class="col">${cards.filter((_, i) => i % 2 === 1).join('')}</div></div>`;
}
for (const k of ['mouse', 'headset', 'keyboard', 'accessory']) PAGES[k]['GENERAL'] = generalPage;

PAGES.home['DASHBOARD'] = async () => {
  const bt = await invoke('battray_get').catch(() => []);
  const cards = devices.map(d => {
    const cb = trayCheck(d, bt);
    return `<div class="devcard" data-tab="${esc(d.id)}" id="${bind(el => el.onclick = () => go(d.id))}">
    <div style="color:#ddd;transform:scale(1.6);margin:0 10px">${icon(d.kind)}</div>
    <div style="flex:1"><div style="font-size:17px">${esc(d.name)}</div>
      <div class="meta">${d.active ? `${d.conn ? d.conn[0].toUpperCase() + d.conn.slice(1) : 'Connected'}${d.firmware ? ' · firmware ' + esc(d.firmware) : ''}${d.serial ? ' · S/N ' + esc(d.serial) : ''}` : 'Not connected'}</div>${cb}</div>
    ${d.battery != null ? `<div style="display:flex;gap:10px;align-items:center;font-size:17px">${d.battery}% ${batteryIcon(d.battery, d.charging)}</div>` : ''}</div>`;
  }).join('');
  const ver = await window.__TAURI__.app.getVersion().catch(() => '');
  return `<div class="banner"><h1>LYNAPSE 2.0</h1><div style="font-size:20px;color:#ddd">Device settings for Linux. Tabs appear as devices connect.</div>${ver ? `<div class="muted" style="font-size:14px;margin-top:8px">Version ${esc(ver)}</div>` : ''}</div>
    <div class="label" style="font-size:17px;margin-bottom:16px">▾ Devices</div>
    <div class="devcards">${cards || '<div class="muted">No supported devices found.</div>'}</div>`;
};

// ---------- macros (stored in remap.json, played by the backend's virtual "razerctl macro" device) ----------
let macroSel = null, macroStep = -1, macroRec = null;
const STEP_KINDS = [['delay', '#ccc', 'Delay'], ['key', '#44d62c', 'Keyboard function'], ['mouse', '#ff9900', 'Mouse function'], ['macro', '#44d62c', 'Macro'], ['launch', '#d63c8f', 'Launch / run command'], ['text', '#3b8fe0', 'Text function'], ['loop', '#e5323b', 'Loop']];
const stepColor = t => (STEP_KINDS.find(k => k[0] === t) || [, '#e5323b'])[1];
async function loadRemap() { if (!remapCfg) { const r = await invoke('remap_get'); remapCfg = r.config; } remapCfg.standard ||= {}; remapCfg.hypershift ||= {}; remapCfg.macros ||= []; }
function newMacro() {
  let n = 1; while (remapCfg.macros.some(q => q.name === `Macro ${n}`)) n++;
  const mm = { id: 'm' + Date.now().toString(36), name: `Macro ${n}`, steps: [] };
  remapCfg.macros.push(mm); macroSel = mm.id; macroStep = -1;
  return mm;
}
function saveMacros() { debounce('macros', () => invoke('remap_set', { config: remapCfg }).catch(toast), 300); }
const curMacro = () => remapCfg.macros.find(m => m.id === macroSel);
function newStep(t) {
  switch (t) {
    case 'delay': return { type: 'delay', ms: 50 };
    case 'key': return { type: 'key', key: 'KEY_A', act: 'tap' };
    case 'mouse': return { type: 'mouse', button: 'left', act: 'tap' };
    case 'macro': return { type: 'macro', id: (remapCfg.macros.find(m => m.id !== macroSel) || {}).id || '' };
    case 'launch': return { type: 'launch', cmd: '' };
    case 'text': return { type: 'text', text: '' };
  }
}
function recStop() {
  if (!macroRec) return;
  window.removeEventListener('keydown', macroRec.kd, true); window.removeEventListener('keyup', macroRec.ku, true);
  macroRec = null; saveMacros(); render();
}
function recStart(m, mode) {
  let last = performance.now(), at = macroStep >= 0 ? macroStep + 1 : m.steps.length;
  const push = st => {
    const now = performance.now(), gap = Math.round(now - last); last = now;
    const steps = [];
    if (m.steps.length || at > 0) {
      if (mode === 'recorded' && gap > 0) steps.push({ type: 'delay', ms: gap });
      else if (mode === 'default') steps.push({ type: 'delay', ms: 50 });
    }
    steps.push(st);
    m.steps.splice(at, 0, ...steps); at += steps.length; macroStep = at - 1;
    drawSteps();
  };
  const kd = e => { if (e.target.closest?.('input,select,textarea')) return; e.preventDefault(); e.stopPropagation(); if (e.repeat) return; const k = codeToKey(e.code); if (k) push({ type: 'key', key: k, act: 'down' }); };
  const ku = e => { if (e.target.closest?.('input,select,textarea')) return; e.preventDefault(); e.stopPropagation(); const k = codeToKey(e.code); if (k) push({ type: 'key', key: k, act: 'up' }); };
  window.addEventListener('keydown', kd, true); window.addEventListener('keyup', ku, true);
  macroRec = { kd, ku, push };
}
let drawSteps = () => { };
// two-click confirm: first click arms the button for 3 s, second click returns true
function armed(b, label) {
  if (b.dataset.armed) return true;
  const old = b.textContent, oldC = b.style.color; b.dataset.armed = 1; b.textContent = label; b.style.color = 'var(--red)';
  setTimeout(() => { delete b.dataset.armed; b.textContent = old; b.style.color = oldC; }, 3000);
  return false;
}

PAGES.macro['MY MACROS'] = async () => {
  await loadRemap();
  if (!curMacro()) { macroSel = remapCfg.macros[0]?.id || null; macroStep = -1; }
  const m = curMacro();
  const inp = (style, attrs) => `<input ${attrs} style="background:#111;border:1px solid #555;color:#fff;padding:4px 6px;font-size:15px;${style}">`;
  const sel = (opts, v, attrs) => `<select ${attrs} style="background:#111;border:1px solid #555;color:#fff;padding:4px;font-size:15px">${opts.map(([k, t]) => `<option value="${esc(k)}" ${String(k) === String(v) ? 'selected' : ''}>${esc(t)}</option>`).join('')}</select>`;
  const ACTS = [['tap', 'Tap'], ['down', 'Press'], ['up', 'Release']];

  const stepRow = (st, i, depth) => {
    let body = '';
    switch (st.type) {
      case 'delay': body = `Delay ${inp('width:90px', `type="number" min="0" data-f="ms" value="${st.ms}"`)} ms`; break;
      case 'key': body = `${sel(ACTS, st.act || 'tap', 'data-f="act"')} <span class="keypick" tabindex="0" style="border:1px solid #555;padding:4px 10px;cursor:pointer;outline:none;min-width:60px;display:inline-block;text-align:center">${esc(prettyKeys([st.key]))}</span>`; break;
      case 'mouse': body = `${sel(ACTS, st.act || 'tap', 'data-f="act"')} ${sel(MOUSE_FUNCS, st.button, 'data-f="button"')}`; break;
      case 'macro': body = `Play macro ${sel([['', '— choose —'], ...remapCfg.macros.filter(x => x.id !== macroSel).map(x => [x.id, x.name])], st.id, 'data-f="id"')}`; break;
      case 'launch': body = `Run ${inp('flex:1', `data-f="cmd" placeholder="program or shell command" value="${esc(st.cmd)}"`)}`; break;
      case 'text': body = `Type ${inp('flex:1', `data-f="text" placeholder="text to type" value="${esc(st.text)}"`)}`; break;
      case 'loop': body = `Loop ${inp('width:70px', `type="number" min="1" data-f="count" value="${st.count}"`)} times`; break;
      case 'endloop': body = `End loop`; break;
    }
    return `<div class="mstep ${i === macroStep ? 'sel' : ''}" data-i="${i}" style="display:flex;align-items:center;gap:10px;padding:7px 10px;margin-left:${depth * 26}px;cursor:pointer;${i === macroStep ? 'background:#232323;outline:1px solid var(--green-dim)' : ''}">
      <span style="color:${stepColor(st.type)}">■</span><span style="color:#777;width:28px;font-size:13px">${i + 1}</span>
      <div style="flex:1;display:flex;align-items:center;gap:10px">${body}</div>
      <span class="mup" title="Move up" style="color:#aaa">▲</span><span class="mdn" title="Move down" style="color:#aaa">▼</span><span class="mdel" title="Delete" style="color:#aaa;margin-left:6px">✕</span></div>`;
  };

  const listId = bind(el => {
    el.querySelectorAll('[data-m]').forEach(x => {
      x.onclick = e => { if (e.target.classList.contains('mx')) return; recStop(); macroSel = x.dataset.m; macroStep = -1; render(); };
      x.ondblclick = () => {
        const mm = remapCfg.macros.find(q => q.id === x.dataset.m), sp = x.querySelector('.mname');
        sp.innerHTML = `<input value="${esc(mm.name)}" style="background:#111;border:1px solid var(--green);color:#fff;padding:3px 6px;font-size:15px;width:190px">`;
        const i = sp.querySelector('input'); i.focus(); i.select();
        const done = () => { if (i.value.trim()) mm.name = i.value.trim(); saveMacros(); render(); };
        i.onkeydown = e => { e.stopPropagation(); if (e.key === 'Enter') done(); if (e.key === 'Escape') render(); };
        i.onblur = done; i.onclick = e => e.stopPropagation();
      };
      x.querySelector('.mx').onclick = e => {
        const mm = remapCfg.macros.find(q => q.id === x.dataset.m), b = e.target;
        if (!armed(b, 'Delete?')) return;
        remapCfg.macros = remapCfg.macros.filter(q => q !== mm);
        for (const L of ['standard', 'hypershift']) for (const k of Object.keys(remapCfg[L])) if (remapCfg[L][k].type === 'macro' && remapCfg[L][k].id === mm.id) delete remapCfg[L][k];
        if (macroSel === mm.id) macroSel = null;
        saveMacros(); render();
      };
    });
    el.querySelector('.mnew').onclick = () => {
      recStop(); newMacro(); saveMacros(); render();
    };
  });
  const addId = bind(el => el.querySelectorAll('[data-add]').forEach(x => x.onclick = () => {
    // With no macro selected, adding a step starts a new macro instead of doing nothing.
    const fresh = !curMacro();
    const mm = curMacro() || newMacro();
    const at = macroStep >= 0 ? macroStep + 1 : mm.steps.length;
    const ins = x.dataset.add === 'loop' ? [{ type: 'loop', count: 2 }, { type: 'endloop' }] : [newStep(x.dataset.add)];
    mm.steps.splice(at, 0, ...ins); macroStep = at; saveMacros();
    if (fresh) render(); else drawSteps();
  }));
  const edId = bind(el => {
    const box = el.querySelector('.msteps');
    drawSteps = () => {
      const mm = curMacro(); if (!mm) return;
      let depth = 0;
      box.innerHTML = mm.steps.length ? mm.steps.map((st, i) => { if (st.type === 'endloop') depth = Math.max(0, depth - 1); const r = stepRow(st, i, depth); if (st.type === 'loop') depth++; return r; }).join('')
        : `<div style="border:1px solid var(--orange);margin:30px auto;width:420px;padding:24px;text-align:center">Press <b>Record</b> and type, or click items on the left to add steps.</div>`;
      box.querySelectorAll('.mstep').forEach(row => {
        const i = +row.dataset.i, st = mm.steps[i];
        row.onclick = e => { if (e.target.closest('input,select,.keypick,.mup,.mdn,.mdel')) return; macroStep = macroStep === i ? -1 : i; drawSteps(); };
        row.querySelectorAll('[data-f]').forEach(f => {
          const upd = () => { const k = f.dataset.f; st[k] = f.type === 'number' ? Math.max(k === 'count' ? 1 : 0, parseInt(f.value) || 0) : f.value; saveMacros(); };
          f.oninput = upd; f.onchange = upd;
        });
        const kp = row.querySelector('.keypick');
        if (kp) { kp.onclick = () => { kp.textContent = 'press a key…'; kp.focus(); }; kp.onkeydown = e => { if (macroRec) return; e.preventDefault(); const k = codeToKey(e.code); if (k) { st.key = k; kp.textContent = prettyKeys([k]); saveMacros(); kp.blur(); } }; kp.onblur = () => kp.textContent = prettyKeys([st.key]); }
        row.querySelector('.mup').onclick = () => { if (i > 0) { [mm.steps[i - 1], mm.steps[i]] = [mm.steps[i], mm.steps[i - 1]]; macroStep = i - 1; saveMacros(); drawSteps(); } };
        row.querySelector('.mdn').onclick = () => { if (i < mm.steps.length - 1) { [mm.steps[i + 1], mm.steps[i]] = [mm.steps[i], mm.steps[i + 1]]; macroStep = i + 1; saveMacros(); drawSteps(); } };
        row.querySelector('.mdel').onclick = () => { mm.steps.splice(i, 1); if (macroStep >= mm.steps.length) macroStep = mm.steps.length - 1; saveMacros(); drawSteps(); };
      });
      if (macroRec) box.scrollTop = box.scrollHeight;
    };
    drawSteps();
    const mm = curMacro(); if (!mm) return;
    // mouse buttons / wheel are recorded when clicked inside the step area
    box.onmousedown = e => { if (!macroRec) return; e.preventDefault(); macroRec.push({ type: 'mouse', button: ['left', 'middle', 'right', 'side', 'extra'][e.button] || 'left', act: 'down' }); };
    box.onmouseup = e => { if (!macroRec) return; e.preventDefault(); macroRec.push({ type: 'mouse', button: ['left', 'middle', 'right', 'side', 'extra'][e.button] || 'left', act: 'up' }); };
    box.oncontextmenu = e => { if (macroRec) e.preventDefault(); };
    box.onwheel = e => { if (!macroRec) return; e.preventDefault(); macroRec.push({ type: 'mouse', button: e.deltaY < 0 ? 'wheel_up' : 'wheel_down', act: 'tap' }); };
    el.querySelector('.mrec').onclick = () => {
      if (macroRec) return recStop();
      recStart(mm, el.querySelector('.mrecmode').value); render();
    };
    el.querySelector('.mrecmode').onchange = e => { (store._ui ||= {}).recMode = e.target.value; saveStore(); };
    el.querySelector('.mclear').onclick = e => { if (mm.steps.length && armed(e.target, 'Click again to clear')) { mm.steps = []; macroStep = -1; saveMacros(); drawSteps(); } };
    el.querySelector('.mtest').onclick = async () => {
      recStop();
      await invoke('remap_set', { config: remapCfg }).catch(toast);
      const b = el.querySelector('.mtest');
      for (let s = 3; s > 0; s--) { b.textContent = `Plays in ${s}…`; await new Promise(r => setTimeout(r, 1000)); }
      b.textContent = '▶ Test'; invoke('macro_play', { id: mm.id }).catch(toast);
    };
  });

  const recMode = store._ui?.recMode || 'recorded';
  return `<div style="display:grid;grid-template-columns:310px 1fr;gap:14px;max-width:1100px;margin:0 auto">
    <div style="display:flex;flex-direction:column;gap:14px">
      <div class="card" id="${listId}" style="padding:20px 14px">
        <div style="display:flex;justify-content:space-between;align-items:center"><span style="font-size:19px;text-transform:uppercase;color:#ddd">Macros</span><span class="flatbtn mnew">+ New</span></div>
        <div style="margin-top:12px">${remapCfg.macros.map(x => `<div data-m="${esc(x.id)}" title="Double-click to rename" style="display:flex;justify-content:space-between;padding:8px 10px;cursor:pointer;${x.id === macroSel ? 'background:var(--green-dark);border-left:3px solid var(--green)' : ''}"><span><span class="mname">${esc(x.name)}</span> <span class="muted" style="font-size:12px">${x.steps.length} steps</span></span><span class="mx" style="color:#888">✕</span></div>`).join('') || '<div class="muted" style="padding:8px">No macros yet.</div>'}</div>
      </div>
      <div class="card" id="${addId}" style="padding:20px 14px"><div style="font-size:19px;text-transform:uppercase;color:#ddd">Add</div>
        ${STEP_KINDS.map(([t, c, l]) => `<div data-add="${t}" style="display:flex;gap:14px;align-items:center;margin:14px 0;font-size:15px;text-transform:uppercase;cursor:pointer"><span style="color:${c}">■</span>${l}</div>`).join('')}
        <p class="muted" style="font-size:12px;margin:0">${m ? 'Inserted after the selected step.' : 'Starts a new macro with this step.'}</p></div>
    </div>
    <div class="card" id="${edId}" style="min-height:640px;padding:16px;display:flex;flex-direction:column">
      ${m ? `<div style="display:flex;justify-content:center;gap:14px;align-items:center;flex-wrap:wrap">
        <span class="flatbtn mrec" style="${macroRec ? 'background:var(--red)' : ''}">${macroRec ? '■ Stop recording' : '● Record'}</span>
        <select class="mrecmode" style="background:#111;border:1px solid #555;color:#fff;padding:6px">${[['recorded', 'Record delays'], ['default', 'Default delay (50 ms)'], ['none', 'No delay']].map(([v, t]) => `<option value="${v}" ${v === recMode ? 'selected' : ''}>${t}</option>`).join('')}</select>
        <span class="flatbtn mtest">▶ Test</span><span class="flatbtn mclear">Clear</span></div>
        <div style="text-align:center;color:var(--green);font-size:18px;margin:14px 0 4px;text-transform:uppercase">${esc(m.name)}</div>
        ${macroRec ? `<p style="text-align:center;color:var(--red);margin:4px 0">Recording: type, or click/scroll inside the box below. Keys only reach Lynapse while this window is focused.</p>` : ''}
        <div class="msteps" style="flex:1;overflow:auto;max-height:560px;margin-top:8px"></div>
        <p class="muted" style="font-size:12px;margin:10px 0 0">Changes save automatically. Assign a macro to a mouse button under Viper → Customize → Macro. Text is typed with a US keyboard layout.</p>`
      : `<div style="border:1px solid var(--orange);margin:80px auto;width:370px;padding:30px;text-align:center">No macro selected. Click <b>+ New</b> to create one.</div>`}
    </div></div>`;
};

// ---------- mouse ----------
// ---------- mouse button remapping (software layer in the backend: evdev grab -> uinput) ----------
const INPUT_LABEL = { left: 'Left Click', right: 'Right Click', middle: 'Scroll Click', side: 'Mouse Button 4', extra: 'Mouse Button 5', wheel_up: 'Scroll Up', wheel_down: 'Scroll Down' };
const MOUSE_FUNCS = [['left', 'Left Click'], ['right', 'Right Click'], ['middle', 'Scroll Click'], ['side', 'Mouse Button 4'], ['extra', 'Mouse Button 5'], ['wheel_up', 'Scroll Up'], ['wheel_down', 'Scroll Down'], ['wheel_left', 'Scroll Left'], ['wheel_right', 'Scroll Right']];
const MEDIA = [['KEY_VOLUMEUP', 'Volume Up'], ['KEY_VOLUMEDOWN', 'Volume Down'], ['KEY_MUTE', 'Mute Volume'], ['KEY_PLAYPAUSE', 'Play / Pause'], ['KEY_NEXTSONG', 'Next Track'], ['KEY_PREVIOUSSONG', 'Previous Track'], ['KEY_STOPCD', 'Stop']];
const DPI_OPS = [['up', 'Sensitivity Stage Up'], ['down', 'Sensitivity Stage Down'], ['cycle', 'Cycle Up Sensitivity Stages']];
const KEY_PRETTY = { LEFTCTRL: 'Ctrl', RIGHTCTRL: 'Ctrl', LEFTSHIFT: 'Shift', RIGHTSHIFT: 'Shift', LEFTALT: 'Alt', RIGHTALT: 'AltGr', LEFTMETA: 'Meta', RIGHTMETA: 'Meta', ESC: 'Esc', SPACE: 'Space', ENTER: 'Enter', BACKSPACE: 'Backspace', LEFTBRACE: '[', RIGHTBRACE: ']', SEMICOLON: ';', APOSTROPHE: "'", GRAVE: '`', BACKSLASH: '\\', COMMA: ',', DOT: '.', SLASH: '/', MINUS: '-', EQUAL: '=', SYSRQ: 'PrtSc' };
function codeToKey(code) {
  const m = { ControlLeft: 'LEFTCTRL', ControlRight: 'RIGHTCTRL', ShiftLeft: 'LEFTSHIFT', ShiftRight: 'RIGHTSHIFT', AltLeft: 'LEFTALT', AltRight: 'RIGHTALT', MetaLeft: 'LEFTMETA', MetaRight: 'RIGHTMETA', Escape: 'ESC', Space: 'SPACE', Enter: 'ENTER', Tab: 'TAB', Backspace: 'BACKSPACE', CapsLock: 'CAPSLOCK', ArrowUp: 'UP', ArrowDown: 'DOWN', ArrowLeft: 'LEFT', ArrowRight: 'RIGHT', Minus: 'MINUS', Equal: 'EQUAL', BracketLeft: 'LEFTBRACE', BracketRight: 'RIGHTBRACE', Semicolon: 'SEMICOLON', Quote: 'APOSTROPHE', Backquote: 'GRAVE', Backslash: 'BACKSLASH', Comma: 'COMMA', Period: 'DOT', Slash: 'SLASH', Delete: 'DELETE', Insert: 'INSERT', Home: 'HOME', End: 'END', PageUp: 'PAGEUP', PageDown: 'PAGEDOWN', PrintScreen: 'SYSRQ', ScrollLock: 'SCROLLLOCK', Pause: 'PAUSE', NumLock: 'NUMLOCK', NumpadAdd: 'KPPLUS', NumpadSubtract: 'KPMINUS', NumpadMultiply: 'KPASTERISK', NumpadDivide: 'KPSLASH', NumpadEnter: 'KPENTER', NumpadDecimal: 'KPDOT', ContextMenu: 'COMPOSE' };
  if (m[code]) return 'KEY_' + m[code];
  let r;
  if ((r = code.match(/^Key([A-Z])$/))) return 'KEY_' + r[1];
  if ((r = code.match(/^Digit(\d)$/))) return 'KEY_' + r[1];
  if ((r = code.match(/^Numpad(\d)$/))) return 'KEY_KP' + r[1];
  if ((r = code.match(/^F(\d{1,2})$/))) return 'KEY_F' + r[1];
  return null;
}
const prettyKeys = keys => keys.map(k => { const n = k.replace(/^KEY_/, ''); return KEY_PRETTY[n] || (n.length === 1 ? n : n[0] + n.slice(1).toLowerCase()); }).join(' + ');
function actionLabel(input, a) {
  if (!a || a.type === 'default') return INPUT_LABEL[input];
  switch (a.type) {
    case 'disabled': return 'Disabled';
    case 'hypershift': return 'Hypershift';
    case 'mouse': return (MOUSE_FUNCS.find(m => m[0] === a.button) || [, a.button])[1];
    case 'key': { const med = MEDIA.find(m => a.keys.length === 1 && m[0] === a.keys[0]); return med ? med[1] : prettyKeys(a.keys); }
    case 'dpi': return (DPI_OPS.find(d => d[0] === a.op) || [, 'Sensitivity'])[1];
    case 'launch': return 'Launch: ' + a.cmd.split(/\s+/)[0].split('/').pop();
    case 'macro': { const mm = (remapCfg?.macros || []).find(x => x.id === a.id); return 'Macro: ' + (mm ? mm.name : '(deleted)'); }
    case 'text': return 'Text: "' + (a.text.length > 14 ? a.text.slice(0, 14) + '…' : a.text) + '"';
  }
  return '?';
}
let remapCfg = null, remapLayer = 'standard', remapEditing = null;
async function saveRemap() { try { await invoke('remap_set', { config: remapCfg }); } catch (e) { toast(e); } render(); }

function remapPanel(input) {
  const layer = remapLayer, cur = remapCfg[layer][input] || { type: 'default' };
  const cats = [['default', 'Default'], ['keyboard', 'Keyboard Function'], ['mouse', 'Mouse Function'], ['sensitivity', 'Sensitivity'], ['media', 'Multimedia'], ['hypershift', 'Hypershift'], ['launch', 'Launch Program'], ['disabled', 'Disable'], ['macro', 'Macro'], ['text', 'Text Function'], ['shortcut', 'Windows Shortcuts']];
  const catOf = a => a.type === 'key' ? (MEDIA.some(m => a.keys.length === 1 && a.keys[0] === m[0]) ? 'media' : 'keyboard') : a.type === 'dpi' ? 'sensitivity' : a.type;
  let cat = catOf(cur);
  const set = a => { if (a.type === 'default') delete remapCfg[layer][input]; else remapCfg[layer][input] = a; };
  const body = () => {
    switch (cat) {
      case 'keyboard': return `<p>Press a key or key combination:</p><div id="keycap" tabindex="0" style="border:1px solid var(--green);padding:14px;text-align:center;font-size:18px;outline:none;cursor:text">${cur.type === 'key' && catOf(cur) === 'keyboard' ? esc(prettyKeys(cur.keys)) : 'Click here, then press keys'}</div>`;
      case 'mouse': return radios(MOUSE_FUNCS.map(([v, t]) => ({ v, t })), cur.type === 'mouse' ? cur.button : '', v => { set({ type: 'mouse', button: v }); saveRemap(); });
      case 'media': return radios(MEDIA.map(([v, t]) => ({ v, t })), cur.type === 'key' ? cur.keys[0] : '', v => { set({ type: 'key', keys: [v] }); saveRemap(); });
      case 'sensitivity': return radios(DPI_OPS.map(([v, t]) => ({ v, t })), cur.type === 'dpi' ? cur.op : '', v => { set({ type: 'dpi', op: v }); saveRemap(); });
      case 'launch': return `<p>Command or program to run:</p><input id="launchcmd" value="${esc(cur.type === 'launch' ? cur.cmd : '')}" placeholder="e.g. firefox  or  spectacle -r" style="width:100%;background:#111;border:1px solid #555;color:#fff;padding:8px;font-size:16px"><div style="margin-top:12px"><button class="greenbtn" id="${bind(el => el.onclick = () => { const c = document.getElementById('launchcmd').value.trim(); if (c) { set({ type: 'launch', cmd: c }); saveRemap(); } })}">Save</button></div>`;
      case 'macro': {
        if (!remapCfg.macros.length) return `<p>No macros yet. Create one in the <b>Macro</b> tab first.</p>`;
        const a = cur.type === 'macro' ? cur : { type: 'macro', id: '', mode: 'once', count: 2 };
        const put = ch => { set({ ...a, mode: a.mode || 'once', count: a.count || 2, ...ch }); saveRemap(); };
        return `<p>Macro:</p>${radios(remapCfg.macros.map(x => ({ v: x.id, t: x.name, s: x.steps.length + ' steps' })), a.id, v => put({ id: v }))}
          <div class="label">Playback</div>${a.id ? radios([{ v: 'once', t: 'Play once' }, { v: 'times', t: 'Play multiple times' }, { v: 'held', t: 'Repeat while button is held' }, { v: 'toggle', t: 'Toggle continuous playback', s: 'Press once to start, again to stop' }], a.mode || 'once', v => put({ mode: v })) : '<p class="muted">Choose a macro first.</p>'}
          ${a.id && a.mode === 'times' ? `<div style="display:flex;gap:10px;align-items:center">Times <input type="number" min="1" value="${a.count || 2}" id="${bind(el => el.onchange = () => put({ count: Math.max(1, parseInt(el.value) || 1) }))}" style="width:80px;background:#111;border:1px solid #555;color:#fff;padding:6px"></div>` : ''}`;
      }
      case 'text': return `<p>Text to type when the button is pressed (US keyboard layout):</p><textarea id="textfn" rows="4" style="width:100%;background:#111;border:1px solid #555;color:#fff;padding:8px;font-size:15px">${esc(cur.type === 'text' ? cur.text : '')}</textarea><div style="margin-top:12px"><button class="greenbtn" id="${bind(el => el.onclick = () => { const t = document.getElementById('textfn').value; if (t) { set({ type: 'text', text: t }); saveRemap(); } })}">Save</button></div>`;
      case 'hypershift': return `<p>Hold this button to switch every other button to its Hypershift function. Set those with the Standard / Hypershift switch below the mouse.</p>`;
      case 'disabled': return `<p>This button will do nothing.</p>`;
      case 'default': return `<p>Restore this button's normal function (${INPUT_LABEL[input]}).</p>`;
    }
    return '';
  };
  const NA_CATS = ['shortcut'].concat(layer === 'hypershift' ? ['hypershift'] : []);
  const id = bind(el => {
    el.querySelectorAll('[data-cat]').forEach(c => c.onclick = () => {
      cat = c.dataset.cat;
      if (['default', 'hypershift', 'disabled'].includes(cat)) {
        if (cat === 'hypershift') for (const k of Object.keys(remapCfg.standard)) if (remapCfg.standard[k].type === 'hypershift') delete remapCfg.standard[k]; // one Hypershift button
        set({ type: cat }); saveRemap(); return;
      }
      el.querySelectorAll('[data-cat]').forEach(x => x.classList.toggle('active', x === c));
      binders = []; el.querySelector('.rbody').innerHTML = body(); binders.forEach(f => f()); wireKeycap();
    });
    el.querySelector('.rclose').onclick = () => { remapEditing = null; render(); };
    wireKeycap();
  });
  function wireKeycap() {
    const kc = document.getElementById('keycap'); if (!kc) return;
    let held = new Set(), combo = [];
    kc.onkeydown = e => { e.preventDefault(); const k = codeToKey(e.code); if (!k) return; held.add(k); if (!combo.includes(k)) combo.push(k); kc.textContent = prettyKeys(combo); };
    kc.onkeyup = e => { e.preventDefault(); held.delete(codeToKey(e.code)); if (held.size === 0 && combo.length) { const mods = combo.filter(k => /CTRL|SHIFT|ALT|META/.test(k)), rest = combo.filter(k => !mods.includes(k)); set({ type: 'key', keys: [...mods, ...rest] }); saveRemap(); } };
  }
  return `<div id="${id}" class="card" style="position:absolute;left:24px;top:10px;bottom:10px;width:380px;padding:20px 22px;overflow:auto;z-index:5;border:1px solid #333">
    <div style="display:flex;justify-content:space-between;align-items:center"><div style="font-size:17px;text-transform:uppercase">${layer === 'hypershift' ? '<span style="color:var(--orange)">Hypershift</span> · ' : ''}${INPUT_LABEL[input]}</div><span class="rclose" style="cursor:pointer;font-size:22px">✕</span></div>
    <div style="display:flex;flex-direction:column;margin:14px 0">${cats.map(([v, t]) => `<div data-cat="${v}" class="pill ${v === cat ? 'active' : ''} ${NA_CATS.includes(v) ? NA : ''}" style="border-radius:3px;text-transform:uppercase;font-size:13px">${t}</div>`).join('')}</div>
    <div class="rbody">${body()}</div></div>`;
}

PAGES.mouse['CUSTOMIZE'] = async () => {
  await loadRemap();
  const status = (await invoke('remap_get')).status;
  const lay = remapCfg[remapLayer];
  const lbl = input => {
    let a = lay[input];
    if (remapLayer === 'hypershift' && (!a || a.type === 'default')) a = remapCfg.standard[input]?.type === 'hypershift' ? { type: 'hypershift' } : null;
    const changed = a && a.type !== 'default';
    return `<span style="color:${remapLayer === 'hypershift' && changed ? 'var(--orange)' : changed ? 'var(--green)' : 'inherit'}">${esc(remapLayer === 'hypershift' && !a ? '—' : actionLabel(input, a))}</span>`;
  };
  const L = [['left', 40], ['middle', 102], ['extra', 165], ['side', 227]];
  const R = [['right', 40], ['wheel_up', 102], ['wheel_down', 165], ['dpi', 227]];
  const item = (input, y, side) => {
    const na = input === 'dpi';
    const id = na ? '' : bind(el => el.onclick = () => { remapEditing = input; render(); });
    const text = na ? 'Cycle Up Sensitivity Stages' : lbl(input);
    const pos = side === 'l' ? `right:calc(50% + 230px)` : `left:calc(50% + 230px)`;
    const line = side === 'l' ? `right:calc(50% + 100px)` : `left:calc(50% + 100px)`;
    return `<div ${id ? `id="${id}"` : ''} class="lbl ${side} ${na ? NA : ''}" style="${pos};top:${y + 30}px;cursor:pointer;${remapEditing === input ? 'text-decoration:underline' : ''}">${text}</div><div style="position:absolute;${line};width:120px;top:${y + 42}px;border-top:1px solid #555"></div>`;
  };
  const anyHs = Object.values(remapCfg.standard).some(a => a.type === 'hypershift');
  return `<div class="art dots" style="height:500px">
    <div style="position:absolute;left:50%;top:70px;transform:translateX(-50%)">${ART.mouse}</div>
    ${L.map(([i, y]) => item(i, y, 'l')).join('')}${R.map(([i, y]) => item(i, y, 'r')).join('')}
    ${remapEditing ? remapPanel(remapEditing) : ''}
  </div>
  <div class="seg-row"><span class="seg ${NA}"><div>▣</div></span>${seg([{ v: 'standard', label: 'Standard' }, { v: 'hs', label: 'Hypershift' }], remapLayer === 'hypershift' ? 'hs' : 'standard', v => { remapLayer = v === 'hs' ? 'hypershift' : 'standard'; remapEditing = null; render(); }, 'hsw')}${help(TIPS.Remap, 'position:static')}</div>
  <p class="muted" style="text-align:center;font-size:13px">Click a button name to change it. ${remapLayer === 'hypershift' ? (anyHs ? 'Hypershift layer: these apply while your Hypershift button is held; "—" keeps the standard function.' : 'Assign Hypershift to a button in Standard first.') : ''}<br>
  Remapping runs in Lynapse on this PC (not stored on the mouse) · status: ${esc(status)} · the bottom DPI button is handled by the mouse itself.</p>`;
};

const STAGE_COL = ['#e5323b', '#44d62c', '#3b8fe0', '#2fd0e0', '#e8d23a'];
const dpiToPos = d => Math.round(1000 * Math.log(d / 100) / Math.log(350));
const posToDpi = p => Math.max(100, Math.min(35000, Math.round(100 * Math.pow(350, p / 1000) / 50) * 50));

// ---------- sensitivity matcher: raw motion counts of another mouse vs this one over the same distance ----------
async function activeDpi(d) {
  const b = await invoke('read_attr_bytes', { path: d.active, attr: 'dpi_stages' });
  const i = 1 + (b[0] - 1) * 4; return { b, i, x: (b[i] << 8) | b[i + 1] };
}
async function setActiveDpi(d, v) {
  v = Math.max(100, Math.min(35000, Math.round(v)));
  const { b, i } = await activeDpi(d);
  b[i] = v >> 8; b[i + 1] = v & 255; b[i + 2] = v >> 8; b[i + 3] = v & 255;
  await wrBytes('dpi_stages', Array.from(b));
  await wrBytes('dpi', [v >> 8, v & 255, v >> 8, v & 255]);
  return v;
}
function sensMatcherCard(d) {
  const cals = mem('sens_cal', []), cur = mem('sens_match', '');
  const selId = bind(el => el.onchange = async () => {
    remember('sens_match', el.value);
    const c = cals.find(x => x.name === el.value); if (!c) return;
    try { const v = await setActiveDpi(d, c.dpi); toast(`Active stage set to ${v} DPI to match ${c.name}`, true); render(); } catch (e) { toast(e); }
  });
  const delId = bind(el => el.onclick = () => { remember('sens_cal', cals.filter(x => x.name !== cur)); remember('sens_match', ''); render(); });
  return card('Sensitivity Matcher', `<p>Create a new sensitivity matcher calibration from a different mouse.</p>
    <button class="greenbtn" id="${bind(el => el.onclick = () => sensWizard(d))}">Calibrate</button>
    <p style="margin-top:20px">Match the sensitivity of your mouse to a previously calibrated mouse.</p>
    <div style="display:flex;gap:12px;align-items:center"><select id="${selId}" style="width:290px"><option value="">None</option>${cals.map(c => `<option value="${esc(c.name)}" ${c.name === cur ? 'selected' : ''}>${esc(c.name)} · ${c.dpi} DPI</option>`).join('')}</select>
    ${cur ? `<span id="${delId}" style="cursor:pointer;color:#aaa;text-decoration:underline;font-size:14px">Remove</span>` : ''}</div>
    ${cals.length ? naNote('Choosing a calibration sets the active sensitivity stage to that DPI.') : ''}`);
}
function sensWizard(d) {
  // Both mice are held together and moved as one, so they cover exactly the same distance;
  // the backend counts raw sensor motion from every mouse at once.
  const me = d.name.replace(/^Razer /, '');
  const isMe = r => r.name.includes(me);
  let step = 0, timer = null;
  const close = () => { clearInterval(timer); invoke('calib_stop'); $('#modal').style.display = 'none'; };
  const fmt = n => Math.round(n).toLocaleString();
  const picks = r => ({ other: r.find(x => !isMe(x) && x.distance > 0), mine: r.find(isMe) });
  const show = async () => {
    clearInterval(timer);
    let body, next, res = null;
    if (step === 0) {
      body = `<p>This measures your other mouse's DPI so the ${esc(me)} can feel the same.</p>
        <p>1. Plug in both mice.<br>2. Put them side by side, touching. Holding them together with one hand or taping them works best.<br>3. Click <b>Start</b>, then slide them together in a straight line, back and forth a few times, without twisting.</p>
        <p class="muted">Raw sensor counts are used, so pointer acceleration doesn't affect the result.</p>`;
      next = 'Start';
    } else if (step === 1) {
      body = `<p>Move both mice together now: straight, back and forth a few times. Click <b>Done</b> when finished.</p><div id="calLive" style="font-size:17px;margin-top:16px;line-height:1.7">Waiting for movement…</div>`;
      next = 'Done';
    } else {
      res = picks(await invoke('calib_read').catch(() => []));
      await invoke('calib_stop');
      let dpi = 0, err = '';
      if (!res.other || !res.mine || res.mine.distance < 300 || res.other.distance < 300) err = 'Not enough movement from both mice. Make sure both are plugged in, then try again.';
      else try { const a = await activeDpi(d); dpi = Math.round(a.x * res.other.distance / res.mine.distance); } catch (e) { err = 'Could not read the current DPI: ' + e; }
      const nm = (res.other?.name || 'Other mouse').replace(/\s+/g, ' ').trim();
      body = err ? `<p>${esc(err)}</p>` : `<p>Your other mouse measured about <b style="color:var(--green);font-size:20px">${dpi} DPI</b></p>
        <p class="muted">${esc(nm)}: ${fmt(res.other.distance)} counts · ${esc(me)}: ${fmt(res.mine.distance)} counts</p>
        <div class="label">Save as</div><input id="calName" value="${esc(nm)}" maxlength="40" style="width:100%;background:#111;border:1px solid #555;color:#fff;padding:8px;font-size:16px">
        <p class="muted" style="font-size:13px;margin-top:14px">This matches the sensors only. If the old mouse used a different pointer speed in KDE or in the game, set the same value there too.</p>`;
      next = err ? 'Try again' : 'Save & apply';
      $('#modal').dataset.dpi = dpi; $('#modal').dataset.err = err ? 1 : '';
    }
    $('#modal').innerHTML = `<div class="card" style="width:600px"><h3>Sensitivity Matcher</h3>${body}
      <div style="display:flex;gap:12px;justify-content:flex-end;margin-top:22px">${step === 1 ? '<span class="flatbtn" id="calRedo">Reset</span>' : ''}<span class="flatbtn" id="calCancel">Cancel</span><button class="greenbtn" id="calNext">${next}</button></div></div>`;
    $('#modal').style.display = 'grid';
    $('#calCancel').onclick = close;
    if (step === 1) {
      $('#calRedo').onclick = () => invoke('calib_start').catch(toast);
      timer = setInterval(async () => {
        if ($('#modal').style.display === 'none') { clearInterval(timer); invoke('calib_stop'); return; }
        const p = picks(await invoke('calib_read').catch(() => [])), el = $('#calLive'); if (!el) return;
        el.innerHTML = `${esc(me)}: <b style="color:var(--green)">${fmt(p.mine?.distance || 0)}</b> counts<br>${p.other ? `${esc(p.other.name)}: <b style="color:var(--green)">${fmt(p.other.distance)}</b> counts` : '<span class="muted">Other mouse: no movement yet</span>'}`;
      }, 200);
    }
    $('#calNext').onclick = async () => {
      if (step === 0 || (step === 2 && $('#modal').dataset.err)) { try { await invoke('calib_start'); } catch (e) { return toast(e); } step = 1; return show(); }
      if (step === 1) { step = 2; return show(); }
      const name = ($('#calName').value || 'Other mouse').trim(), dpi = +$('#modal').dataset.dpi;
      remember('sens_cal', [...mem('sens_cal', []).filter(c => c.name !== name), { name, dpi }]);
      remember('sens_match', name);
      close();
      try { const v = await setActiveDpi(d, dpi); toast(`Active stage set to ${v} DPI`, true); } catch (e) { toast(e); }
      render();
    };
  };
  show();
}

async function spSet(cfg) { try { await invoke('smartpoll_set', { config: cfg }); } catch (e) { toast(e); } setTimeout(render, 400); }
PAGES.mouse['PERFORMANCE'] = async d => {
  const sp = await invoke('smartpoll_get').catch(() => ({}));
  let active = 1, stages = [];
  try {
    const b = await invoke('read_attr_bytes', { path: d.active, attr: 'dpi_stages' });
    active = b[0];
    for (let i = 1; i + 3 < b.length; i += 4) stages.push([(b[i] << 8) | b[i + 1], (b[i + 2] << 8) | b[i + 3]]);
  } catch (e) { toast(e); }
  const poll = await getv('poll_rate', '1000');
  const write = () => debounce('dpi', async () => {
    const bytes = [active];
    for (const [x, y] of stages) bytes.push(x >> 8, x & 255, y >> 8, y & 255);
    await wrBytes('dpi_stages', bytes);
    const [x, y] = stages[active - 1];
    await wrBytes('dpi', [x >> 8, x & 255, y >> 8, y & 255]);
  }, 300);
  const ticks = [100, 400, 1600, 6400, 35000];
  const tickHtml = `<div style="position:relative;height:18px;margin-top:6px;font-size:13px;color:#ccc">${ticks.map(t => `<span style="position:absolute;left:${dpiToPos(t) / 10}%;transform:translateX(-${dpiToPos(t) / 10}%)">${t}</span>`).join('')}</div>`;
  const rows = stages.map(([x], i) => {
    const sel = i + 1 === active;
    const id = bind(el => {
      el.onclick = e => { if (e.target.closest('input,.na')) return; if (active !== i + 1) { active = i + 1; write(); el.parentElement.querySelectorAll('.stage').forEach((s, j) => s.classList.toggle('sel', j === i)); } };
      const num = el.querySelector('input[type=number]');
      const rng = el.querySelector('input[type=range]');
      const val = el.querySelector('.val');
      const set = (v, from) => {
        v = Math.max(100, Math.min(35000, Math.round(v / 50) * 50)); stages[i] = [v, v];
        if (from !== 'num') num.value = v; val.textContent = v;
        if (from !== 'rng') { rng.value = dpiToPos(v); rng.dispatchEvent(new Event('input')); }
        write();
      };
      rng.addEventListener('input', e => { if (e.isTrusted) set(posToDpi(+rng.value), 'rng'); });
      num.onchange = () => set(+num.value, 'num');
    });
    return `<div class="stage ${sel ? 'sel' : ''}" id="${id}">
      <div class="num"><i style="background:${STAGE_COL[i % 5]}"></i>${i + 1}</div>
      <div><input type="number" min="100" max="35000" step="50" value="${x}" style="display:${sel ? 'block' : 'none'}"><span class="val" style="display:${sel ? 'none' : 'inline'}">${x}</span></div>
      <div>${slider({ min: 0, max: 1000, val: dpiToPos(x), nobubble: true, delay: 0 })}${tickHtml}</div>
      <div class="${NA}" title="X/Y"><svg width="34" height="34" viewBox="0 0 34 34" fill="none" stroke="#ddd"><rect x="1" y="1" width="32" height="32" rx="4"/><path d="M8 26L26 8"/><text x="8" y="15" fill="#ddd" stroke="none" font-size="11">X</text><text x="20" y="28" fill="#ddd" stroke="none" font-size="11">Y</text></svg></div>
      <div>${toggle(true, null, { na: true })}</div>
      <div class="${NA}" style="font-size:20px">⋮⋮</div></div>`;
  }).join('');
  // a stage row's own CSS selection logic shows the number box only on the selected row
  binders.push(() => document.querySelectorAll('.stage').forEach(s => {
    const obs = () => { const sel = s.classList.contains('sel'); s.querySelector('input[type=number]').style.display = sel ? 'block' : 'none'; s.querySelector('.val').style.display = sel ? 'none' : 'inline'; };
    new MutationObserver(obs).observe(s, { attributes: true, attributeFilter: ['class'] });
  }));
  const wireless = d.conn === 'wireless';
  const rates = [125, 500, 1000].concat(wireless ? [2000, 4000, 8000] : []);
  return `<div class="grid2"><div class="col">
    ${card('Sensitivity', `<p>The number of dots-per-inch (DPI) of mouse movement.</p>
      <div style="display:flex;gap:14px;align-items:center;margin-bottom:12px;font-size:17px">Sensitivity Stages ${toggle(true, null, { na: true })}</div>
      <div style="display:grid;grid-template-columns:42px 90px 1fr;gap:10px;font-size:17px;margin-bottom:6px"><span></span><span>DPI</span><span style="display:flex;justify-content:space-between"><span>100</span><span>35000</span></span></div>
      ${rows || '<p class="muted">Could not read DPI stages.</p>'}
      <div class="${NA}" style="margin-top:14px"><span class="flatbtn">+ Add stage</span></div>`)}
    ${card('Mouse Properties', `<div style="display:flex;gap:20px;align-items:center"><svg width="54" height="54" viewBox="0 0 24 24" fill="#ddd"><path d="M12 2a8 8 0 0 0-8 8v4a8 8 0 0 0 16 0v-4a8 8 0 0 0-8-8zm-1 2.1V9H6.1A6 6 0 0 1 11 4.1zm2 0A6 6 0 0 1 17.9 9H13z"/></svg>
      <a href="#" style="color:#ddd;font-size:17px" id="${bind(el => el.onclick = e => { e.preventDefault(); invoke('open_settings', { which: 'mouse' }).catch(toast); })}">Open System Mouse Settings</a></div>`)}
  </div><div class="col">
    ${card(wireless ? 'Wireless Polling Rate' : 'Wired Polling Rate', `<p>The frequency (Hz) of data updates in a second.</p>
      ${btns(rates.map(r => ({ v: r, label: r })), poll, v => wr('poll_rate', v))}
      <div style="margin-top:24px"><div style="display:flex;gap:14px;align-items:center;text-transform:uppercase;font-size:17px">Smart Polling Rate Switcher ${toggle(sp.enabled, on => spSet({ enabled: on, rate: sp.rate || +rates[rates.length - 1] }))}</div>
      <p style="margin-top:12px">Change the polling rate when running games in full screen mode.</p>
      <div style="${sp.enabled ? '' : 'opacity:.45'}">${btns(rates.map(r => ({ v: r, label: r })), sp.rate || rates[rates.length - 1], v => spSet({ enabled: sp.enabled, rate: +v }))}</div>
      ${sp.enabled ? naNote(sp.error ? 'Error: ' + esc(sp.error) : !sp.watching ? 'Starting window watcher…' : sp.in_game ? `Game in full screen (${esc(sp.window)}): running at ${sp.rate} Hz. The previous rate comes back when you leave it.` : 'Waiting for a full-screen game. Browsers and video players are ignored.') : ''}</div>`)}
    ${sensMatcherCard(d)}
  </div></div>`;
};

PAGES.mouse['POWER'] = async d => {
  const idle = Math.round((+await getv('device_idle_time', 300)) / 60) || 5;
  const thr = Math.round((+await getv('charge_low_threshold', 77)) * 100 / 255) || 30;
  return `<div class="grid2">
    ${card('Wireless Power Saving', `<p>Enter sleep mode if idle for (minutes)</p>${slider({ min: 1, max: 15, val: Math.min(15, Math.max(1, idle)), ends: ['1', '15'], onInput: v => wr('device_idle_time', v * 60) })}`, { na: !has('device_idle_time') })}
    ${card('Low Power Mode', `<p>When wireless, enter Low Power Mode if the battery level is below (%)</p>${slider({ min: 5, max: 100, val: Math.min(100, Math.max(5, thr)), ends: ['5%', '100%'], onInput: v => wr('charge_low_threshold', Math.round(v * 255 / 100)) })}`, { na: !has('charge_low_threshold') })}
    ${has('hyperpolling_wireless_dongle_indicator_led_mode') ? card('Dongle Indicator LED', `<p>Select what the indicator LED on the dongle will display:</p>${radios([
      { v: 1, t: 'CONNECTION STATUS', s: 'Shows whether the mouse is connected to the dongle.' },
      { v: 2, t: 'BATTERY STATUS', s: 'Colour follows the mouse battery level.' },
      { v: 3, t: 'BATTERY WARNING ONLY', s: 'Stays off; blinks only when the mouse needs charging.' }],
      // write-only in the driver (no readback), so show the last choice made here
      mem('hyperpolling_wireless_dongle_indicator_led_mode', '1'), v => wr('hyperpolling_wireless_dongle_indicator_led_mode', v))}
      <p class="muted" style="font-size:12px">The dongle can't report its current mode, so this shows the last one set here.</p>`) : ''}
  </div>`;
};

PAGES.mouse['CALIBRATION'] = () => `<div class="card ${NA}" style="text-align:center;padding:24px;margin-bottom:36px"><div style="color:var(--green);font-size:32px;text-transform:uppercase">Mouse Mat Surface Calibration</div>
  <p style="margin-top:12px">Set a tracking distance and use Asymmetric Cut-Off to adjust lift-off and landing distance across surfaces.</p></div>
  <div class="center">${card('Smart Tracking', `<p>Set a single cut off point to your preferred distance regardless of the surface it is placed on.</p>
    ${check('Enable Asymmetric Cut-off', false)}<div class="label" style="margin-top:28px">Tracking Distance</div>
    ${slider({ min: 0, max: 2, val: 1, nobubble: true, ends: ['LOW', 'HIGH', 'MEDIUM'] })}
    <div class="label">Reset</div><p>To reset your mouse manually, press and hold the left, right, and scroll wheel mouse buttons simultaneously for 7 seconds.</p>`, { na: true })}</div>`;

PAGES.mouse['ADVANCED'] = () => {
  const chart = `<svg viewBox="0 0 1400 420" width="100%"><g stroke="#2a2a2a">${[0, 1, 2, 3, 4, 5, 6, 7].map(i => `<line x1="${100 + i * 180}" x2="${100 + i * 180}" y1="20" y2="340"/>`).join('')}${[0, 1, 2, 3].map(i => `<line x1="100" x2="1380" y1="${340 - i * 100}" y2="${340 - i * 100}"/>`).join('')}</g>
    <polyline points="100,340 1360,170" stroke="#44d62c" stroke-width="3" fill="none"/>
    ${[0, 10, 20, 30, 40, 50, 60, 70].map((v, i) => `<text x="${100 + i * 180}" y="375" fill="#ddd" font-size="20" text-anchor="middle">${v}</text>`).join('')}
    ${['1.0', '1.2', '1.4', '1.6'].map((v, i) => `<text x="70" y="${346 - i * 100}" fill="#ddd" font-size="20" text-anchor="end">${v}</text>`).join('')}
    <text x="740" y="412" fill="#ddd" font-size="22" text-anchor="middle">Input Speed (count/ms)</text></svg>`;
  return `<div class="col">${card('Dynamic Sensitivity', `<p>Select a dynamic sensitivity preset or customize your own sensitivity curve.</p>
    ${btns([{ v: 'c', label: 'Classic' }, { v: 'n', label: 'Natural' }, { v: 'j', label: 'Jump' }, { v: 'x', label: 'Custom' }], 'c', null, 'wide')}${chart}`, { na: true, toggle: toggle(false) })}
    <div style="max-width:50%">${card('Rotation', `<p>Rotate the mouse's angle to get straight horizontal movement with any kind of grip style.</p>
    <div style="display:grid;place-items:center;margin:10px">${ART.mouseIcon.replace('width="22" height="26"', 'width="90" height="110"')}</div>
    ${slider({ min: -44, max: 44, val: 0, ends: ['-44°', '44°', '0°'], fmt: v => v + '°' })}`, { na: true, toggle: toggle(false) })}</div></div>`;
};

// ---------- headset ----------
const EQ_PRESETS = [
  { name: 'Default', icon: ART.sliders, bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { name: 'Game', icon: ART.pad, bands: [2, 2, 5, 5, 1, -1, 2, 3, 3, 3] },
  { name: 'Movie', icon: ART.film, bands: [3, 3, 3, -1, -4, -4, 2, 3, 3, 3] },
  { name: 'Music', icon: ART.note, bands: [2, 2, 0, 0, 1, -1, -1, 3, 3, 3] },
  { name: 'Esports', icon: ART.swords, bands: [1, 1, -1, 0, 2, 0, 4, 4, 4, -3], game: 'Counter-Strike 2', badge: '1',
    // game helper labels under the bands (widths measured from Synapse)
    groups: [['', 391], ['Jumping', 137], ['Footsteps', 264], ['Radio Commands', 264], ['Bomb Ticker', 138], ['', 138]] },
];
const EQ_GROUPS = [['Sub Bass', 142], ['Bass', 271], ['Mid Range', 528], ['Treble', 399]];
const bandGroups = i => (EQ_PRESETS[i].groups || EQ_GROUPS).map(([t, w]) => `<div style="flex:${w}">${t || '&nbsp;'}</div>`).join('');
function headsetArt(labels) {
  return `<div class="art dots" style="height:330px">
    <div style="position:absolute;left:50%;top:20px;transform:translateX(-50%)">${ART.headset}</div>
    ${labels.map(([t, y, na]) => `<div style="position:absolute;left:calc(50% + 50px);width:90px;top:${y + 10}px;border-top:1px solid #555"></div><div class="lbl ${na ? NA : ''}" style="left:calc(50% + 150px);top:${y}px">${t}</div>`).join('')}</div>`;
}
async function setThx(v) {
  await wr('v3pro_thx_spatial_audio', v);
  remember('thx_on', String(v));
  await invoke('thx_route', { on: String(v) === '1', needle: audioNeedle() }).catch(toast);
  if (demo.playing) demoRoute();
}
// THX is software (a PipeWire convolver sink). Without it the switch would do nothing, so grey it out.
let THX_OK = true;
const thxCheck = async () => (THX_OK = await invoke('thx_ok').catch(() => false));
const THX_MISSING = 'Spatial Audio is greyed out because you are missing the PipeWire profile (virtual-surround filter) it needs. Setup instructions are coming soon.';
function thxSeg(thx) {
  if (!THX_OK) return `<div class="seg-row"><div class="${NA}" title="${THX_MISSING}"><div style="pointer-events:none">${seg([{ v: 0, label: 'Stereo' }, { v: 1, label: 'Spatial Audio' }], 0, null, 'big')}</div></div>${help(THX_MISSING, 'position:static')}</div>`;
  return `<div class="seg-row">${seg([{ v: 0, label: 'Stereo' }, { v: 1, label: 'Spatial Audio' }], thx, setThx, 'big')}${help(TIPS.THX, 'position:static')}</div>`;
}

// ---------- Game Profiles: per-game THX / EQ while the game's window is in front (backend: gameprof.rs) ----------
const GP_SEL = 'background:#111;border:1px solid #555;color:#fff;padding:4px;font-size:14px';
async function gpRender(el) {
  const d = await invoke('gp_list').catch(() => ({ rules: [], seen: [] }));
  const rules = d.rules;
  const saveRules = () => invoke('gp_save', { rules }).catch(toast);
  const opt = (pairs, v) => pairs.map(([k, t]) => `<option value="${esc(k)}" ${String(k) === String(v) ? 'selected' : ''}>${esc(t)}</option>`).join('');
  const thxOpts = [['', 'Keep'], ['0', 'Stereo'], ['1', THX_OK ? 'Spatial Audio' : 'Spatial Audio (not installed)']];
  const eqOpts = [['', 'Keep EQ']].concat(EQ_PRESETS.map((p, i) => [String(i), p.name]));
  const v = x => (x === null || x === undefined ? '' : String(x === true ? 1 : x === false ? 0 : x));
  el.innerHTML = `
    <div style="display:flex;justify-content:space-between;color:#ccc"><span>Games &amp; Programs</span><span>Audio Output</span></div>
    ${rules.map((r, i) => `<div class="gprow" data-i="${i}" style="display:flex;gap:8px;align-items:center;margin-top:10px">
        <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="${esc(r.key)}">${esc(r.name || r.key)}</span>
        <select class="gpthx" style="${GP_SEL}">${opt(thxOpts, v(r.thx))}</select>
        <select class="gpeq" style="${GP_SEL}">${opt(eqOpts, v(r.eq))}</select>
        <span class="gpdel" title="Remove" style="cursor:pointer;color:#888;padding:0 4px">✕</span></div>`).join('')}
    <div class="gpadd" style="margin-top:16px;font-size:17px;cursor:pointer">＋ Add Games &amp; Programs</div>
    <div class="gppick" style="display:none;margin-top:10px">
      <p class="muted" style="margin:0 0 6px">Recently in front (start the game, then come back here):</p>
      <select class="gpseen" style="${GP_SEL};width:100%">${d.seen.length ? d.seen.map(x => `<option value="${esc(x.key)}" data-n="${esc(x.name)}">${esc(x.name)} (${esc(x.key)})</option>`).join('') : '<option value="">Nothing seen yet</option>'}</select>
      <p class="muted" style="margin:10px 0 6px">Or type it: Steam/Proton <code>steam_app_ID</code>, a Windows <code>Game.exe</code>, or a program name.</p>
      <input class="gpkey" placeholder="e.g. steam_app_251570 or 7DaysToDie.exe" style="width:100%;box-sizing:border-box;background:#111;border:1px solid #555;color:#fff;padding:6px">
      <div style="margin-top:10px;display:flex;gap:10px"><span class="flatbtn gpok">Add</span><span class="flatbtn gpcancel">Cancel</span></div>
    </div>`;
  el.querySelectorAll('.gprow').forEach(row => {
    const r = rules[+row.dataset.i];
    row.querySelector('.gpthx').onchange = e => { r.thx = e.target.value === '' ? null : e.target.value === '1'; saveRules(); };
    row.querySelector('.gpeq').onchange = e => { r.eq = e.target.value === '' ? null : +e.target.value; saveRules(); };
    row.querySelector('.gpdel').onclick = async () => { rules.splice(+row.dataset.i, 1); await saveRules(); gpRender(el); };
  });
  const pick = el.querySelector('.gppick');
  el.querySelector('.gpadd').onclick = () => { pick.style.display = ''; };
  el.querySelector('.gpcancel').onclick = () => { pick.style.display = 'none'; };
  el.querySelector('.gpok').onclick = async () => {
    const typed = el.querySelector('.gpkey').value.trim();
    const s = el.querySelector('.gpseen');
    const key = typed || s.value;
    if (!key) return toast('Pick a game or type its name');
    if (rules.some(r => r.key.toLowerCase() === key.toLowerCase())) return toast('Already in the list');
    const name = typed ? typed : (s.selectedOptions[0] && s.selectedOptions[0].dataset.n) || key;
    rules.push({ key, name, thx: THX_OK ? true : null, eq: null });
    await saveRules();
    gpRender(el);
  };
}

PAGES.headset['SOUND'] = async d => {
  await thxCheck(); const thx = await getv('v3pro_thx_spatial_audio', '0');
  let eq = +(await getv('v3pro_headphone_eq', '0')); if (!(eq >= 0 && eq < 5)) eq = 0;
  const gc = +(await getv('game_chat_balance', '10'));
  const vol = await audioGet('game');
  const volOn = mem('vol_section_on', true) !== false;
  const volWrap = 'volwrap' + (++uid);
  // Custom curves per slot live in our store; the driver takes "<slot> b0..b9" (custom) or "<slot>" (factory).
  const custom = () => mem('eq_custom', {});
  const curve = i => custom()[i] || EQ_PRESETS[i].bands;
  const eqWrite = i => wr('v3pro_headphone_eq', custom()[i] ? `${i} ${custom()[i].join(' ')}` : i);
  const drawChart = i => {
    binders = [];
    document.getElementById('eqchart').innerHTML = eqChart(curve(i), { onChange: b => { remember('eq_custom', { ...custom(), [i]: b }); eqWrite(i); resetBtn(i); } });
    binders.forEach(f => f());
    resetBtn(i);
    const g = document.getElementById('eqgroups'); if (g) g.innerHTML = bandGroups(i);
  };
  const resetBtn = i => { const r = document.getElementById('eqreset')?.parentElement; if (r) r.style.opacity = custom()[i] ? 1 : .4; };
  const tilesId = bind(el => el.querySelectorAll('.tile').forEach((t, i) => t.onclick = () => {
    el.querySelectorAll('.tile').forEach(x => x.classList.remove('active')); t.classList.add('active');
    eq = i; eqWrite(i); drawChart(i);
  }));
  const resetId = bind(el => el.onclick = () => { const c = { ...custom() }; delete c[eq]; remember('eq_custom', c); eqWrite(eq); drawChart(eq); });
  binders.push(() => drawChart(eq));
  return `${headsetArt([['ⓘ View SmartSwitch Shortcuts', 170, true], ['Game/Chat Balance', 250]])}
    ${thxSeg(thx)}
    <div class="card" style="margin-bottom:24px">${help(TIPS['Audio Equalizer'])}<h3>Audio Equalizer</h3>
      <p>EQ presets are stored on your headset for quick access.</p>
      <div style="display:flex;justify-content:flex-end;margin-bottom:12px"><span class="flatbtn ${NA}">✎ Edit EQ List</span></div>
      <div class="tiles" id="${tilesId}">${EQ_PRESETS.map((p, i) => `<div class="tile ${i === eq ? 'active' : ''}"><div class="top"><div style="display:grid;justify-items:center;gap:6px">${p.icon}${p.game ? `<span style="font-size:12px;text-transform:uppercase;color:#bbb">${p.game}</span>` : ''}</div></div><div class="name" style="position:relative">${p.name}${p.badge ? `<span style="position:absolute;right:0;top:0;bottom:0;width:30px;display:grid;place-items:center;background:#1b1b1b">${p.badge}</span>` : ''}</div></div>`).join('')}</div>
      <div id="eqchart" style="margin-top:20px"></div>
      <div class="bands" id="eqgroups">${bandGroups(eq)}</div>
      <div style="text-align:center;margin-top:20px"><span class="flatbtn" id="${resetId}"><span id="eqreset" style="display:contents">⟲ Reset Current EQ</span></span></div>
      ${naNote('Drag the dots to customise the selected preset (−6..+6 dB); Reset restores its factory curve. Needs the patched driver (v3pro-eq-bands).')}</div>
    <div class="grid2">
      ${card('Game Profiles', `<p>Your preferred Spatial Audio or audio equalizer will load automatically when a game is in the foreground.</p>
        <div id="${bind(el => gpRender(el))}"></div>`)}
      <div class="col">
        <div class="card">${help(TIPS['Volume'])}<h3>Volume ${toggle(volOn, on => {
            remember('vol_section_on', on);
            const w = document.getElementById(volWrap); if (w) w.classList.toggle('locked', !on);
          })}</h3>
          <div id="${volWrap}" class="${volOn ? '' : 'locked'}">
          ${vol == null ? '<p class="muted">Headset audio output not found in PipeWire.</p>' : slider({ min: 0, max: 100, val: vol, ends: ['0', '100'], onInput: v => (lastWrite = Date.now(), invoke('audio_set', { kind: 'game', needle: audioNeedle(), value: v }).catch(toast)) })}
          <a href="#" style="color:#ddd;font-size:17px;display:inline-block;margin:10px 0 18px" id="${bind(el => el.onclick = e => { e.preventDefault(); invoke('open_settings', { which: 'mixer' }).catch(() => invoke('open_settings', { which: 'sound' }).catch(toast)); })}">Volume Mixer ↗</a>
          <div class="label">Game/Chat Balance</div>
          ${has('game_chat_balance') ? slider({ min: 0, max: 20, val: gc, nobubble: true, ends: ['🎧 MORE CHAT', 'MORE GAME 🎮', '|'], onInput: v => wr('game_chat_balance', v) }) : '<p class="muted">Not supported.</p>'}
          </div>
        </div>
        ${card('Sound Properties', `<a href="#" style="color:#ddd;font-size:17px" id="${bind(el => el.onclick = e => { e.preventDefault(); invoke('open_settings', { which: 'sound' }).catch(toast); })}">Open System Sound Settings</a>`)}
      </div></div>`;
};

// Audio / Mic Enhancement (Synapse's THX software DSP, rebuilt as PipeWire filter-chains by the backend, enhance.rs).
// One mode per side at a time, like Synapse. Slider/mode changes are live; the toggle reroutes audio.
const ENH = {
  audio: [
    { v: 'normalization', label: 'Sound Normalization', d: 'Amplifies quiet sounds such as footsteps and soft dialogues.' },
    { v: 'bass', label: 'Bass Boost', d: 'Boosts the low end for deeper, punchier sound.' },
    { v: 'clarity', label: 'Voice Clarity', d: 'Brings voices and dialogue forward by trimming boomy bass and lifting the speech range.' },
  ],
  mic: [
    { v: 'normalization', label: 'Volume Normalization', d: 'Volume Normalization ensures consistent output levels by reducing sudden loud noises, and increasing soft audio.' },
    { v: 'clarity', label: 'Vocal Clarity', d: 'Cuts low rumble and lifts the voice range so you are easier to understand.' },
    { v: 'noise', label: 'Mic Noise Cancellation', d: 'Turns quiet background sound down when you are not talking. Higher levels remove more, but can also cut quiet speech.' },
    { v: 'gate', label: 'Voice Gate', d: 'Control the threshold of microphone input. A lower threshold will allow more sound to be registered.' },
  ],
};
const ENH_DEF = m => (m === 'noise' ? 60 : m === 'gate' ? 2 : 55);
async function enhCard(side, title) {
  const all = await invoke('enh_get').catch(() => null);
  const st = (all && all[side]) || { on: false, mode: '', levels: {} };
  const modes = ENH[side];
  let mode = st.mode || modes[0].v;
  const lvl = m => st.levels?.[m] ?? ENH_DEF(m);
  const send = async (on, m, level) => {
    if (on && !mem('enh_loaded')) toast('Loading the enhancement filter: PipeWire restarts once, audio may blip.', true);
    try {
      const r = await invoke('enh_set', { side, on, mode: m, level, needle: audioNeedle() });
      if (on) remember('enh_loaded', true);
      return r;
    } catch (e) { toast(e); render(); }
  };
  const info = modes.find(m => m.v === mode) || modes[0];
  const ctl = mode === 'gate'
    ? `<div style="margin-top:22px">${seg([{ v: 1, label: 'Low' }, { v: 2, label: 'Medium' }, { v: 3, label: 'High' }], lvl('gate'), v => send(st.on, 'gate', +v))}</div>`
    : slider({ min: 0, max: 100, val: lvl(mode), ends: ['LOW', 'HIGH'], dim: !st.on, delay: 120, onInput: v => send(st.on, mode, v) });
  // like Synapse: while the switch is off the tabs and slider are locked
  const wrap = bind(() => {});
  return card(title, `<div id="${wrap}" class="${st.on ? '' : 'locked'}">${btns(modes.map(m => ({ v: m.v, label: m.label })), mode, async v => { await send(st.on, v, lvl(v)); render(); }, 'wide')}
      <p class="muted" style="margin-top:24px">${info.d}</p>${ctl}</div>`,
    { toggle: toggle(st.on, async on => {
      const w = document.getElementById(wrap); if (w) w.classList.toggle('locked', !on);
      await send(on, mode, lvl(mode)); render();
    }) });
}

PAGES.headset['ENHANCEMENT'] = async () => {
  const ull = await getv('v3pro_ultra_low_latency', '0');
  const [ancMode, ancLvl] = (await getv('v3pro_anc', '0 4')).split(/\s+/).map(Number);
  const ancSub = ancMode ? ancMode : +mem('anc_sub', 1);
  let st = { mode: ancMode, sub: ancSub, lvl: ancLvl || 4 };
  const ancWrite = () => { remember('anc_sub', st.sub); wr('v3pro_anc', `${st.mode} ${st.lvl}`); };
  const mix = await getv('in_call_audio_mix', '0');
  const audioEnh = await enhCard('audio', 'Audio Enhancement');
  return `<div class="grid2"><div class="col">
    ${card('Ultra-Low Latency', `<p>Experience high-speed wireless audio while connected via the HyperSpeed 2.4GHz dongle.</p>
      ${seg([{ v: 1, label: 'Always On' }, { v: 'smart', label: 'Smart Switcher', na: true }, { v: 0, label: 'Always Off' }], ull === '1' ? 1 : 0, v => wr('v3pro_ultra_low_latency', v))}
      <p style="margin-top:22px">Ultra-Low Latency offers the fastest response time, as low as ~10ms.</p><p>When disabled, enjoy gaming grade latency, extended range, and longer battery life.</p>
      ${naNote('Note: ULL on has been linked to audio dropouts on this headset.')}`, { na: !has('v3pro_ultra_low_latency') })}
    ${card('Active Noise Cancellation', `
      ${seg([{ v: 1, label: 'ANC' }, { v: 2, label: 'Ambient' }], ancSub, v => { st.sub = +v; if (st.mode) { st.mode = +v; ancWrite(); } else remember('anc_sub', st.sub); })}
      <div style="margin-top:22px">${btns([1, 2, 3, 4].map(n => ({ v: n, label: n })), st.lvl, v => { st.lvl = +v; if (st.mode) ancWrite(); }, 'wide')}</div>`,
    { na: !has('v3pro_anc'), toggle: toggle(!!ancMode, on => { st.mode = on ? st.sub : 0; ancWrite(); }) })}
  </div><div class="col">
    ${card('In-Call Audio Mix', `<p>Select what happens for incoming calls from your mobile device via Bluetooth while connected to 2.4GHz mode.</p>
      ${radios([{ v: 0, t: 'Combine 2.4GHz and Bluetooth' }, { v: 1, t: 'Lower 2.4GHz volume' }, { v: 2, t: 'Mute 2.4GHz volume' }], mix, v => wr('in_call_audio_mix', v))}
      <p style="margin-top:16px">Note: This feature is only available when simultaneously connected to both Bluetooth and 2.4GHz modes.</p>`, { na: !has('in_call_audio_mix') })}
    ${audioEnh}
  </div></div>`;
};

// Synapse's button order + hover names (Joe's Synapse screenshots 2026-11-03): Default, Esports, Broadcast, Flat.
// v = driver preset slot (0x20 + v). Slot 1 is what the driver/Lynapse call "MicBoost"; Synapse labels it "Flat".
const MIC_BUTTONS = [{ v: 0, ic: ART.sliders }, { v: 3, ic: ART.headph }, { v: 2, ic: ART.mega }, { v: 1, ic: ART.sliders }];
// Factory mic EQ curves per preset slot (= headset readback; Default, Flat, Broadcast, Esports).
const MIC_FACTORY = [[-5, -4, -4, -3, -2, 1, 2, 3, 3, 3], [-3, -3, -1, -1, 0, 0, 3, 4, 2, 1], [-4, 1, 2, 0, -1, 0, 2, 3, 2, -1], [2, 2, 3, 4, 4, 4, 4, 3, 2, 2]];
const MIC_NAMES = ['Default', 'Flat', 'Broadcast', 'Esports'];
PAGES.headset['MIC'] = async () => {
  const micVol = await audioGet('mic');
  const prompts = await getv('audio_prompts', '0');
  const preset = +(await getv('mic_eq_preset', '0'));
  let bands = mem('mic_eq', null);
  if (!bands) { // first run: take what Lynapse last wrote, else the preset's factory curve
    const ly = await invoke('lynapse_config').catch(() => null);
    const lv = ly?.state?.['0577']?.mic_eq;
    bands = lv ? lv.split(/\s+/).map(Number) : MIC_FACTORY[preset] || MIC_FACTORY[0];
  }
  if (typeof bands === 'string') bands = bands.split(/\s+/).map(Number);
  const side = await getv('v3pro_sidetone', '0');
  const micEnh = await enhCard('mic', 'Mic Enhancement');
  const setBands = b => { bands = b; remember('mic_eq', b); wr('mic_eq', b.join(' ')); };
  return `<div class="grid2"><div class="col">
    ${card('Microphone', `${micVol == null ? '<p class="muted">Headset microphone not found in PipeWire.</p>' : slider({ min: 0, max: 100, val: micVol, ends: ['0', '100'], onInput: v => (lastWrite = Date.now(), invoke('audio_set', { kind: 'mic', needle: audioNeedle(), value: v }).catch(toast)) })}
      <div style="display:flex;gap:14px;align-items:center;margin-top:22px" class="${NA}"><span class="flatbtn" style="background:none;border:1px solid #666">🎙 Mic Preview</span>Activate to test your mic audio settings</div>`)}
    ${micEnh}
    ${card('Audio Prompts', `<p>Receive sound notifications for optional events.</p>${check('Mic Mute / Unmute', prompts === '1', on => wr('audio_prompts', on ? 1 : 0))}`, { na: !has('audio_prompts') })}
  </div><div class="col">
    <div class="card ${has('mic_eq') ? '' : NA}">${help(TIPS['Mic Equalizer'])}<h3>Mic Equalizer <span style="margin-left:auto;font-size:14px;color:#aaa;cursor:pointer;text-decoration:underline" id="${bind(el => el.onclick = () => { setBands(MIC_FACTORY[+mem('mic_eq_preset', preset)] || MIC_FACTORY[0]); render(); })}">⟲ RESET</span></h3>
      ${btns(MIC_BUTTONS.map(b => ({ v: b.v, label: `<span data-tip="${MIC_NAMES[b.v]}" data-tip-pos="below" style="display:grid;place-items:center;width:100%;height:100%">${b.ic}</span>` })), preset, async v => { await wr('mic_eq_preset', v); setBands(MIC_FACTORY[v].slice()); render(); }, 'wide')}
      <div style="margin-top:20px">${eqChart(bands, { bars: true, onChange: setBands })}</div>
      ${naNote('Click a preset for its factory curve, then drag dots to fine-tune (−6..+6 dB). The headset cannot report its mic EQ, so this shows the last values written.')}</div>
    ${card('Mic Monitoring (Side Tone)', `<p>Listen to your microphone directly through your headset without any enhancements.</p>
      ${slider({ min: 0, max: 15, val: side, ends: ['0', '15'], onInput: v => wr('v3pro_sidetone', v) })}`, { na: !has('v3pro_sidetone') })}
  </div></div>`;
};

PAGES.headset['POWER'] = async () => {
  const led = await getv('indicator_led', '0');
  const ps = +(await getv('v3pro_power_save', '30'));
  let last = ps || +mem('power_save_last', 30);
  return `<div class="grid2">
    ${card('Indicator LED', `<div style="display:flex;gap:30px"><div style="flex:1"><p>Select what the indicator LED on the dongle will display:</p>
      ${radios([{ v: 0, t: 'CONNECTION STATUS', s: 'Indicates connection status to the dongle: Green (2.4GHz), White (Simultaneous), or Off (not connected).' }, { v: 1, t: 'BATTERY STATUS', s: 'Gradually changes from Green (100%), Yellow (50%), and Red (8%) depending on the current battery level of your device.' }, { v: 2, t: 'BATTERY WARNING ONLY', s: 'Remains off at all times and only displays blinking red when the device needs charging.' }], led, v => wr('indicator_led', v))}</div>${ART.dongle}</div>`, { na: !has('indicator_led') })}
    ${card('Wireless Power Saving', `<p>Device will turn off after (mins) of inactivity when running on battery.</p>
      ${btns([15, 30, 45, 60].map(m => ({ v: m, label: m })), last, v => { last = +v; remember('power_save_last', last); wr('v3pro_power_save', v); })}`,
    { na: !has('v3pro_power_save'), toggle: toggle(ps > 0, on => wr('v3pro_power_save', on ? last : 0)) })}
  </div>`;
};

// Demo: original 7.1 clip (tools/make_demo.py). Stereo plays to the headset; THX moves the
// same stream live into a surround engine so you can A/B while it plays.
const DEMO_ORDER = [['Front Left', -30], ['Front Center', 0], ['Front Right', 30], ['Side Right', 90], ['Rear Right', 145], ['Rear Left', -145], ['Side Left', -90]];
const demo = { playing: false, t0: 0, dur: 25.9, timer: null, engines: [], game: null };
function demoAz(t) {
  if (t < 7 * 2.2) { const [n, a] = DEMO_ORDER[Math.floor(t / 2.2)]; return { az: a, label: 'Footsteps · ' + n }; }
  return { az: ((t - 15.4) / 10) * 720, label: 'Drone circling you' };
}
function demoSink() {
  const thx = mem('thx_on', '0') === '1';
  if (!thx || !demo.engines.length) return demo.game;
  return (demo.engines.find(e => e.id === mem('demo_engine', demo.engines[0].id)) || demo.engines[0]).sink;
}
function demoRoute() { const s = demoSink(); if (s) invoke('demo_move', { sink: s }).catch(toast); }
function demoStop() { clearInterval(demo.timer); demo.playing = false; invoke('demo_stop'); }
function demoTick() {
  const t = (performance.now() - demo.t0) / 1000;
  const dot = document.getElementById('demodot'), lbl = document.getElementById('demolbl'), btn = demo.btn;
  if (!dot) return demoStop();
  if (t >= demo.dur) { demoStop(); dot.style.display = 'none'; lbl.textContent = 'Press play, then switch Stereo / Spatial Audio while it plays.'; if (btn) btn.innerHTML = '▶'; return; }
  const { az, label } = demoAz(t), r = Math.PI * (az - 90) / 180;
  dot.style.display = ''; dot.setAttribute('cx', 200 + 150 * Math.cos(r)); dot.setAttribute('cy', 200 + 150 * Math.sin(r));
  lbl.textContent = label;
  document.getElementById('demobar').style.width = (100 * t / demo.dur) + '%';
}
PAGES.headset['DEMO'] = async () => {
  await thxCheck(); const thx = await getv('v3pro_thx_spatial_audio', mem('thx_on', '0'));
  demo.engines = await invoke('thx_engines').catch(() => []);
  demo.game = await invoke('game_sink', { needle: audioNeedle() }).catch(() => null);
  const eng = mem('demo_engine', demo.engines[0]?.id);
  const spk = DEMO_ORDER.map(([n, a]) => { const r = Math.PI * (a - 90) / 180; return `<circle cx="${200 + 150 * Math.cos(r)}" cy="${200 + 150 * Math.sin(r)}" r="9" fill="#2f8a22"><title>${n}</title></circle>`; }).join('');
  const btnId = bind(el => (demo.btn = el).onclick = async () => {
    if (demo.playing) { demoStop(); el.innerHTML = '▶'; return; }
    const sink = demoSink(); if (!sink) return toast('Headset audio output not found');
    try { demo.dur = await invoke('demo_play', { sink }); } catch (e) { return toast(e); }
    demo.playing = true; demo.t0 = performance.now(); el.innerHTML = '■';
    clearInterval(demo.timer); demo.timer = setInterval(demoTick, 50);
  });
  return `${thxSeg(thx)}<p style="text-align:center;font-size:17px">Experience the difference.</p>
    <div style="background:#000;border-radius:6px;padding:30px;display:grid;grid-template-columns:1fr 420px 1fr;align-items:center;gap:20px">
      <div class="${demo.engines.length ? '' : NA}"><div class="label">Spatial engine</div>
        ${demo.engines.length ? radios(demo.engines.map(e => ({ v: e.id, t: e.label, s: e.id === 'lynapse' ? 'What games get when Spatial Audio is on: stereo widening from the HRIR file.' : 'Places each 7.1 channel around you (razer_fix HRIR).' })), eng, v => { remember('demo_engine', v); if (demo.playing) demoRoute(); }) : `<p title="${THX_MISSING}">No THX filter installed.</p>`}</div>
      <div style="position:relative"><svg viewBox="0 0 400 400" width="400"><circle cx="200" cy="200" r="170" fill="#0b0b0b" stroke="#222" stroke-width="14"/><circle cx="200" cy="200" r="110" fill="none" stroke="#1a1a1a"/>${spk}
        <circle id="demodot" r="16" fill="#44d62c" style="display:none;filter:drop-shadow(0 0 10px #44d62c)"/><ellipse cx="200" cy="205" rx="30" ry="20" fill="#ddd"/><text x="200" y="160" fill="#666" font-size="14" text-anchor="middle">FRONT</text></svg>
        <button id="${btnId}" class="iconbtn" style="position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);width:64px;height:64px;border:3px solid #fff;border-radius:50%;font-size:26px;background:rgba(0,0,0,.5)">▶</button></div>
      <div><div id="demolbl" style="font-size:18px">Press play, then switch Stereo / Spatial Audio while it plays.</div>
        <div style="height:4px;background:#222;margin-top:14px"><div id="demobar" style="height:4px;width:0;background:var(--green)"></div></div>
        <p class="muted" style="margin-top:14px;font-size:13px">Original 26-second test clip: footsteps from each speaker in turn, then a drone circling twice. Stereo folds the rear speakers into left/right, so front and back sound the same.</p></div>
    </div>`;
};

// ---------------- settings modal ----------------
async function openSettings() {
  const auto = await invoke('autostart_get');
  const osdScreen = await invoke('osd_get').catch(() => 'tray');
  let osdPick = osdScreen;
  const screens = await invoke('osd_screens').catch(() => []);
  const where = g => { const m = /(-?\d+),(-?\d+)/.exec(g || ''); return m ? `at ${m[1]},${m[2]}` : ''; };
  const osdOpts = [['tray', 'Near the tray (KDE notification)'], ...screens.map(s => [s.name, `Screen ${s.id}: ${s.name} ${where(s.geometry)}`]), ['off', 'Off']];
  binders = [];
  $('#modal').innerHTML = `<div class="card">${'<h3>Settings</h3>'}
    <div class="row">Start Lynapse 2.0 at login (minimised to tray) ${toggle(auto, on => invoke('autostart_set', { on }).catch(toast))}</div>
    <div class="row">Scroll-wheel pop-up <select id="${bind(el => el.onchange = () => { osdPick = el.value; invoke('osd_preview', { screen: el.value }).catch(toast); })}">${osdOpts.map(([v, t]) => `<option value="${esc(v)}" ${v === osdScreen ? 'selected' : ''}>${esc(t)}</option>`).join('')}</select></div>
    <p class="muted" style="font-size:13px;margin-top:-6px">Currently: <b style="color:#ddd">${esc((osdOpts.find(o => o[0] === osdScreen) || ['', osdScreen])[1])}</b>. Picking another shows a sample there; Save &amp; Close keeps it.</p>
    <div class="row ${NA}">Check for updates ${toggle(false)}</div>
    <div class="row ${NA}">Theme ${seg([{ v: 'd', label: 'Dark' }, { v: 'l', label: 'Light' }], 'd')}</div>
    <p class="muted" style="font-size:13px">Closing the window keeps Lynapse 2.0 in the tray. Use the tray menu → Quit to exit.</p>
    <div style="text-align:right;display:flex;gap:12px;justify-content:flex-end">
      <span class="flatbtn" id="${bind(el => el.onclick = () => { invoke('osd_preview', { screen: null }); $('#modal').style.display = 'none'; })}">Cancel</span>
      <button class="greenbtn" id="${bind(el => el.onclick = async () => {
        if (osdPick !== osdScreen) await invoke('osd_set', { screen: osdPick }).catch(toast);
        await invoke('osd_preview', { screen: null });
        $('#modal').style.display = 'none';
      })}">Save &amp; Close</button></div></div>`;
  binders.forEach(b => b());
  $('#modal').style.display = 'grid';
}

// ---------------- not-added-yet guard + tooltip ----------------
for (const ev of ['click', 'mousedown', 'pointerdown', 'input', 'change', 'keydown', 'dblclick']) {
  document.addEventListener(ev, e => { if (e.target.closest && e.target.closest('.na')) { e.preventDefault(); e.stopPropagation(); } }, true);
}
document.addEventListener('mousemove', e => {
  const tip = $('#natip');
  if (e.target.closest && e.target.closest('.na')) {
    tip.style.display = 'block';
    tip.style.left = Math.min(e.clientX + 14, innerWidth - tip.offsetWidth - 6) + 'px';
    tip.style.top = (e.clientY + 18) + 'px';
  } else tip.style.display = 'none';
});

// ---------------- boot ----------------
$('#back').onclick = () => { const h = back.pop(); if (!h) return; fwd.push({ tab: view.tab, sub: view.sub[view.tab] }); view.tab = h.tab; if (h.sub) view.sub[h.tab] = h.sub; render(); };
$('#fwd').onclick = () => { const h = fwd.pop(); if (!h) return; back.push({ tab: view.tab, sub: view.sub[view.tab] }); view.tab = h.tab; if (h.sub) view.sub[h.tab] = h.sub; render(); };
$('#refresh').onclick = async () => { devices = await invoke('devices'); render(); };
$('#gear').onclick = openSettings;
$('#modal').onclick = e => { if (e.target.id === 'modal') $('#modal').style.display = 'none'; };

(async () => {
  store = await invoke('state_load');
  store._ui ||= {};
  devices = await invoke('devices');
  if (store._ui.last && (store._ui.last === 'macro' || devices.some(d => d.id === store._ui.last))) view.tab = store._ui.last;
  const sp = await invoke('start_page');
  if (sp) { const [t, sub] = sp.split('|'); view.tab = t; if (sub) view.sub[t] = sub; }
  render();
  listen('devices', e => {
    const before = JSON.stringify(devices.map(d => [d.id, d.active, d.attrs.length]));
    devices = e.payload;
    const after = JSON.stringify(devices.map(d => [d.id, d.active, d.attrs.length]));
    if (before !== after) render();      // hotplug / connection change: full re-render
    else { renderTabs(); const d = dev(); if (d && d.battery != null) $('#status').firstElementChild && ($('#status').innerHTML = $('#status').innerHTML.replace(/\d+ %/, d.battery + ' %')); }
  });
  listen('select-device', e => go(e.payload));
  listen('battray-changed', () => { if (view.tab === 'home') render(); });
})();

// ---------- frameless window controls ----------
(() => {
  const w = window.__TAURI__.window.getCurrentWindow();
  $('#wmin').onclick = () => w.minimize();
  $('#wmax').onclick = () => w.toggleMaximize();
  $('#wclose').onclick = () => w.close();
})();

// ---------- About (the ? button) ----------
$('#help').onclick = async () => {
  let ver = '';
  try { ver = await window.__TAURI__.app.getVersion(); } catch (_) {}
  $('#modal').innerHTML = `<div class="card" style="max-width:520px">
    <h3 style="margin-top:0">Lynapse 2.0${ver ? ` <span class="muted" style="font-size:14px">v${esc(ver)}</span>` : ''}</h3>
    <p>Created by <b>ShenaniganJoe</b>.</p>
    <p>Built and tested with <b>BlackShark V3 Pro firmware 1.3.3</b> (and Viper V3 Pro firmware 1.14).
      Newer or older firmware may experience issues.</p>
    <p>Want another device supported? Email <b style="user-select:text">j71rivera@gmail.com</b> with interface screenshots (the device's pages in Synapse)
      and he'll see what he can do.</p>
    <p>If you like Lynapse 2.0, feel free to donate to his Cash App: <b style="user-select:text">$j71rivera</b></p>
    <p class="muted" style="font-size:13px">Not affiliated with Razer. Device control goes through the openrazer driver.</p>
    <div style="text-align:right"><button class="greenbtn" id="aboutok">OK</button></div></div>`;
  $('#modal').style.display = 'grid';
  $('#aboutok').onclick = () => $('#modal').style.display = 'none';
};

// Floating tooltip for anything with data-tip (cards clip overflow, so it lives on <body>).
(() => {
  const tip = document.createElement('div');
  tip.id = 'tip';
  document.body.appendChild(tip);
  document.addEventListener('mouseover', e => {
    const t = e.target.closest && e.target.closest('[data-tip]');
    if (!t) { tip.classList.remove('on'); return; }
    tip.textContent = t.dataset.tip;
    const below = t.dataset.tipPos === 'below';   // Synapse-style name label centred under a button
    tip.classList.toggle('small', below);
    tip.classList.add('on');
    const r = (below && t.closest('.btns > div') || t).getBoundingClientRect(), w = tip.offsetWidth, h = tip.offsetHeight;
    let x = below ? r.left + (r.width - w) / 2 : r.right - w, y = r.bottom + (below ? 6 : 8);
    if (x + w > innerWidth - 8) x = innerWidth - 8 - w;
    if (x < 8) x = 8;
    if (y + h > innerHeight - 8) y = r.top - h - 8;
    tip.style.left = x + 'px'; tip.style.top = y + 'px';
  });
})();
