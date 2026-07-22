# DSP.md — how the engine works

*This document tells you what the shipped engine (`rt/engine`) does.
It is written to ASD-STE100 Simplified Technical English: short
sentences, one idea for each sentence, and one meaning for each word.
Section 1 defines the technical names. Formulas and tables are data,
not prose. The unit tests are the exact version of this document. The
reason for each decision is in `LISTENING-LOG.md` (the maintainer's
local listening diary — not in the repo). The plan that came
before the code is `design/01-architecture.md`. The evidence base is
`research/01-prior-art.md` (the "§" references point into it). A
denser engineering version of this document is in the git history
(before 2026-07-22).*

## 1. Words

This document uses these technical names. Each has one meaning here.

- **Sample** — one number in the audio stream. At a 48 kHz sample
  rate, the engine gets 48,000 samples each second.
- **Frame** — a group of 256 samples. The engine does all its work
  one frame at a time.
- **Impulse response (IR)** — a recording of the sound that a space
  makes after one short click. An IR of a church holds the full echo
  of that church.
- **Convolution** — the operation that puts your sound into the space
  of an IR. Each input sample starts one copy of the IR, at the
  loudness of that sample. The output is the sum of all the copies.
  "To convolve" is the verb.
- **FFT / spectrum** — the FFT changes a group of samples into a
  spectrum: a list of the quantity of each frequency. Convolution is
  fast on spectra: one multiplication replaces many additions.
- **Partition** — one 256-sample piece of an IR. The engine cuts each
  IR into partitions and keeps each partition as a spectrum.
- **Wet / dry** — dry is your input, not changed. Wet is the
  convolved sound. The output is dry plus wet.
- **Latency** — the fixed delay of the engine: 256 samples (5.3 ms at
  48 kHz). The engine reports it, and the host corrects for it.
- **Zone** — one input level band (Zones mode), or one harmonic order
  (the crystal modes). There are up to four zones.
- **Corner** — one convolver on the XY pad. Each zone has four
  corners: NW, NE, SW, SE. Each corner can hold a different IR.
- **Voice** — one live copy of an IR, with its own start time.
- **Tail** — the reverb sound that continues after the input stops.
- **Click** — an unwanted step in the output signal. Most of this
  design exists to prevent clicks.

## 2. The signal path

The engine does five steps for each frame:

1. The **shaping stage** divides the input into four branch signals.
   In Zones mode, the branches are level bands. In the crystal modes,
   the branches are harmonic orders.
2. Each branch signal goes through the FFT one time. The result goes
   into the shared input ring of that zone.
3. In each zone, up to four **corner convolvers** read the same ring.
   Each corner convolves the branch signal with its own IR.
4. **Blend X/Y** mixes the four corner outputs. The mix weights come
   from the ball position on the XY pad.
5. The wet sum adds to the dry signal. There is no saturation stage
   anywhere. The wet path is fully linear (project law, batch 007:
   "distortion bad").

```
in ─┬─ shaping (Zones | Quartz | Bismuth) → shaped[z], z = 0..3
    │        zone z: FFT once → shared x_ring[z]
    │        ┌────────────── corner convolvers ──────────────┐
    │        │ NW: IR bank + pending stream + frozen voices  │ ×4 corners,
    │        │ NE/SW/SE: same                                │ mixed by
    │        └── Σ · w_corner(X,Y) · zone_gain · wet ────────┘ Blend X/Y
    └─ dry (delayed 256 samples, unity) ─────Σ──▶ out
```

## 3. Frames, buffers, latency

The host can send blocks of any size. The engine collects samples in
an input buffer. When 256 samples are there, one frame runs. The
output comes from an output buffer. At reset, the output buffer gets
exactly 256 zeros. Thus the reported latency is exactly 256 samples
(test: `latency_is_one_partition`).

The dry signal mixes inside the frame. Thus dry and wet stay aligned,
and bypass makes no click.

## 4. Zones mode — the level ladder

### 4.1 The idea

A usual convolution reverb gives all input levels the same IR. Zones
mode does not. Quiet parts of the input go to the low-zone IRs. Loud
parts go to the high-zone IRs. Load a small room in the low zone and
a large hall in the top zone. Then ghost notes stay tight, and
accents open the hall.

This is the "dynamic convolution" idea (Kemp 1999, §5.4). The direct
form is too slow for long IRs. The engine uses a mathematically equal
form: each zone hears the input, multiplied by its own level weight.
The lab proved that this equals the per-sample form (exp01, 305 dB
agreement).

### 4.2 The weights

Each zone has a center level in dB. The weights are triangles in dB
space across the centers. At every level, the four weights add up to
exactly one (test: `zone_weights_partition_of_unity`). Thus, if all
four zones hold the same IR, the result is a usual convolution.

The **Selector** control selects the level source:

- **Instant** — the level is the absolute value of each sample, for
  each channel. Zone changes occur at audio rate. This makes a
  waveshaper color. It is intentional (batch 002 verdict).
