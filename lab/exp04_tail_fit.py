#!/usr/bin/env python3
"""exp04 — Parametric tail, phase 1: can a synthesized tail pass for the
real one? (Research §7-C / PATHS-NOT-TAKEN #10 re-entry.)

Model under test: the classic "reverb tail is filtered noise" STFT model —
keep the IR's first 100 ms verbatim (the identity), replace everything
after with random-phase noise whose per-bin magnitudes follow decay rates
fitted from the original tail. If this passes on a class of IRs, that
class gets continuously-modulatable Decay/Damp/Freeze for free (the knobs
become the fitted coefficients).

Three IR classes, chosen to bracket the answer:
- noise-like (rooms-style): the model's home turf — should pass
- tonal (subdrop-style boom): should fail informatively (noise can't
  ring a pitch → motivates the modal route for tonal content)
- real samples (murky/prism/press): the open question

Outputs (out/batch009_tailfit/, untracked): for each IR, the raw pair
`<name>_ir_{orig,hybrid}.wav` and convolution renders against the thumps
probe `<name>_conv_{orig,hybrid}.wav`. Listen blind; the log's question
is simply "which ones can you tell apart, and does it matter?"
"""

import os

import numpy as np
import soundfile as sf
from scipy.signal import fftconvolve, istft, stft

from convlab import exp_decay_noise

SR = 48000
HEAD_S = 0.100
XFADE_S = 0.030
NPERSEG = 1024
HOP = 256

rng = np.random.default_rng(0xF17)
OUT = "out/batch009_tailfit"


def tonal_boom(seconds=1.0, f0=52.0, f1=36.0, t60=0.8):
    """Subdrop-style gliding boom (the adversarial tonal case)."""
    n = int(seconds * SR)
    t = np.arange(n) / SR
    f = f1 + (f0 - f1) * np.exp(-t / 0.06)
    ph = 2 * np.pi * np.cumsum(f) / SR
    env = 10 ** (-3 * t / t60)
    h = np.sin(ph) * env + 0.3 * np.sin(2 * ph) * env**2
    return h / np.max(np.abs(h))


def fit_and_resynth_tail(tail):
    """Per-bin exponential-decay fit of the tail's STFT magnitudes, then
    random-phase resynthesis. Returns (synth_tail, median_t60_err_frames)."""
    f, t, Z = stft(tail, SR, nperseg=NPERSEG, noverlap=NPERSEG - HOP)
    mag = np.abs(Z) + 1e-12
    n_bins, n_frames = mag.shape
    logm = np.log(mag)
    # robust-ish linear fit per bin over frames above the bin's noise floor
    frames = np.arange(n_frames)
    floor = logm.min(axis=1, keepdims=True) + 1.0
    A = np.zeros(n_bins)
    slope = np.zeros(n_bins)
    for b in range(n_bins):
        use = logm[b] > floor[b]
        if use.sum() < 4:
            A[b], slope[b] = logm[b, 0], -0.5
            continue
        p = np.polyfit(frames[use], logm[b][use], 1)
        slope[b] = min(p[0], -1e-4)  # decays only
        A[b] = p[1]
    # resynthesize: fitted envelopes × random phase
    synth_mag = np.exp(A[:, None] + slope[:, None] * frames[None, :])
    phase = rng.uniform(-np.pi, np.pi, size=synth_mag.shape)
    _, y = istft(
        synth_mag * np.exp(1j * phase), SR, nperseg=NPERSEG, noverlap=NPERSEG - HOP
    )
    y = y[: len(tail)]
    if len(y) < len(tail):
        y = np.pad(y, (0, len(tail) - len(y)))
    # energy-match the first 100 ms of the tail
    n0 = int(0.1 * SR)
    e_orig = np.sqrt(np.sum(tail[:n0] ** 2))
    e_syn = np.sqrt(np.sum(y[:n0] ** 2)) + 1e-12
    y *= e_orig / e_syn
    return y


def hybridize(ir):
    n_head = int(HEAD_S * SR)
    n_x = int(XFADE_S * SR)
    if len(ir) < n_head + 4 * n_x:
        return ir.copy()  # too short to bother
    head = ir[: n_head + n_x]
    tail = ir[n_head:]
    synth = fit_and_resynth_tail(tail)
    out = np.zeros(len(ir))
    out[: n_head + n_x] = head
    w = 0.5 - 0.5 * np.cos(np.pi * np.arange(n_x) / n_x)
    out[n_head : n_head + n_x] = head[n_head:] * (1 - w) + synth[:n_x] * w
    out[n_head + n_x :] = synth[n_x : len(ir) - n_head]
    return out


def mono(path):
    x, sr = sf.read(path)
    if x.ndim > 1:
        x = x.mean(axis=1)
    if sr != SR:  # crude resample for lab purposes
        n = int(len(x) * SR / sr)
        x = np.interp(np.linspace(0, len(x) - 1, n), np.arange(len(x)), x)
    return x


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    # descending variant: loudest first (browser-A/B ergonomics)
    probe = mono("../testdata/probes/thumps_desc.wav")

    irs = {
        "noiseroom": exp_decay_noise(2.5, SR, t60=1.8, cutoff=5000.0, seed=3),
        "boom": tonal_boom(),
        "murky": mono("../testdata/material/irs/ir_murky.wav"),
        "prism": mono("../testdata/material/irs/ir_prism.wav"),
        "press": mono("../testdata/material/irs/ir_press.wav"),
    }
    print("exp04: parametric-tail fit — phase 1 (STFT-noise model)")
    for name, ir in irs.items():
        hyb = hybridize(ir)
        # objective: spectral distance of the tails (post-head)
        n_head = int(HEAD_S * SR)
        fo = np.abs(np.fft.rfft(ir[n_head:]))
        fh = np.abs(np.fft.rfft(hyb[n_head:]))
        n = min(len(fo), len(fh))
        lsd = np.sqrt(
            np.mean((20 * np.log10(fo[:n] + 1e-9) - 20 * np.log10(fh[:n] + 1e-9)) ** 2)
        )
        print(f"  {name:10s} tail log-spectral distance {lsd:6.1f} dB "
              f"({'noise-model friendly' if lsd < 12 else 'tonal/structured — expect audible'})")
        peak = max(np.max(np.abs(ir)), 1e-9)
        sf.write(f"{OUT}/{name}_ir_orig.wav", (0.9 * ir / peak).astype(np.float32), SR)
        sf.write(f"{OUT}/{name}_ir_hybrid.wav", (0.9 * hyb / peak).astype(np.float32), SR)
        for tag, h in [("orig", ir), ("hybrid", hyb)]:
            y = fftconvolve(probe, h)
            y *= 0.9 / max(np.max(np.abs(y)), 1e-9)
            sf.write(f"{OUT}/{name}_conv_{tag}.wav", y.astype(np.float32), SR)
    print(f"wrote A/B pairs to {OUT}/ — the ears gate phase 2.")
