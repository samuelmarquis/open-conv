#!/usr/bin/env python3
"""exp02 — Stepwise partition replacement vs dual-convolver reference.

Claim under test (Brandtsegg & Saue, DAFx-17; docs/research §3.4):
replacing IR partitions one-per-frame in load order inside a uniformly
partitioned OLA convolver reproduces the output of two overlapped
time-invariant convolutions (old input stops at n_T, new starts there).
They report 67.2 dB SNR at NP=1024, 1 s speech IRs, 44.1 kHz.

We reproduce the setup shape with synthetic room IRs and check we land in
the same regime (>= 60 dB), plus a shrink-grow case (different IR lengths)
that the engine must also handle.
"""

import numpy as np

from convlab import Upola, dual_convolver_reference, exp_decay_noise, snr_db

SR = 44100
P = 1024

rng = np.random.default_rng(7)

print("exp02: stepwise IR replacement (B&S DAFx-17)")

# --- case 1: equal-length 1 s IRs, mid-signal swap ---------------------
h_a = exp_decay_noise(1.0, SR, t60=0.7, cutoff=6000.0, seed=1)
h_b = exp_decay_noise(1.0, SR, t60=0.9, cutoff=3000.0, seed=2)
n = 5 * SR
x = rng.standard_normal(n) * (0.3 + 0.2 * np.sin(2 * np.pi * np.arange(n) / SR * 0.7))

k_parts = int(np.ceil(len(h_a) / P))
swap_frame = int(2.2 * SR / P)  # partition boundary ~2.2 s
n_t = swap_frame * P

conv = Upola(h_a, P)
out = np.zeros(int(np.ceil(n / P)) * P)
for i, f0 in enumerate(range(0, len(out), P)):
    if i == swap_frame:
        conv.queue(h_b)
    frame = x[f0 : f0 + P]
    if len(frame) < P:
        frame = np.pad(frame, (0, P - len(frame)))
    out[f0 : f0 + P] = conv.process_frame(frame)

ref = dual_convolver_reference(x, h_a, h_b, n_t)
s = snr_db(ref[: len(out)], out)
print(f"  equal-length swap ({k_parts} partitions): SNR {s:6.1f} dB "
      f"({'PASS' if s >= 60 else 'FAIL'} — B&S report 67.2)")

# --- case 2: length change (1.0 s -> 0.5 s), the size-retarget shape ---
h_c = exp_decay_noise(0.5, SR, t60=0.35, cutoff=9000.0, seed=3)
conv2 = Upola(h_a, P)
out2 = np.zeros_like(out)
for i, f0 in enumerate(range(0, len(out2), P)):
    if i == swap_frame:
        conv2.queue(h_c)
    frame = x[f0 : f0 + P]
    if len(frame) < P:
        frame = np.pad(frame, (0, P - len(frame)))
    out2[f0 : f0 + P] = conv2.process_frame(frame)

ref2 = dual_convolver_reference(x, h_a, h_c, n_t)
s2 = snr_db(ref2[: len(out2)], out2)
print(f"  shrinking swap (1.0s -> 0.5s IR):        SNR {s2:6.1f} dB "
      f"({'PASS' if s2 >= 60 else 'FAIL'})")

# --- click check: max abs sample-to-sample step around the transition --
w = out[n_t - SR : n_t + 2 * SR]
step = np.max(np.abs(np.diff(w)))
step_ref = np.max(np.abs(np.diff(ref[n_t - SR : n_t + 2 * SR])))
print(f"  max |Δsample| through transition: test {step:.4f} vs ref {step_ref:.4f} "
      f"({'PASS' if step < 1.5 * step_ref else 'FAIL'} — no added discontinuity)")

assert s >= 60 and s2 >= 60 and step < 1.5 * step_ref
print("exp02: PASS")
