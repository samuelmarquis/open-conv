# 01 — Architecture (before-the-fact plan)

*Written 2026-07-21, before the first line of engine code. The as-shipped
walkthrough will live in `docs/DSP.md` once there is a shipped thing to walk
through. Prior-art grounding: `docs/research/01-prior-art.md` (§ references
below point there).*

## What we are building

A convolution reverb that escapes two limits of conventional convolvers:

- **L1 — modulatable IR parameters.** Size/stretch (and later damping, etc.)
  sweepable during playback with no clicks and no interruption.
- **L2 — level-dependent spaces.** Different IRs gated by input level —
  dynamic convolution (Kemp 1999) aimed at *rooms* instead of gear.

## v1 engine: level-gated branches over a streaming partitioned convolver

```
                       ┌────────────────────────────────────────────┐
            level      │ zone weight windows w_m (partition of 1)   │
 x(n) ──┬── detect ───▶│  Instant: w_m(|x(n)|)   (per-sample, Kemp) │
        │  (|x| / env) │  Envelope: w_m(env(n))  (attack/release)   │
        │              └───────┬────────────────────────────────────┘
        │            ┌─────────┼─────────┬──────────┐
        │        x·w_0      x·w_1     x·w_2      x·w_3       (shaped inputs)
        │            │         │         │          │
        │        ┌───▼───┐ ┌───▼───┐ ┌───▼───┐ ┌───▼───┐
        │        │ conv 0│ │ conv 1│ │ conv 2│ │ conv 3│    UPOLA branches,
        │        │ IR_0  │ │ IR_1  │ │ IR_2  │ │ IR_3  │    each with stepwise
        │        └───┬───┘ └───┬───┘ └───┬───┘ └───┬───┘    partition streaming
        │            └────┬────┴────┬────┴──────┘
        │                 Σ  · zone_gain_m · wet
        └── dry delay (latency-aligned) ──────Σ──▶ y(n)
```

### Why this shape (the load-bearing results)

1. **Branch factorization (research §5.4).** Kemp's per-tap dynamic
   convolution `y(n)=Σ_k x(n−k)·h_{S(x(n−k))}(k)` factorizes exactly into
   static branches `y = Σ_m (f_m(x) * h_m)` with `f_m(x) = x·w_m(|x|)` when
   the interpolation weights are evaluated per-tap. Each branch is LTI ⇒
   FFT-partitionable ⇒ full-length reverb IRs are affordable (Kemp was stuck
   at 2048 taps of direct form on 9 SHARCs; we get seconds of tail per branch
   on one core). Verified in `lab/exp01`.
2. **Stepwise partition replacement (research §3.4, Brandtsegg & Saue
   DAFx-17).** Replacing IR partitions one-per-block in load order inside a
   uniformly partitioned OLA convolver reproduces a dual-convolver crossfade
   at zero added cost (their measured 67.2 dB SNR; ours in `lab/exp02`).
   This is the *universal update path*: IR swaps, size retargets, damping
   re-renders — everything becomes "stream new partitions."
3. **The IR is a stream, not an object (research §7-B).** A non-RT renderer
   re-generates partition spectra under the current knob state; the RT engine
   only ever *consumes* prepared partitions at partition boundaries. Sweeping
   Size just works; the transition is the reverb's own decay.

### Level zones

- Up to `MAX_ZONES = 4` zones, centers in dBFS (default −48/−30/−18/−6),
  triangular interpolation **in dB space** between adjacent centers; below
  the lowest / above the highest center the extreme zone owns everything.
  Weights form a partition of unity ⇒ with identical IRs in all zones the
  engine degenerates to an ordinary convolver (regression probe).
- **Instant mode** = Kemp's per-sample selector: zone-crossing at audio rate
  behaves like a waveshaper family — harmonic side-color is *the* dynamic
  convolution character, kept deliberately.
- **Envelope mode** = Kemp's/US7095860's smoothed selector: asymmetric
  one-pole attack/release follower (shared across channels, fed by the
  per-sample channel max). Weights move at envelope speed — "which room am I
  in" follows dynamics, no waveshaping. NB in envelope mode the weights are
  *shared state*, so strictly the factorization argument no longer applies —
  but since w_m(env) varies smoothly at audio rate on the *input* side, the
  branch inputs stay continuous and click-free by construction.
