#!/usr/bin/env python3
"""exp03 — Continuous size sweep as a stream of re-rendered IRs.

The v1 answer to limitation L1: "size" re-renders the IR (resample,
pitch-coupled) on the control path and streams it into the running
convolver via stepwise partition replacement — transitions are the
reverb's own decay, never a click.

Renders two wavs for listening (lab/out/, untracked):
  exp03_stream.wav    sequential re-render + stepwise replacement
  exp03_hardswap.wav  same schedule, instantaneous IR swaps (the control —
                      expect audible clicks/steps)
and prints a discontinuity metric for both.
"""

import os

import numpy as np
import soundfile as sf

from convlab import Upola, exp_decay_noise

SR = 44100
P = 1024
SIZE_FROM, SIZE_TO = 1.0, 1.6
DUR = 8.0

rng = np.random.default_rng(11)


def render_ir(size):
    """Resample-stretch the source room by `size` (linear interp,
    1/sqrt(size) energy compensation) — mirrors the Rust engine."""
    src = SOURCE
    n_out = int(len(src) * size)
    pos = np.arange(n_out) / size
    i0 = np.minimum(pos.astype(int), len(src) - 1)
    i1 = np.minimum(i0 + 1, len(src) - 1)
    f = pos - i0
    return (src[i0] * (1 - f) + src[i1] * f) / np.sqrt(size)


SOURCE = exp_decay_noise(1.5, SR, t60=1.1, cutoff=5000.0, seed=5)

# Sustained pad: detuned sine cluster with slow amplitude wobble
n = int(DUR * SR)
t = np.arange(n) / SR
pad = sum(np.sin(2 * np.pi * f * t + p) for f, p in
          [(110.0, 0.1), (110.7, 1.2), (164.8, 2.3), (220.3, 0.7), (277.2, 1.9)])
pad *= 0.12 * (0.7 + 0.3 * np.sin(2 * np.pi * 0.15 * t))

frames = n // P
sizes = SIZE_FROM + (SIZE_TO - SIZE_FROM) * np.arange(frames) / frames

print("exp03: size sweep — streamed vs hard-swapped IR")

# --- streamed: queue a new render whenever the convolver goes idle -----
conv = Upola(render_ir(sizes[0]), P)
out_s = np.zeros(frames * P)
rendered = sizes[0]
renders = 0
for i in range(frames):
    if conv.pending is None and abs(sizes[i] - rendered) > 0.01:
        conv.queue(render_ir(sizes[i]))
        rendered = sizes[i]
        renders += 1
    out_s[i * P : (i + 1) * P] = conv.process_frame(pad[i * P : (i + 1) * P])

# --- control: hard swap on the same schedule ---------------------------
conv_h = Upola(render_ir(sizes[0]), P)
out_h = np.zeros(frames * P)
rendered = sizes[0]
for i in range(frames):
    if abs(sizes[i] - rendered) > 0.01:
        new = conv_h._partition(render_ir(sizes[i]))
        conv_h.h[:] = 0.0  # instantaneous H swap, signal state kept
        conv_h.h[: len(new)] = new
        conv_h.K = len(new)
        rendered = sizes[i]
    out_h[i * P : (i + 1) * P] = conv_h.process_frame(pad[i * P : (i + 1) * P])


def disc_metric(y):
    """99.9th percentile of |second difference| — clicks poke far above a
    smooth reverb's baseline."""
    return float(np.percentile(np.abs(np.diff(y, 2)), 99.9))


d_s, d_h = disc_metric(out_s), disc_metric(out_h)
print(f"  re-renders during sweep: {renders}")
print(f"  discontinuity metric: streamed {d_s:.6f} vs hard-swap {d_h:.6f} "
      f"({'PASS' if d_s < d_h else 'CHECK'} — streamed should be smoother)")

os.makedirs("out", exist_ok=True)
peak = max(np.max(np.abs(out_s)), np.max(np.abs(out_h)), 1e-9)
sf.write("out/exp03_stream.wav", 0.9 * out_s / peak, SR)
sf.write("out/exp03_hardswap.wav", 0.9 * out_h / peak, SR)
print("  wrote out/exp03_stream.wav, out/exp03_hardswap.wav — listen!")
print("exp03: done")
