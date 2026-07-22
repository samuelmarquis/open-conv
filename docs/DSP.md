# DSP.md — the as-shipped engine walkthrough

*Written 2026-07-22, after the fact, describing what `rt/engine` actually
does as of M4 + the batch-008/009 fixes. The before-the-fact plan is
`design/01-architecture.md`; the evidence base is
`research/01-prior-art.md` (§ refs below); the *reasons* live in
`LISTENING-LOG.md` — many numbers here exist because a specific batch
verdict demanded them. The unit tests are the executable version of this
document.*

## 0. One-paragraph topology

Audio is processed in frames of `P = 256` samples (reported latency = P).
Each frame, an **input-shaping stage** splits the signal into four branch
signals (level *zones*, or harmonic *orders* in the crystal modes). Each
branch signal is FFT'd **once** into its zone's shared spectra ring, then
convolved by up to **four corner convolvers** (the XY pad) whose outputs
are bilinearly weighted by Blend X/Y. Every corner runs the full IR
machinery: a streaming-replacement H bank (Gated tails), plus parallel
input-epoch voices (Ungated tails). Wet sums linearly with the
frame-aligned dry — there is **no saturation stage anywhere** (engine
law, batch 007: "distortion bad").

```
in ─┬─ shaping (Zones | Quartz | Bismuth) → shaped[z], z = 0..3
    │        zone z: FFT once → shared x_ring[z]
    │        ┌────────────── corner convolvers ──────────────┐
    │        │ NW: H-bank + pending stream + epoch voices    │ ×4 corners,
    │        │ NE/SW/SE: same                                │ blended by
    │        └── Σ · w_corner(X,Y) · zone_gain · wet ────────┘ Blend X/Y
    └─ dry (delayed P, unity path) ──────Σ──▶ out
```

## 1. Framing, FIFOs, latency

`process_block` accepts any host block size; samples accumulate in
`in_fifo` and a frame fires every `P` samples. Output pops from
`out_fifo`, primed with exactly `P` zeros at reset — so reported latency
is exactly `P` (asserted by `latency_is_one_partition`). Dry is mixed
inside the frame (input frame × dry gain), so bypass (shell-side: wet 0
/ dry 1) is click-free and PDC-exact.

## 2. Input shaping

### 2.1 Zones mode (the level ladder; research §5.4)

Kemp's per-tap dynamic convolution factorized: branch z hears
`x · w_z(level)` where `w_z` are triangular windows **in dB space** over
the ascending zone centers (partition of unity; verified to 1e-6 across
the range). Level source per `LevelMode`:

- **Instant** — per-sample rectified value, per channel. Zone crossings
  at audio rate = the waveshaper color (validated as a feature, batch
  002 verdict).
- **Envelope** — shared asymmetric follower on the channel-max
  (`coeff = exp(−1/(ms·sr/1000))`), attack when rising, release when
  falling.

**Symmetry** (`sym` 0..1): on negative samples the weight vector is
convexly blended toward its mirror (`w'_z = (1−s)·w_z + s·w_{n−1−z}`) —
still a partition of unity, so the transparency regression holds. Both
modes, per sample.

The factorization identity (`exp01`, 305 dB float64) is what lets
level-dependent convolution run in partitioned FFT form at all — and the
lab quantified that Kemp's own eq. 3 as printed (output-time
interpolation fraction) collapses to ~10 dB SNR at 4-zone granularity,
so we build the consistent per-tap form.

### 2.2 Crystal modes (slots = harmonic orders 1..4)

Both shapers share plumbing: per order k, the input is pre-lowpassed by
a 2× cascaded one-pole at `0.45·sr/k` (generated harmonics stay in-band
— the "clean law"), and **even orders are DC-blocked** (one-pole HP,
R = 0.995; x² of a sine carries DC). `n_zones` limits the order count.
Filter states live per (zone, channel) on the engine.