- Departure from Kemp (logged): he used 128 linearly-spaced amplitude bins
  (measurement fidelity for gear); we use ≤4 perceptual zones in dB
  (creative sound design, user-loadable IRs per zone). PCA-compressed banks
  (Primavera 2012) are the path back to dense ladders if ever needed
  (PATHS-NOT-TAKEN #6).

### Convolver core

- **Uniform partitioned overlap-add (UPOLA)**, partition P = 256 samples
  default, FFT size 2P, `realfft`. Frequency-domain delay line: ring of past
  input spectra per branch/channel, `Y = Σ_k H_k · X_{m−k}`, one IFFT per
  branch/channel/block, P-sample overlap tail.
- Uniform (not Gardner/García non-uniform) because the B&S replacement proof
  is for uniform OLA, and NUP exchange has the Müller-Tomfelde
  coherence/latency trade (research §3.7). García scheduling is a later
  CPU optimization (PATHS-NOT-TAKEN #7). OLS likewise deferred (#8).
- **Latency = P samples** (256 ≈ 5.3 ms @48k), reported honestly. Not
  chasing Gardner zero-latency in v1 (#5): reverbs tolerate small PDC.
- Per (branch, channel) cost @48k, P=256, 3 s IR ≈ 1 FFT + ~560 complex
  MAC-blocks + 1 IFFT per 256 samples — well within budget for 4 branches
  × 2 channels; measured, not assumed, once the CLI exists.

### IR streaming protocol (RT discipline)

- `Engine::process_block` is allocation-free and lock-free. All spectra
  rings, FIFOs, scratch: preallocated for `MAX_IR_SECONDS` at construction.
- Non-RT side (`service()` in the CLI now; a worker thread in the plugin
  shell later) renders `PartitionSet`s (resample → window → FFT per
  partition) and hands them over as a `pending` swap (a move, no alloc on
  the RT side). RT advances a cursor one partition per block — the B&S
  stream. Retired sets are handed back to the non-RT side for dropping.
- Size v1 = plain resampling of the source IR (pitch-coupled stretch — the
  classic "size" sound). Yamaha-style granular stretch (research §4.2) is
  the upgrade path when listening demands pitch-invariant size (#9).

### Params (flat `EngineParams`, one field per knob — template contract)

`n_zones, zone_db[4], zone_gain[4], level_mode, attack_ms, release_ms,
wet, dry, size` — the shells translate; behavioral choices are engine enums.

### Viz feed

`VizFrame { t, in_peak_db, env_db, weights[4], zone_energy[4],
swap_progress }`, fixed 16-frame ring, `viz_pop()`. CLI `--viz-dump` JSONL
doubles as the future panel's data contract (template convention). The
panel concept: a level-meter "ladder" showing which zone(s) the signal is
exciting, live, with per-zone tail energy — decided when we get there.

## Milestones

1. **M1 (now):** engine + CLI render probes; lab experiments validate
   factorization & replacement SNR; first listening batch in the LOG.
2. **M2:** size-sweep quality (resample vs granular), damping axis
   (per-zone tilt/T60 re-render through the same streaming path), zone
   crossfade character tuning by ear.
3. **M3:** WRAC shell + panel (per template §7: WRAC-only, no nih-plug).
4. **M4 (exploration):** hybrid parametric tail (dark velvet noise / modal —
   research §7-C), bilinear second axis (LFO/macro × level), Farina xᵏ
   drive branches.

## Testing spine

- `lab/exp01` — factorization equivalence (must be ~exact).
- `lab/exp02` — stepwise replacement SNR vs dual-convolver reference
  (target ≥60 dB, B&S report 67.2).
- Probes (`testdata/probes/`, generated): `staircase` (noise bursts through
  zone centers), `sineburst` (1 kHz at zone-boundary levels — zipper hunt),
  `impulses` (sparse clicks at varied levels — IR identity per zone),
  `sweepbed` (sustained pad — size-sweep clickability).
- CLI regression: all-zones-same-IR == single-convolver reference render.
