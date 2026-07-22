# lab — Python prototype lab

Float64 numpy/scipy reference implementations establishing the *math*
before the Rust engine establishes the *engineering*. Run from this
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