- **Quartz** (Chebyshev; research §5.2, Novak's basis):
  `s = g·lp/√(1+(g·lp)²)` (algebraic sigmoid — bounded), then
  `T₂+1 = 2s²` (×0.5), `T₃ = 4s³−3s`, `T₄+1 = 8s⁴−8s²` (×0.5), rest
  constants dropped so silence stays silence. Bounded by construction;
  approaches *pure* k-th harmonics as drive opens (equioscillation —
  hence the name). Diffuse character.
- **Bismuth** (raw powers; batch-008 saga): `lpᵏ · mult(g)`. Pure powers
  are homogeneous — pre-gain ≡ post-gain — so this is *exactly* the
  original v1 waveform at every knob position; only the level law is
  tempered: `mult = g^(k−1)` for `g ≤ 2` (v1-identical in the beloved
  region), continuing at ⅓ the dB slope above (g=8: order 4 = +30 dB,
  not +54). The signature rumble is even-power envelope rectification
  (x²/x⁴ of decaying impacts → LF swell), pitched by short IRs at low
  Size — a component Quartz's balanced polynomials deliberately lack.
  Verdict on record: "coolest sounds of ANY plugin… n=3."

`drive` (Crystal Gain) is `g`, 1..8. Zone gains act as per-order sends;
Selector/Attack/Release/Symmetry are inert in crystal modes (v1 scope).

## 3. The zone: shared ring + four corners

Per zone: one `InputRing` (`x_ring[ch][max_parts × bins]`, head index)
and `CORNERS = 4` corner convolvers (`Branch`), indexed zone-major
(`branches[z·4 + c]`). The shaped input is FFT'd **once per zone per
channel** into the ring; corners and all their voices read it — the key
economy that makes both the XY pad (M4) and Ungated voices nearly free
on the input side.

**Never resize a spectra ring mid-stream** — ring-modulo history
corruption; found the hard way in the lab (batch 001 notes). Rings and H
banks are sized once at construction (`max_parts = ⌈sr·max_ir_seconds/P⌉`,
default 5 s) and never touched.

## 4. The corner convolver

Uniform partitioned overlap-add (UPOLA): FFT size `2P`, `bins = P+1`,
frequency-domain delay line `Y = Σ_k H_k · X_{head−k}`, one IFFT, first
half + carried OLA tail out, second half becomes the new tail.
Correctness vs naive convolution: rel err < 1e-4 f32
(`single_zone_matches_naive_convolution`). DC/Nyquist imaginary parts
are zeroed before each IFFT (realfft contract insurance).

**H-bank invariant:** partitions beyond `active_k` are always zero —
relied on whenever a swap grows k (the grown region contributes nothing
until the cursor writes it).

## 5. Gated tails: the streaming replacement (research §3.4)

All IR *content* changes (loads, Size, Damp, bank switches in Gated
mode) are one mechanism: **B&S stepwise partition replacement** — the
`Pending` cursor writes `morph` partitions per frame in load order.
The lab proved the schedule *exact* (302 dB vs the dual-convolver
reference, including IR-length changes) — B&S's own 67.2 dB was their
float32 measurement, not the method's floor.

Shipped refinements, each traceable to a batch verdict:

- **`morph`** (1..16 partitions/frame): 1 = tail-length glide; 16 ≈
  200 ms for a 3 s IR (batch-007 automation-determinism report).
- **Graduated writes** (`fade_frames` 1..MAX_FADE=16): each written
  partition fades to target over `fade` frames via the in-place
  recurrence `h += (T−h)/(fade+1−s)` (lerp chain ¼→½→¾→1 at fade 4);
  stages at/past `fade` finalize exactly, which also covers a fade
  decrease mid-stream. Kills the "skitter" needles (batch-007 #2);
  fade 1 restores them on purpose (user control, batch-007 #3).
- **Displacement:** a new set replaces an in-flight pending (cursor
  restarts; H transiently mixes three IRs — click-free, since only a
  bounded slice changes per frame); the displaced set parks in the
  retired queue. Latest request always wins — the fix for
  edge-triggered messages dropping bank changes under fast automation.
- **Silence target:** `Pending.set = None` streams the H voice out to
  zero (Ungated bootstrap uses it).
- Completion updates `rendered_size/rendered_damp` bookkeeping and
  retires the set. `service`/shell-side staleness compares both.

## 6. Ungated tails: input-epoch voices (research §4.2, parallel-convolver architecture)

`queue_partition_set(…, Ungated)` freezes the corner's live voice and
adopts the new set as a fresh epoch. Exactness
(`ungated_epochs_are_exact_input_split`, >80 dB f32):
`y = conv(x_before, old) + conv(x_after, new)` — click-free by
construction, no crossfade machinery.

The cheap trick: **all voices share the zone's x_ring**, gated by lag
windows. Live voice at age a hears lags `0..=a` (never pre-epoch
input); a frozen voice at m frames post-freeze with epoch length L
hears `m..min(k, m+L)` (its own epoch only — both bounds matter; the
missing upper bound was a real bug caught by the exactness test). A
voice is its adopted `PartitionSet` + a P-sample OLA tail (pooled) —
near-zero memory. Dead when `m ≥ k` (tail flushed on its last frame).

**Ring slots & graceful eviction** (batch-007 #4/#5): `RING_SLOTS = 8`
frozen voices per corner, always at max (the depth knob was removed —
capacity only bites under fast switching, where eviction handles it).
Freeze placement: free slot → quietest dying → oldest (last resort).
When slots fill, the oldest healthy voice is proactively marked *dying*:
gain ×0.5 per frame, **ramped per-sample within the frame** (a
per-frame gain step is itself a staircase of clicklets — the eviction
storm test caught my first fix's own defect). Hard steals thus only
ever touch voices ≥ ~54 dB down. `ungated_eviction_storm_stays_smooth`
pins it (max |Δ²| ~10× under hard-cut territory).

## 7. XY blend (M4)

`w = [(1−x)(1−y), x(1−y), (1−x)y, xy]` over corners NW/NE/SW/SE —
**output-gain math**: instant, comb-free (the naive alternative,
interpolating IR waveforms, comb-filters; research §4.2), exact
(`blend_mixes_corners_exactly`, >80 dB). Weights are **ramped in-frame**
from the previous frame's values (`prev_w`). Corners with both old and
new weight ≤ `W_EPS = 1e-4` skip all output compute; on skip entry
their tails (corner + voices) are zeroed once so re-entry carries no
stale remainder — bookkeeping (pending cursors, voice ages) continues
regardless, so a silent corner stays time-consistent.

## 8. The render path (control side)

`IrRenderer::render(data, ir_sr, size, damp)`:

1. **Resample** (linear) by `(sr/ir_sr)·size` — pitch-coupled stretch
   (the classic "size"; on tuned banks a *tuning* knob, per the batch-002
   "bonus law"), energy-compensated by `1/√size`.
2. **Damp**: two cascaded one-poles (−12 dB/oct) whose cutoff glides
   log-linearly from 18 kHz down to `18k·(600/18k)^damp`, reaching full
   depth **40 % into the tail** (the first version reached it only at
   the very end — audibly too polite). Heads stay bright = the sample's
   identity survives (`damp_darkens_the_render`).
3. **Partition + FFT** into a `PartitionSet` (spectra laid out
   `[(c·k + part)·bins + bin]`, carrying `rendered_size/damp`).

Synthetic banks (`engine::banks`, shared with the CLI) are normalized by
**windowed spectral peak** (~85 ms frames, +6 dB burst-gain target).
The normalization saga is Defect 001 in the log: energy norm exploded
(+24 dBFS) on tonal IRs, global-spectral norm buried the wet 40–70 dB
(Q physics: bounded steady-state gain ⇒ tiny impulse response). Bound
the *burst* gain — the thing percussive material actually excites.
Target was +12, lowered to +6 by the clean-defaults verdict.

## 9. Threading contract (as exercised by the plugin shell)

- **RT** (`process_block`): allocation-free, lock-free. Accepts sets via
  `queue_partition_set` (move-only), parks spent/displaced sets in
  per-corner retired queues (capacity 12; `retire_push` never drops on
  the RT thread).
- **Control** (`set_source_ir[_at]`, `service`, `IrRenderer`,
  `take_retired`): allocates/drops freely. The shell's worker owns
  sources (banks / `~/Music/open-conv/zone{1..4}.wav`), reconciles to a
  coalesced desired-state `Sync{corners, size, damp, load_gen}`
  (deferred-never-dropped dirty flags, 50 ms wall-clock debounce so live
  and offline behave identically), renders 16 sets per change, and
  disposes everything the audio thread hands back. mpsc sends allocate
  — control-edge frequency only; a lock-free ring is queued for the
  panel milestone.

## 10. Numbers (48 kHz, defaults)

| thing | value |
|---|---|
| partition / latency | 256 samples (5.3 ms) |
| FFT size / bins | 512 / 257 |
| max IR (post-stretch) | 5 s → `max_parts` 938 |
| memory | H: 16 corners × 2 ch ≈ 62 MB; rings: 4 × 2 ch ≈ 15 MB |
| zones × corners × voices | 4 × 4 × (1 H + 1 live + 8 ring) worst case |
| morph / fade | 1..16 partitions/frame / 1..16 frames |
| eviction ramp | −6 dB/frame, per-sample lerped |
| throughput (pre-M4, 4 branches) | ~150× real-time; M4 worst-case ≈ ÷4, unmeasured |

## 11. The invariant list (things that bit us once)

1. Spectra rings never resize; H beyond `active_k` is always zero.
2. Nothing is ever dropped on the audio thread (retired queues, pools).
3. Every gain that changes per frame ramps *within* the frame (corner
   weights, dying voices) — per-frame steps are click staircases.
4. Desired-state reconciliation, never edge-triggered messages — a
   dropped edge is a dropped feature.
5. Peak checks are not presence checks; every render gate exists
   because its absence shipped a defect (Defect 001).
6. The wet path is linear. No tanh. "Distortion bad."
