#!/usr/bin/env python3
"""Build Lynapse's free spatial-audio HRIR (14-channel HeSuVi layout, 48 kHz float WAV) from the MIT KEMAR data.

Source data: "HRTF Measurements of a KEMAR Dummy-Head Microphone", Bill Gardner and Keith Martin, MIT Media Lab (1994).
  "This data is Copyright 1994 by the MIT Media Laboratory. It is provided free with no restrictions on use, provided
   the authors are cited when the data is used in any research or commercial application."
  https://sound.media.mit.edu/resources/KEMAR.html  (compact set: 128-tap stereo IRs, 44.1 kHz, int16 big-endian,
  interleaved left/right ear, positive azimuth = listener's right, right side only - left side is the mirror image)

Processing (all original to this script): virtual speakers at FL/FR +-30, FC 0, SL/SR +-90, BL/BR +-135 deg, elevation 0;
diffuse-field equalisation (inverse of the average power response over every measured direction, 1/3-octave smoothed,
limited to +-12 dB, minimum phase); band-limited resampling to 48 kHz; a short synthetic room tail (decorrelated
decaying noise, ~-20 dB) for externalisation; level matched so the near-ear front channel has ~0 dB energy (same as
the HRIR razerctl/Lynapse was tuned with).

HeSuVi 14-channel order (what the PipeWire filter-chain reads):
  0 FL->L  1 FL->R  2 SL->L  3 SL->R  4 BL->L  5 BL->R  6 FC->L  7 FR->R  8 FR->L  9 SR->R  10 SR->L  11 BR->R  12 BR->L  13 FC->R

Usage: make_hrir.py OUT.wav [compact.tar.Z]   (downloads the compact set if no archive is given; needs numpy)
"""
import io, os, struct, subprocess, sys, tarfile, urllib.request
import numpy as np

URL = "https://sound.media.mit.edu/resources/KEMAR/compact.tar.Z"
SR_IN, SR_OUT = 44100, 48000


def load_compact(archive):
    raw = open(archive, "rb").read() if archive else urllib.request.urlopen(URL, timeout=60).read()
    # .Z (LZW) -> tar: python has no LZW decoder, so use the system's uncompress/gzip
    tar = None
    for cmd in (["uncompress", "-c"], ["gzip", "-dc"]):
        try:
            tar = subprocess.run(cmd, input=raw, capture_output=True, check=True).stdout
            break
        except (OSError, subprocess.CalledProcessError):
            continue
    if not tar:
        sys.exit("could not decompress the KEMAR archive (need 'uncompress' or 'gzip')")
    out = {}
    with tarfile.open(fileobj=io.BytesIO(tar)) as t:
        for m in t.getmembers():
            n = os.path.basename(m.name)
            if not (m.isfile() and n.startswith("H") and n.endswith("a.dat")):
                continue
            elev, az = int(n[1:n.index("e")]), int(n[n.index("e") + 1:-5])
            d = np.frombuffer(t.extractfile(m).read(), ">i2").astype(np.float64) / 32768.0
            out[(elev, az)] = d.reshape(-1, 2)   # [:,0] left ear, [:,1] right ear (source on the right side)
    return out


def hrir(data, az):
    """(left ear, right ear) for a source at azimuth az (deg, + = right), elevation 0"""
    a = abs(az)
    lr = data[(0, a)]
    return (lr[:, 0], lr[:, 1]) if az >= 0 else (lr[:, 1], lr[:, 0])   # mirror for the left side


