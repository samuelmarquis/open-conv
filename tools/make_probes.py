#!/usr/bin/env python3
"""Generate testdata/probes/ — small synthetic inputs, each engineered to
answer one question (see testdata/probes/README.md). Deterministic; audio
stays untracked, this script is the source of truth.

Run from repo root: python tools/make_probes.py
"""

import os

import numpy as np
import soundfile as sf

SR = 48000
OUT = os.path.join(os.path.dirname(__file__), "..", "testdata", "probes")
rng = np.random.default_rng(0x5EED)


def fade(x, ms=10.0):
    n = int(SR * ms / 1000.0)
    if n * 2 >= len(x):
        return x
    w = 0.5 - 0.5 * np.cos(np.pi * np.arange(n) / n)
    x[:n] *= w
    x[-n:] *= w[::-1]
    return x


def pink(n):
    """Voss-ish pink noise, peak-normalized."""
    white = rng.standard_normal(n)
    b = [0.0] * 3
    out = np.zeros(n)
    for i, w in enumerate(white):
        b[0] = 0.997 * b[0] + 0.029591 * w
        b[1] = 0.985 * b[1] + 0.032534 * w
        b[2] = 0.950 * b[2] + 0.048056 * w
        out[i] = sum(b) + 0.05 * w
    return out / np.max(np.abs(out))


def write(name, x):
    path = os.path.join(OUT, name)
    sf.write(path, x.astype(np.float32), SR)
    print(f"  {name}: {len(x)/SR:.1f}s peak {20*np.log10(np.max(np.abs(x))+1e-12):.1f} dBFS")


def staircase():
    """Q: do the zones read as distinct rooms as level climbs?
    Pink-noise bursts stepping -54..-6 dBFS through all four zones."""
    levels = [-54, -42, -33, -24, -15, -6]
    gap = np.zeros(int(0.6 * SR))
    parts = []
    for l in levels:
        burst = pink(int(0.5 * SR)) * 10 ** (l / 20.0)
        parts += [fade(burst, 10), gap.copy()]
    write("staircase.wav", np.concatenate(parts))


def sineburst():
    """Q: zipper/waveshape artifacts at zone boundaries?
    1 kHz bursts at 3 dB steps spanning the default zone centers."""
    t = np.arange(int(0.3 * SR)) / SR
    tone = np.sin(2 * np.pi * 1000.0 * t)
    gap = np.zeros(int(0.25 * SR))
    parts = []
    for l in range(-51, -2, 3):
        parts += [fade(tone.copy() * 10 ** (l / 20.0), 5), gap.copy()]
    write("sineburst.wav", np.concatenate(parts))


def impulses():
    """Q: does each level fire its own space cleanly (instant mode), and
    how does envelope mode treat one-sample transients?
    Single-sample impulses at the four default zone centers, then three
    5 ms noise clicks at -12 dBFS."""
    parts = []
    for l in [-48, -30, -18, -6]:
        seg = np.zeros(int(1.8 * SR))
        seg[100] = 10 ** (l / 20.0)
        parts.append(seg)
    for _ in range(3):
        seg = np.zeros(int(1.2 * SR))
        click = rng.standard_normal(int(0.005 * SR)) * 10 ** (-12 / 20.0)
        seg[100 : 100 + len(click)] = fade(click, 1)
        parts.append(seg)
    write("impulses.wav", np.concatenate(parts))


def sweepbed():
    """Q: is a size sweep click-free on sustained material?
    Detuned sine-cluster pad with slow level wobble crossing two zones."""
    n = int(10.0 * SR)
    t = np.arange(n) / SR
    pad = sum(np.sin(2 * np.pi * f * t + p) for f, p in
              [(110.0, 0.1), (110.7, 1.2), (164.8, 2.3), (220.3, 0.7),
               (277.2, 1.9), (329.6, 0.4)])
    pad /= np.max(np.abs(pad))
    level_db = -24.0 + 12.0 * np.sin(2 * np.pi * 0.08 * t)  # -36..-12
    write("sweepbed.wav", fade(pad * 10 ** (level_db / 20.0), 50))


def thumps():
    """Q: the representative 'impulse' for a low-end instrument — a
    1-sample Dirac through a narrow resonator is physically near-silent
    (bounded resonant gain ⇒ tiny impulse response; Q-factor physics),
    so the honest minimal probe is a band-limited sub thump. 80 ms
    Hann-windowed 55 Hz bursts at the four zone centers, then an
    8-step velocity ramp -54..-3 dBFS."""
    def thump(level_db, f=55.0, dur=0.08):
        t = np.arange(int(dur * SR)) / SR
        return np.sin(2 * np.pi * f * t) * np.hanning(len(t)) * 10 ** (level_db / 20)
    parts = []
    for l in [-48, -30, -18, -6]:
        seg = np.zeros(int(1.6 * SR))
        tb = thump(l)
        seg[100 : 100 + len(tb)] = tb
        parts.append(seg)
    for l in np.linspace(-54, -3, 8):
        seg = np.zeros(int(1.0 * SR))
        tb = thump(float(l))
        seg[100 : 100 + len(tb)] = tb
        parts.append(seg)
    write("thumps.wav", np.concatenate(parts))


def thumps_desc():
    """thumps, frontloaded: loudest first (A/B ergonomics — the telling
    content plays immediately when flipping files in a browser).
    8-step velocity ramp descending -3..-54, then zone centers loud→quiet."""
    def thump(level_db, f=55.0, dur=0.08):
        t = np.arange(int(dur * SR)) / SR
        return np.sin(2 * np.pi * f * t) * np.hanning(len(t)) * 10 ** (level_db / 20)
    parts = []
    for l in np.linspace(-3, -54, 8):
        seg = np.zeros(int(1.0 * SR))
        tb = thump(float(l))
        seg[100 : 100 + len(tb)] = tb
        parts.append(seg)
    for l in [-6, -18, -30, -48]:
        seg = np.zeros(int(1.6 * SR))
        tb = thump(l)
        seg[100 : 100 + len(tb)] = tb
        parts.append(seg)
    write("thumps_desc.wav", np.concatenate(parts))


def bursts_desc():
    """Broadband pink-noise bursts, loudest first: the full-spectrum
    sibling of thumps_desc — excites an IR across its whole bandwidth
    (a sine probe only reveals the IR at its own frequency). For
    auditioning tails, damping, and anything spectral."""
    parts = []
    for l in np.linspace(-3, -45, 8):
        seg = np.zeros(int(1.2 * SR))
        b = pink(int(0.08 * SR)) * 10 ** (float(l) / 20)
        seg[100 : 100 + len(b)] = fade(b, 3)
        parts.append(seg)
    write("bursts_desc.wav", np.concatenate(parts))


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    print("probes -> testdata/probes/")
    staircase()
    sineburst()
    impulses()
    sweepbed()
    thumps()
    thumps_desc()
    bursts_desc()
