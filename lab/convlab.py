"""Shared prototype library for the open-conv lab.

Pure numpy/scipy reference implementations of the engine's two load-bearing
mechanisms, kept deliberately close to the primary sources:

- Kemp dynamic convolution (AES 106th, 1999), per-tap selector, and its
  factorization into static waveshaper->convolution branches
  (docs/research/01-prior-art.md §5.4).
- Uniformly partitioned overlap-add convolution with stepwise partition
  replacement (Brandtsegg & Saue, DAFx-17; docs/research §3.4).

Float64 throughout — the lab establishes *math*, the Rust engine
establishes *engineering*.
"""

import numpy as np
from scipy.signal import fftconvolve

SILENCE_DB = -160.0


def db(x):
    return 20.0 * np.log10(np.maximum(np.abs(x), 1e-8))


def zone_weights(level_db, centers):
    """Triangular windows in dB space, partition of unity.

    level_db: array (N,); centers: ascending list (Z,).
    Returns (Z, N) weights.
    """
    c = np.asarray(centers, float)
    z = len(c)
    lvl = np.asarray(level_db, float)
    w = np.zeros((z, len(lvl)))
    w[0, lvl <= c[0]] = 1.0
    w[-1, lvl >= c[-1]] = 1.0
    for i in range(z - 1):
        m = (lvl > c[i]) & (lvl < c[i + 1])
        f = (lvl[m] - c[i]) / (c[i + 1] - c[i])
        w[i, m] = 1.0 - f
        w[i + 1, m] = f
    return w


def kemp_dynamic_conv(x, irs, centers, per_tap_p=True):
    """Direct-form dynamic convolution, Kemp eq. 3 generalized to zone
    windows. Selector always per-tap (S(x(n-k))); `per_tap_p` chooses
    whether the interpolation fraction is also per-tap (consistent form)
    or per-output-sample (Kemp's eq. 3 as literally printed — his p(x(n))).
    O(N·L·Z) python loops — keep sizes small.
    """
    x = np.asarray(x, float)
    L = len(irs[0])
    H = np.stack(irs)  # (Z, L)
    W_tap = zone_weights(db(x), centers)  # (Z, N) per-tap weights
    N = len(x)
    y = np.zeros(N + L - 1)
    if per_tap_p:
        # y[n] = sum_k x[n-k] * sum_z w_z(|x[n-k]|) h_z[k]
        # loop over taps of each input sample's contribution instead
        for n in range(N):
            wz = W_tap[:, n]  # weights of sample n (as the delayed sample)
            h_eff = wz @ H  # (L,)
            y[n : n + L] += x[n] * h_eff
    else:
        # Kemp eq.3 as literally printed: the *selector* S is per-tap
        # (bracket chosen from |x(n-k)|) but the interpolation fraction p
        # is evaluated at output time n (his p(x(n)) — the paper's
        # internal inconsistency, verifier-confirmed). Translated to zone
        # windows: bracket from the tap's level, fractional position from
        # the output-time level clamped into that bracket.
        c = np.asarray(centers, float)
        lvl = db(x)
        for n in range(N):
            ldb_n = lvl[n]
            for k in range(min(L, n + 1)):
                ldb_k = lvl[n - k]
                if ldb_k <= c[0]:
                    h_eff = H[0, k]
                elif ldb_k >= c[-1]:
                    h_eff = H[-1, k]
                else:
                    i = int(np.searchsorted(c, ldb_k) - 1)
                    f = (ldb_n - c[i]) / (c[i + 1] - c[i])  # p(x(n))!
                    f = min(max(f, 0.0), 1.0)
                    h_eff = (1.0 - f) * H[i, k] + f * H[i + 1, k]
                y[n] += x[n - k] * h_eff
        y = y[:N]  # tail beyond N not computed in this slow form
    return y


def branch_factorized(x, irs, centers):
    """The engine's form: y = sum_z conv(x * w_z(|x|), h_z)."""
    x = np.asarray(x, float)
    W = zone_weights(db(x), centers)
    out = None
    for z, h in enumerate(irs):
        y = fftconvolve(x * W[z], h)
        out = y if out is None else out + y
    return out