def resample(x, n_out):
    """band-limited FFT resampling of a short IR"""
    X = np.fft.rfft(x)
    Y = np.zeros(n_out // 2 + 1, complex)
    k = min(len(X), len(Y))
    Y[:k] = X[:k]
    return np.fft.irfft(Y, n_out) * n_out / len(x)


def min_phase(mag, n):
    """minimum-phase impulse response (length n) for a magnitude response sampled on rfft bins of size n"""
    logm = np.log(np.maximum(mag, 1e-6))
    cep = np.fft.irfft(logm, n)
    w = np.zeros(n); w[0] = 1; w[1:n // 2] = 2; w[n // 2] = 1
    return np.fft.irfft(np.exp(np.fft.rfft(cep * w, n)), n)


def diffuse_field_eq(data, n=1024):
    f = np.fft.rfftfreq(n, 1 / SR_OUT)
    p = np.zeros(len(f))
    for ir in data.values():
        for ch in (0, 1):
            p += np.abs(np.fft.rfft(resample(ir[:, ch], 140), n)) ** 2
    p /= 2 * len(data)
    sm = np.empty_like(p)
    for i, fc in enumerate(f):              # 1/3-octave smoothing
        lo, hi = fc / 2 ** (1 / 6), fc * 2 ** (1 / 6)
        m = (f >= lo) & (f <= hi)
        sm[i] = p[m].mean() if m.any() else p[i]
    df = 10 * np.log10(sm)
    ref = df[(f > 200) & (f < 1000)].mean()
    inv = np.clip(-(df - ref), -12, 12)
    inv[f < 150] = inv[np.argmin(abs(f - 150))]       # keep the extremes flat
    inv[f > 16000] = inv[np.argmin(abs(f - 16000))]
    return min_phase(10 ** (inv / 20), n)[:512] * np.hanning(1024)[512:]


def room_tail(rng, n, t60=0.12, level_db=-20):
    t = np.arange(n) / SR_OUT
    env = 10 ** (-3 * t / t60)
    x = rng.standard_normal(n) * env
    x = np.convolve(x, np.hanning(9) / np.hanning(9).sum(), "same")   # gentle low-pass: rooms absorb highs
    start = int(0.004 * SR_OUT)                                       # first reflections ~4 ms after direct sound
    x[:start] = 0
    return x / np.sqrt((x ** 2).sum()) * 10 ** (level_db / 20)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "lynapse-kemar.wav"
    data = load_compact(sys.argv[2] if len(sys.argv) > 2 else None)
    eq = diffuse_field_eq(data)
    n_ir = int(round(128 * SR_OUT / SR_IN))
    rng = np.random.default_rng(1994)
    L = 4096
    spk = {"FL": -30, "FR": 30, "FC": 0, "SL": -90, "SR": 90, "BL": -135, "BR": 135}
    ears = {}
    for name, az in spk.items():
        l, r = hrir(data, az)
        for ear, x in (("L", l), ("R", r)):
            y = np.convolve(resample(x, n_ir), eq)
            buf = np.zeros(L)
            off = 40                                              # a little pre-delay, like common HeSuVi files
            buf[off:off + len(y)] = y[:L - off]
            buf += np.roll(room_tail(rng, L), off) * np.sqrt((y ** 2).sum())
            ears[(name, ear)] = buf
    order = [("FL", "L"), ("FL", "R"), ("SL", "L"), ("SL", "R"), ("BL", "L"), ("BL", "R"), ("FC", "L"),
             ("FR", "R"), ("FR", "L"), ("SR", "R"), ("SR", "L"), ("BR", "R"), ("BR", "L"), ("FC", "R")]
    m = np.stack([ears[k] for k in order], axis=1)
    m *= 1 / np.sqrt((m[:, 0] ** 2).sum())                 # near-ear front channel = 0 dB energy
    fade = np.ones(L); fade[-512:] = np.hanning(1024)[512:]
    m = (m * fade[:, None]).astype("<f4")
    d = m.tobytes()
    fmt = struct.pack("<HHIIHH", 3, 14, SR_OUT, SR_OUT * 4 * 14, 4 * 14, 32)
    with open(out, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(d)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(d)) + d)
    e = 10 * np.log10((m.astype(float) ** 2).sum(0))
    print(f"wrote {out}: 14 ch, {L} taps @ {SR_OUT} Hz; channel energy dB:", " ".join(f"{v:+.1f}" for v in e))


if __name__ == "__main__":
    main()
