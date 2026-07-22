# open-conv

A convolution reverb built to escape the two standing limits of convolution
reverbs:

1. **Modulatable IR parameters** — size/stretch and friends sweepable during
   playback, click-free, by treating the IR as a *stream* (background
   re-render + stepwise partition replacement, Brandtsegg & Saue DAFx-17)
   rather than a static object.
2. **Level-dependent spaces** — dynamic convolution (Kemp, AES 106th, 1999)
   aimed at rooms instead of gear: up to four IRs gated by input level,
   factorized into static waveshaper→convolver branches so the whole thing
   runs in partitioned FFT convolution at full reverb length.

Whisper into a cathedral; shout into a spring.

## Status

M3 — **OpenConv is installed**: CLAP + VST3 + AU (auval PASS), headless v0
(generic host UI; native panel is the next milestone). Engine/CLI/lab per M1;
six listening batches logged. Build: `cd wrac && cargo xtask install -p
open_conv_plugin_wrac --release`.

## Layout

- `docs/research/` — numbered clean-room evidence corpus
  (`01-prior-art.md` is the literature survey this design stands on)
- `docs/design/` — before-the-fact plans (`01-architecture.md`)
- `rt/` — Rust workspace: `engine/` (the DSP, plain Rust, dep: realfft) +
  `cli/` (offline renderer for listening batches)
- `lab/` — Python prototype lab (validates the two load-bearing claims:
  branch factorization exactness; replacement-vs-crossfade SNR)
- `tools/make_probes.py` — synthesizes `testdata/probes/` (committed script,
  generated audio stays untracked)
- `LISTENING-LOG.md` / `PATHS-NOT-TAKEN.md` — the decision records

## Build / run

```
nix develop
python tools/make_probes.py
(cd rt && cargo build --release)
rt/target/release/open-conv testdata/probes/staircase.wav out/first.wav \
  --synth-irs --mode envelope --viz-dump out/first.jsonl
python lab/exp01_factorization.py && python lab/exp02_stepwise.py
```

## License

**GPL-3.0-or-later** for everything in this repository (`LICENSE`).
Rationale: (a) distributed VST3 builds link Steinberg's VST3 SDK, whose
open-source option is GPLv3 — the binding constraint for any open-source
plugin; (b) GPLv3 accepts one-way the LGPL reference implementations
adjacent to this design (TVOLAP, Csound liveconv) should we ever port from
them; (c) GPLv3's contributor patent grant and retaliation clauses are
welcome in patent-adjacent DSP territory. The vendored WRAC template (when
added) remains MIT (GPL-compatible) under its own notice. Freedom-to-operate
notes: `docs/research/01-prior-art.md` §6 (not legal advice).
