#!/usr/bin/env python3
"""Generate razerctl's original 7.1 positional demo clip (no third-party audio).
Part 1: footsteps from each speaker in turn (FL, FC, FR, SR, RR, RL, SL), 2.2 s each.
Part 2: a buzzing drone circles you twice in 10 s, starting front, going clockwise.
The UI animation in app.js (demoAz) uses the same timeline; keep them in sync.
Output: 8ch FLAC, channel order FL FR FC LFE RL RR SL SR, 48 kHz."""
import numpy as np, subprocess, sys
SR = 48000
STEP, CIRCLE = 2.2, 10.0
ORDER = ['FL', 'FC', 'FR', 'SR', 'RR', 'RL', 'SL']
AZ = {'FC': 0, 'FR': 30, 'SR': 90, 'RR': 145, 'RL': 215, 'SL': 270, 'FL': 330}
CH = ['FL', 'FR', 'FC', 'LFE', 'RL', 'RR', 'SL', 'SR']
RING = sorted(AZ, key=AZ.get)
rng = np.random.default_rng(7)

def gains(az):
    az %= 360
    g = dict.fromkeys(CH, 0.0)
    for i, a in enumerate(RING):
        b = RING[(i + 1) % len(RING)]
        lo, hi = AZ[a], AZ[b] if AZ[b] > AZ[a] else AZ[b] + 360
        x = az if az >= lo else az + 360
        if lo <= x <= hi:
            f = (x - lo) / (hi - lo)
            g[a] = np.cos(f * np.pi / 2); g[b] = np.sin(f * np.pi / 2)
            return g
    return g

def footsteps(dur):
    n = int(dur * SR); out = np.zeros(n)
    for k, t0 in enumerate(np.arange(0.1, dur - 0.2, 0.36)):
        i = int(t0 * SR); L = int(0.12 * SR)
        env = np.exp(-np.linspace(0, 9, L))
        thump = np.sin(2 * np.pi * (70 + 15 * (k % 2)) * np.arange(L) / SR) * env
        grit = np.convolve(rng.standard_normal(L), np.ones(6) / 6, 'same') * env * 0.5
        out[i:i + L] += (thump + grit)[: n - i]
    return out * 0.8

total = len(ORDER) * STEP + CIRCLE + 0.5
N = int(total * SR); out = np.zeros((N, 8))
for k, name in enumerate(ORDER):
    s = footsteps(STEP); i = int(k * STEP * SR)
    for c, gv in gains(AZ[name]).items():
        out[i:i + len(s), CH.index(c)] += s * gv
t = np.arange(int(CIRCLE * SR)) / SR
saw = 2 * ((110 * t) % 1) - 1
drone = (0.35 * saw + 0.25 * np.convolve(rng.standard_normal(len(t)), np.ones(4) / 4, 'same')) * (0.6 + 0.4 * np.sin(2 * np.pi * 9 * t))
fade = np.minimum(1, np.minimum(t / 0.3, (CIRCLE - t) / 0.5))
drone *= fade * 0.7
az = (t / CIRCLE) * 720
i0 = int(len(ORDER) * STEP * SR)
for c in CH:
    if c == 'LFE': continue
    gv = np.array([gains(a)[c] for a in az[::480]])          # per 10 ms
    gv = np.interp(np.arange(len(t)), np.arange(0, len(t), 480), gv)
    out[i0:i0 + len(t), CH.index(c)] += drone * gv
out /= max(1e-9, np.abs(out).max()) / 0.8
dst = sys.argv[1]
subprocess.run(['ffmpeg', '-y', '-loglevel', 'error', '-f', 'f32le', '-ar', str(SR), '-ac', '8', '-i', '-',
                '-channel_layout', '7.1', '-c:a', 'flac', dst], input=out.astype('<f4').tobytes(), check=True)
print(dst, f'{total:.1f}s')
