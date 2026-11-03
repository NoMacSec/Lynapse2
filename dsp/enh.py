"""Reference model for razerctl's Synapse/THX-style enhancement chains (playback "Audio Enhancement" + "Mic Enhancement").

One fixed filter-chain graph per side; every Synapse mode/slider value maps to a set of live control values on it
(pw-cli set-param <node> Props '{ params = [ "node:Port" value ... ] }'), so changing mode/level never restarts PipeWire.
Mode None = all stages neutral (pass-through).

Numbers come from measurements of THX on Joe's PC (2026-11-03, audio-enhance/measurements, fitted by Windows-side Claude
and re-checked here in a sandbox PipeWire). Slider points 25/55/100 (mic noise cancellation 25/60/100) were measured;
other slider values are linearly interpolated / extrapolated. The Rust port lives in src-tauri/src/enhance.rs.
"""
import os

DYN = os.path.expanduser("~/.local/lib/ladspa/razer_dyn.so")

AUDIO_MODES = ["normalization", "bass", "clarity"]
MIC_MODES = ["normalization", "clarity", "noise", "gate"]


def interp(L, pts):
    """piecewise-linear through [(x, y), ...] (x = slider/100), extrapolating from the end segments"""
    for i in range(len(pts) - 1):
        (x0, y0), (x1, y1) = pts[i], pts[i + 1]
        if L <= x1 or i == len(pts) - 2:
            return y0 + (y1 - y0) * (L - x0) / (x1 - x0)


def i3(L, a, b, c):
    return interp(L, [(0.25, a), (0.55, b), (1.00, c)])


DYN_NEUTRAL = {"Input gain (dB)": 0, "Expander threshold (dB)": -120, "Expander ratio": 1, "Expander floor (dB)": -200,
               "Mute threshold (dB)": -200, "Compressor threshold (dB)": 0, "Compressor ratio": 1, "Knee (dB)": 0,
               "Attack (ms)": 10, "Release (ms)": 100, "Peak detect": 0, "Detector window (ms)": 10}

# node name -> kind. Order = signal order.
GRAPH = {
    "audio": [("bass", "bq_lowshelf"), ("vc_shelf", "bq_lowshelf"), ("vc_dip", "bq_peaking"), ("vc_pres", "bq_peaking"),
              ("norm", "dyn"), ("lim", "dyn")],
    "mic": [("nc", "dyn"), ("gate", "dyn"), ("vc_low", "bq_lowshelf"), ("vc_mid", "bq_peaking"), ("vc_air", "bq_highshelf"),
            ("norm", "dyn"), ("lim", "dyn")],
}
BQ_DEFAULT = {"bass": 105, "vc_shelf": 169, "vc_dip": 151, "vc_pres": 2535, "vc_low": 334, "vc_mid": 819, "vc_air": 8000}


def neutral(side):
    p = {}
    for name, kind in GRAPH[side]:
        if kind == "dyn":
            p.update({f"{name}:{k}": v for k, v in DYN_NEUTRAL.items()})
        else:
            p.update({f"{name}:Freq": BQ_DEFAULT[name], f"{name}:Q": 0.707, f"{name}:Gain": 0.0})
    return p