class Upola:
    """Uniform partitioned overlap-add convolver with stepwise IR
    replacement (B&S DAFx-17: one partition per frame, load order).

    Mirrors the Rust engine's invariants: the input-spectra ring and the
    active H bank are allocated at `max_parts` once and NEVER resized —
    resizing mid-stream corrupts ring-modulo history (found the hard way).
    H rows beyond K are always zero.
    """

    def __init__(self, ir, part, max_parts=256):
        self.P = part
        self.max_parts = max_parts
        new = self._partition(ir)
        assert len(new) <= max_parts
        self.h = np.zeros((max_parts, part + 1), complex)
        self.h[: len(new)] = new
        self.K = len(new)
        self.x_ring = np.zeros((max_parts, part + 1), complex)
        self.head = -1
        self.tail = np.zeros(part)
        self.pending = None  # [spectra, cursor, eff_k]

    def _partition(self, ir):
        P = self.P
        k = int(np.ceil(len(ir) / P))
        h = np.zeros((k, P + 1), complex)
        for i in range(k):
            seg = ir[i * P : (i + 1) * P]
            buf = np.zeros(2 * P)
            buf[: len(seg)] = seg
            h[i] = np.fft.rfft(buf)
        return h

    def queue(self, ir):
        """Begin stepwise replacement with a new IR (any length ≤
        max_parts·P). Only when idle — mirrors the engine's policy."""
        assert self.pending is None
        new = self._partition(ir)
        assert len(new) <= self.max_parts
        eff_k = max(len(new), self.K)
        self.K = eff_k  # rows beyond old K are zero (invariant) — safe
        self.pending = [new, 0, eff_k]

    def process_frame(self, frame):
        P = self.P
        assert len(frame) == P
        if self.pending is not None:
            new, cur, eff_k = self.pending
            if cur < eff_k:
                self.h[cur] = new[cur] if cur < len(new) else 0.0
                self.pending[1] += 1
            if self.pending[1] >= eff_k:
                self.K = len(new)
                self.pending = None
        self.head = (self.head + 1) % self.max_parts
        buf = np.zeros(2 * P)
        buf[:P] = frame
        self.x_ring[self.head] = np.fft.rfft(buf)
        acc = np.zeros(P + 1, complex)
        for k in range(self.K):
            acc += self.h[k] * self.x_ring[(self.head - k) % self.max_parts]
        y = np.fft.irfft(acc)
        out = y[:P] + self.tail
        self.tail = y[P:]
        return out

    def process(self, x):
        n = int(np.ceil(len(x) / self.P)) * self.P
        xp = np.zeros(n)
        xp[: len(x)] = x
        out = np.zeros(n)
        for i in range(0, n, self.P):
            out[i : i + self.P] = self.process_frame(xp[i : i + self.P])
        return out


def dual_convolver_reference(x, h_a, h_b, n_t):
    """B&S reference: two overlapped time-invariant convolutions — the old
    convolver's input stops at n_t (tail rings out), the new one's starts
    there."""
    xa = np.array(x)
    xa[n_t:] = 0.0
    xb = np.array(x)
    xb[:n_t] = 0.0
    ya = fftconvolve(xa, h_a)
    yb = fftconvolve(xb, h_b)
    n = max(len(ya), len(yb))
    out = np.zeros(n)
    out[: len(ya)] += ya
    out[: len(yb)] += yb
    return out


def snr_db(ref, test):
    n = min(len(ref), len(test))
    err = ref[:n] - test[:n]
    return 10.0 * np.log10(np.sum(ref[:n] ** 2) / max(np.sum(err**2), 1e-300))


def exp_decay_noise(seconds, sr, t60, cutoff=8000.0, seed=1):
    """Synthetic room: one-pole-lowpassed noise under exponential decay,
    energy-normalized (mirrors the CLI's --synth-irs)."""
    rng = np.random.default_rng(seed)
    n = int(seconds * sr)
    x = rng.standard_normal(n)
    alpha = 1.0 - np.exp(-2 * np.pi * cutoff / sr)
    lp = np.zeros(n)
    acc = 0.0
    for i in range(n):
        acc += alpha * (x[i] - acc)
        lp[i] = acc
    env = 10.0 ** (-3.0 * np.arange(n) / (t60 * sr))
    h = lp * env
    return h / np.sqrt(np.sum(h**2))
