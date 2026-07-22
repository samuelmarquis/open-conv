# lab — Python prototype lab **[FROZEN 2026-07-22]**

**The Rust engine (`rt/engine`) is canonical.** This lab is kept as the
mathematical record of the load-bearing proofs (exp01–03) and as the
re-entry vehicle for the parametric tail (exp04 / PATHS-NOT-TAKEN #10).
Do not extend it to mirror engine features; write new experiments only
for *new* math, against the engine's docs (`docs/DSP.md`).

**Known drift vs the shipped engine** (deliberate, will not be fixed):

- `Upola` models a single H bank: no write-fade stages, no morph rate,
  no displacement, no epoch voices, no corner blend, no weight gating.
- Zone weights lack `sym` mirroring; no crystal shapers exist here.
- `exp_decay_noise` predates `engine::banks` (different from the
  shipped `rooms` bank: energy-normalized, not windowed-spectral).
- Everything is float64; the engine is f32.

Float64 numpy/scipy reference implementations establishing the *math*
before the Rust engine established the *engineering*. Run from this
directory (`python exp01_factorization.py` …) inside `nix develop`.

- `convlab.py` — shared reference lib: Kemp dynamic convolution (per-tap),
  branch factorization, UPOLA convolver with B&S stepwise replacement,
  dual-convolver reference, synthetic rooms.
- `exp01_factorization.py` — proves the §5.4 factorization exactly and
  quantifies Kemp's eq.3 p(x(n)) inconsistency.
- `exp02_stepwise.py` — reproduces B&S DAFx-17's replacement-vs-reference
  SNR regime (target ≥60 dB; they report 67.2), incl. an IR-length-change
  case.
- `exp03_size_stream.py` — the L1 headline in miniature: size sweep as
  streamed re-renders vs hard swaps; renders comparison wavs into
  `out/` (untracked) for listening.

Iterations are strategy toggles selected per listening batch, numbered in
LISTENING-LOG.md, not in filenames. **Freeze note:** once the Rust engine
is canonical, this lab gets a header saying exactly that + known drift.