def params(side, mode, level):
    """full control set for one Synapse setting (mode None = off).
    Attack/Release are one-pole time constants in dB = THX's measured 90 % settle time / 2.3."""
    p = neutral(side)
    L = level / 100.0
    if mode is None:
        return p

    def dyn(name, **kw):
        names = {"gain": "Input gain (dB)", "exp_t": "Expander threshold (dB)", "exp_r": "Expander ratio",
                 "floor": "Expander floor (dB)", "mute": "Mute threshold (dB)", "comp_t": "Compressor threshold (dB)",
                 "comp_r": "Compressor ratio", "knee": "Knee (dB)", "attack": "Attack (ms)", "release": "Release (ms)",
                 "peak": "Peak detect", "window": "Detector window (ms)"}
        for k, v in kw.items():
            p[f"{name}:{names[k]}"] = v

    def bq(name, f, q, g):
        p[f"{name}:Freq"], p[f"{name}:Q"], p[f"{name}:Gain"] = f, q, g

    if side == "audio":
        if mode == "bass":          # one low shelf, 105 Hz, Q 0.75, +9 dB x slider (fit error < 0.01 dB)
            bq("bass", 105.0, 0.75, 9.0 * L)
        elif mode == "clarity":     # -6 dB low shelf (corner rises with slider) + low-mid dip + 2.5 kHz presence peak
            bq("vc_shelf", i3(L, 123.6, 169.1, 227.4), i3(L, 0.995, 0.964, 0.914), -6.0)
            bq("vc_dip", i3(L, 107.4, 151.1, 217.7), i3(L, 1.187, 1.235, 1.299), i3(L, -3.28, -3.78, -5.01))
            bq("vc_pres", 2535.0, i3(L, 0.954, 1.002, 1.009), i3(L, 1.538, 3.229, 6.142))
        elif mode == "normalization":   # fixed makeup gain, then a soft-knee limiter near -4 dB RMS
            # Compressor fitted to the THX static curve with this plugin's knee (max error 0.03/0.01/0.23 dB), using the
            # stimulus level as THX's input: the Windows loopback read 1 dB low at every level, i.e. after THX.
            dyn("norm", gain=i3(L, 6.5, 9.5, 16.0), comp_t=i3(L, -2.75, -2.75, -5.25), comp_r=i3(L, 100, 50, 6),
                knee=i3(L, 6, 8, 8), attack=1, release=i3(L, 200, 313, 500), window=10)
        dyn("lim", comp_t=-0.1, comp_r=100, attack=0.5, release=80, peak=1, window=20)
    else:
        if mode == "noise":         # broadband downward expander (+ hard mute from ~45 up)
            # refitted 2026-11-03 on the drift-corrected THX static curves (rms error 1.9/1.5/2.8 dB)
            pts = lambda a, b, c: [(0.25, a), (0.60, b), (1.00, c)]   # noqa: E731
            dyn("nc", exp_t=interp(L, pts(-47.5, -36.5, -19.5)), exp_r=interp(L, pts(2.5, 2.8, 3.2)),
                floor=-60 if L < 0.45 else -200, mute=interp(L, pts(-92, -77.7, -61.6)) if L >= 0.45 else -200,
                attack=4, release=interp(L, pts(83, 117, 117)), window=10)
        elif mode == "gate":        # Synapse Low/Medium/High = level 1/2/3
            step = int(level)
            if step <= 1:
                dyn("gate", exp_t=-57.0, exp_r=2.0, floor=-32, attack=4, release=52, window=10)
            elif step == 2:
                dyn("gate", mute=-55.9, exp_t=-34.0, exp_r=2.1, attack=4, release=43, window=10)
            else:
                dyn("gate", mute=-33.5, exp_t=-6.5, exp_r=1.9, attack=4, release=43, window=10)
        elif mode == "clarity":     # pure EQ: big low cut, broad ~1 kHz boost, a little air
            # refitted on drift-corrected third-octave THX data (rms error 0.6/1.6/2.3 dB; the rest is acoustic noise)
            bq("vc_low", i3(L, 308, 337, 184), i3(L, 0.85, 1.55, 1.16), i3(L, -7.35, -11.7, -17.0))
            bq("vc_mid", i3(L, 532, 858, 1340), i3(L, 0.122, 0.253, 0.569), i3(L, 4.82, 8.7, 13.6))
            bq("vc_air", 8000.0, 0.707, i3(L, 2.37, 1.38, 1.82))
        elif mode == "normalization":   # fixed makeup gain, compression once the output passes ~-14 dB RMS
            # Static curve only (fitted, ~1 dB). THX also has a slow (~1.4 s) loudness AGC on top that the acoustic data
            # can't pin down, so speech peaks come out ~1-4 dB louder than with THX at 55-100.
            dyn("norm", gain=i3(L, 7.2, 11.9, 18.9), comp_t=i3(L, -13, -13, -9.5), comp_r=i3(L, 3, 8, 20), knee=i3(L, 0, 0, 11),
                attack=5, release=300, window=10)
        dyn("lim", comp_t=-1.0, comp_r=100, attack=0.5, release=80, peak=1, window=20)
    return p


def _fmt(v):
    return f"{v:.6g}" if isinstance(v, float) else str(v)


def graph(side, p=None, dyn_path=DYN):
    """filter.graph body (nodes/links/inputs/outputs) with initial control values"""
    p = p or neutral(side)
    nodes, links = [], []
    order = GRAPH[side]
    for i, (name, kind) in enumerate(order):
        ctl = " ".join(f'"{k.split(":", 1)[1]}" = {_fmt(v)}' for k, v in p.items() if k.split(":", 1)[0] == name)
        if kind == "dyn":
            nodes.append(f'{{ type = ladspa name = {name} plugin = "{dyn_path}" label = razer_dyn control = {{ {ctl} }} }}')
        else:
            nodes.append(f'{{ type = builtin name = {name} label = {kind} control = {{ {ctl} }} }}')
    port = lambda n, d: (f"{n}:{'Input' if d == 'in' else 'Output'}" if dict(order)[n] == "dyn"   # noqa: E731
                         else f"{n}:{'In' if d == 'in' else 'Out'}")
    for (a, _), (b, _) in zip(order, order[1:]):
        links.append(f'{{ output = "{port(a, "out")}" input = "{port(b, "in")}" }}')
    return (f"nodes = [ {' '.join(nodes)} ] links = [ {' '.join(links)} ] "
            f'inputs = [ "{port(order[0][0], "in")}" ] outputs = [ "{port(order[-1][0], "out")}" ]')


if __name__ == "__main__":
    import sys
    side, mode, level = sys.argv[1], (None if sys.argv[2] == "off" else sys.argv[2]), float(sys.argv[3])
    print(graph(side, params(side, mode, level)))