- **Envelope** — a smooth level follower with attack and release
  times. The follower reads the louder channel and applies to both.

**Symmetry** gives negative samples a different weight set. At full
Symmetry, the zone order is a mirror image for the negative half. The
weights continue to add up to one, so the transparency rule holds.

## 5. The crystal modes — Quartz and Bismuth

In the crystal modes, the four zone slots become harmonic orders 1 to
4. Order k makes a tone at k times the input frequency. The second
order of an A at 110 Hz is an A at 220 Hz. Each order goes into its
own corner set: each harmonic gets its own room. The Zones count
control limits how many orders run. The zone gain knobs become sends,
one for each order. **Crystal Gain** (g, 1 to 8) sets the strength of
the harmonics. Selector, Attack, Release, and Symmetry are not active
in these modes.

Common processing for both modes:

- Before order k, a lowpass filter at `0.45 · sr / k` removes the
  frequencies that would fold back down (aliasing). New harmonics
  stay in the audio band — the "clean law".
- Even orders (2 and 4) get a DC-block filter. The square of a sine
  contains a constant offset; the filter removes it.

**Quartz** (Chebyshev, §5.2). The input first goes through a bounded
curve: `s = g·x / √(1 + (g·x)²)`. Then Chebyshev polynomials make the
harmonics: `2s²` (order 2, ×0.5), `4s³ − 3s` (order 3), `8s⁴ − 8s²`
(order 4, ×0.5). Constant terms are removed, so silence stays
silence. The output stays bounded at every gain. As the gain opens,
each order becomes an almost pure harmonic. The character is smooth
and diffuse.

**Bismuth** (raw powers). Order k is `xᵏ`, multiplied by a gain law.
A pure power has a special property: gain before it equals gain after
it. Thus Bismuth makes exactly the first-version (v1) waveform at
every knob position; the knob only sets the level. The level law is
`g^(k−1)` up to g = 2 — identical to v1 in that region — and then 1/3
of the dB slope above it. The signature low rumble comes from the
even powers: `x²` of a decaying hit becomes a low swell. Short IRs at
low Size give that swell a pitch. Verdict on record: "coolest sounds
of ANY plugin… n=3."

## 6. One FFT for each zone, four corners

Each zone owns one input ring: the spectra of its recent frames. The
branch signal goes through the FFT one time for each zone and
channel. All four corners, and all their voices, read the same ring.
This shared work makes the XY pad and the Ungated voices almost free
on the input side.

The ring never changes size while audio runs. A size change corrupts
the ring history (found in the lab, batch 001). Rings and IR banks
get their full size at start (5 s of IR maximum) and keep it.

## 7. The corner convolver

Each corner is a uniform partitioned convolver (UPOLA):

- The FFT size is 512 samples (two partitions). A spectrum has 257
  bins.
- Each frame, the output spectrum is the sum `Y = Σ_k H_k · X_{head−k}`
  — IR partition k, multiplied by the input spectrum from k frames
  ago.
- One inverse FFT gives 512 samples. The first 256 go out, plus the
  kept overlap from the last frame. The second 256 become the new
  overlap.

The result equals direct convolution to better than 0.01 % (test:
`single_zone_matches_naive_convolution`). One rule always holds: IR
partitions after the active count are zero. IR growth depends on this
rule — the new region is silent until it is written.

## 8. Gated tails — the streamed replacement

All IR content changes go through one mechanism: **stepwise partition
replacement** (Brandtsegg & Saue, §3.4). This includes IR loads,
Size, Damp, and pad-corner changes in Gated mode. The engine does not
stop and swap. Each frame, it writes some partitions of the new IR
over the old one, front to back. Old reverb becomes new reverb along
the tail. The lab proved this schedule exact (exp02, 302 dB against a
two-convolver reference, IR-length changes included).

Refinements, each caused by a listening verdict:

- **Morph** (1 to 16 partitions per frame). At 1, a change takes the
  full tail length. At 16, a 3 s IR changes in approximately 200 ms.
- **Transition Fade** (1 to 16 frames). Each written partition fades
  to its target across the fade count, with the recurrence
  `h += (T − h) / (fade + 1 − s)`. At fade 4, the steps are ¼ → ½ →
  ¾ → 1. This removes the "skitter" needles of hard writes
  (batch 007). Fade 1 gives the hard writes back, on purpose.
- **Displacement.** A new request replaces a change that is not
  complete. The write position starts again; the newest request
  always wins. For a short time, the IR is a mix of three IRs. That
  is safe: only a small piece changes in each frame, so no click
  occurs.
- **Silence target.** A pending change can point at silence. The IR
  then streams out to zero. Ungated mode uses this at start.

## 9. Ungated tails — voices

In Ungated mode, an IR change does not touch the old sound at all.
The engine freezes the live voice and starts a new voice with the new
IR. The frozen voice keeps only the input from its own time period.
The new voice hears only the new input. The sum of the two is exactly
one unbroken convolution (test: agreement better than 80 dB). No
crossfade is necessary. No click is possible.

