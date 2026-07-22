#!/usr/bin/env python3
"""exp01 — Branch factorization of dynamic convolution.

Claim under test (docs/research/01-prior-art.md §5.4): Kemp's per-tap
dynamic convolution equals a bank of static waveshaper->convolution
branches, exactly. Also quantifies the difference of Kemp's eq. 3 as
literally printed (per-output-sample interpolation fraction), which the
verification pass flagged as the paper's internal inconsistency.

Expected: factorized == per-tap direct to float64 roundoff (SNR > 250 dB);
the literal-eq.3 variant measurably differs (finite SNR) — i.e. the
consistent per-tap form is the right thing to build.
"""

import numpy as np

from convlab import branch_factorized, kemp_dynamic_conv, snr_db, zone_weights, db

rng = np.random.default_rng(0xC0)

CENTERS = [-48.0, -30.0, -18.0, -6.0]
L = 256
N = 2000

# Four random IRs (decaying so the tail matters but errors don't drown)
irs = [rng.standard_normal(L) * np.exp(-np.arange(L) / 60.0) for _ in range(4)]

# Input that actually crosses all zones: noise under a level staircase
levels_db = np.repeat([-54.0, -36.0, -24.0, -12.0, -3.0], N // 5)
x = rng.standard_normal(len(levels_db)) * 10 ** (levels_db / 20.0)

print("exp01: dynamic-convolution branch factorization")

y_direct = kemp_dynamic_conv(x, irs, CENTERS, per_tap_p=True)
y_branch = branch_factorized(x, irs, CENTERS)
s = snr_db(y_direct, y_branch)
print(f"  per-tap direct vs factorized branches: SNR {s:8.1f} dB "
      f"({'PASS' if s > 250 else 'FAIL'} — expect exact to roundoff)")

y_literal = kemp_dynamic_conv(x, irs, CENTERS, per_tap_p=False)
s2 = snr_db(y_literal, y_branch[: len(y_literal)])
print(f"  Kemp eq.3-as-printed vs factorized:    SNR {s2:8.1f} dB "
      f"(finite — quantifies the paper's p(x(n)) inconsistency)")

# sanity: weights are a partition of unity
W = zone_weights(db(x), CENTERS)
err = np.max(np.abs(W.sum(axis=0) - 1.0))
print(f"  zone-weight partition-of-unity max err: {err:.2e} "
      f"({'PASS' if err < 1e-12 else 'FAIL'})")

assert s > 250 and err < 1e-12
print("exp01: PASS")