The voices cost little, because all of them read the shared ring of
their zone. Each voice only selects a different time window of it:

- The live voice, at age `a` frames, reads lags 0 to `a`.
- A frozen voice, `m` frames after its freeze, with period length
  `L`, reads lags `m` to `min(k, m + L)`. Both limits matter. The
  missing upper limit was a real defect; the exactness test caught
  it.

Each corner keeps up to 8 frozen voices. When the slots are full, the
oldest healthy voice starts to die: its gain halves each frame, with
a smooth ramp inside each frame. A gain step at the frame edge is
itself a stairway of small clicks — a test caught this in the first
fix. A hard removal thus only touches voices that are 54 dB down or
more (test: `ungated_eviction_storm_stays_smooth`).

## 10. The XY pad

The pad mixes the four corner outputs. A corner gets more weight when
the ball is nearer to it. The four weights always add up to one:
`w = [(1−x)(1−y), x(1−y), (1−x)y, xy]` for NW, NE, SW, SE. The engine
mixes output gains only. The movement is instant and makes no comb
filter. (The alternative — interpolation between IR waveforms — makes
comb filters; §4.2.) The mix is exact (test: agreement better than
80 dB).

The weights ramp inside each frame, from the values of the last
frame. A corner with a weight below 0.0001, old and new, does no
output work. On entry to this idle state, the engine sets the
overlaps and voice tails of that corner to zero, one time. Its clocks
continue, so the corner agrees with time when it comes back.

## 11. How an IR is prepared

The control side prepares each IR before the audio thread gets it:

1. **Size.** The IR is resampled by the Size factor. This is the
   classic size sound: pitch and length change together. On tuned
   banks, Size is thus also a tuning knob. The energy compensation is
   `1 / √size`.
2. **Damp.** Two one-pole lowpass filters in series (−12 dB for each
   octave). The cutoff starts at 18 kHz and glides down along the
   tail, to `18k · (600/18k)^damp`. It gets to full depth 40 % into
   the tail. The first version got there only at the very end — too
   polite to hear. The head stays bright, so the identity of the
   sample survives (test: `damp_darkens_the_render`).
3. **Partition + FFT.** The result is cut into partitions and kept as
   spectra, together with its Size and Damp values.

The synthetic banks are normalized by **windowed spectral peak**: the
largest gain that any 85 ms piece of the IR can apply to a burst. The
target is +6 dB. This is the answer to Defect 001. Energy
normalization exploded on tonal IRs (+24 dBFS). Global spectral
normalization buried the wet sound by 40 to 70 dB — a high-Q IR with
a bounded peak gain has an almost silent impulse response. The
correct bound is the burst gain, because bursts are what drums send
in.

## 12. Two threads

**The audio thread** runs `process_block`. It never allocates memory,
and it never waits on a lock. It receives prepared IRs as move-only
messages. It puts used and displaced IRs into a "retired" queue
(capacity 12 for each corner), and it never drops one.

**The worker thread** does everything slow. It loads the bank sources
and the user files (`~/Music/open-conv/zone1..4.wav`). It keeps one
desired state: which IR, which Size, which Damp, for each corner.
Control changes only mark this state as changed. At most every 50 ms,
the worker compares, renders the 16 IRs that changed, and sends them.
Marks are deferred, never dropped. Thus a live automation ride and an
offline render give the same result. The worker also frees everything
that the audio thread retires. Message sends allocate memory; they
occur only at control changes. A lock-free ring is planned with the
panel milestone.

## 13. Numbers (48 kHz, default settings)

| item | value |
|---|---|
| partition / latency | 256 samples (5.3 ms) |
| FFT size / spectrum bins | 512 / 257 |
| maximum IR (after stretch) | 5 s → 938 partitions |
| memory | IR banks: 16 corners × 2 ch ≈ 62 MB; rings: 4 zones × 2 ch ≈ 15 MB |
| zones × corners × voices | 4 × 4 × (1 bank + 1 live + 8 frozen) worst case |
| Morph / Transition Fade | 1..16 partitions per frame / 1..16 frames |
| eviction ramp | −6 dB per frame, ramped per sample |
| throughput (pre-M4, 4 branches) | ~150× real time; M4 worst case ≈ ÷4, not measured |

## 14. Rules that a defect taught us

1. A spectra ring never changes size. IR partitions after the active
   count are always zero.
2. The audio thread never drops or frees anything. Queues and pools
   catch everything.
3. Every gain that changes must ramp inside the frame. Steps at frame
   edges are stairways of clicks.
4. Keep one desired state and compare with it. Do not send edge
   messages — a lost edge is a lost feature.
5. A peak check is not a presence check. Each render gate exists
   because its absence shipped a defect (Defect 001).
6. The wet path is linear. No saturation. "Distortion bad."
