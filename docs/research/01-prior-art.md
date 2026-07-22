# 01 — Prior art: dynamic convolution, time-varying & nonlinear convolution reverb

*Compiled 2026-07-21 from a 23-agent research sweep (5 search angles → 12 primary-source
deep reads → 66-claim adversarial verification: 60 confirmed / 2 plausible / 4
refuted-or-corrected). Raw fleet output preserved in `raw/`. Clean-room hygiene: public
documents only — papers, patents, vendor documentation, open-source code. No disassembly
of commercial products. Citations inline; verification caveats marked ⚠.*

**Project goal recap.** Two limitations of conventional convolution reverbs are being
attacked: **(L1)** IR-altering parameters (size/stretch/pitch, damping…) cannot be
modulated during playback without clicks or interruptions; **(L2)** convolution is
LTI-only — we want input-level-dependent impulse responses.

---

## 1. The "dynamic convolution" paper (pinned)

**Michael J. Kemp (Sintefex Audio Lda), "Analysis and Simulation of Non-Linear Audio
Processes using Finite Impulse Responses Derived at Multiple Impulse Amplitudes,"
AES 106th Convention, Munich, 8–11 May 1999, preprint #4919 (session J-5).**
Republished verbatim by Sintefex as an app note titled simply *"Dynamic Convolution"*
(© 1999/2000). Full 15-page text retrievable via the Wayback Machine:
`web.archive.org/web/20240616125342/http://www.sintefex.com/docs/appnotes/dynaconv.PDF`
(the live URL 403s). Citation, venue, and preprint number verified against the AES
e-library entry (elib 8261).

Notably, Kemp's paper *is* limitation L2: per-sample input amplitude indexes a family of
IRs measured at different drive levels. It was built for gear emulation (valve EQ, tape,
guitar amp, compressors), not spaces — pointing it at rooms is the unexplored move.

### 1.1 Measurement

- Excite the device with **step** impulses (more LF energy than unit impulses) at
  **M = 128 linearly spaced amplitudes** from full scale down (step d = FS/128;
  *note: Kemp writes full-scale amplitude as "fs" — not sample rate*).
  Alternating-polarity step train; 8000-sample settle, 4000-sample capture per step,
  at 44.1 kHz.
- Convert each step response to a unit IR by first-order differencing
  `y[n] = x[n] − x[n−1]` (double-precision mandatory), window the final quarter with a
  linear ramp.
- Normalize the m-th IR by `FS/(FS − m·d)` so all IRs sit at a common peak-referenced
  level.
- **Noise handling** (the low-level IRs sit ~40 dB down): (a) *noise-floor splicing* —
  walk each IR down to where it nears the measured noise floor, splice in data from the
  next-cleaner (higher-level) IR with crossfades at the boundaries; (b) coherent
  averaging of repeats (3 dB/doubling); (c) discard IRs below ~12 dB-from-peak for
  devices that are essentially linear there, replicating the lowest retained IR.

### 1.2 Playback algorithm (implementation-grade, verified verbatim against the primary)

Selector and interpolation fraction, per sample magnitude:

```
S(x) = 1 + ⌊ |x| / (FS/M) ⌋            (define h_0 := h_1 to avoid special-casing)
p(x) = ( |x| mod (FS/M) ) / (FS/M)
```

Dynamic convolution (Kemp eq. 3):

```
y(n) = Σ_k  x(n−k) · [ p·h_{S(x(n−k))}(k)  +  (1−p)·h_{S(x(n−k))−1}(k) ]
```

Every tap is a 2-tap blend between the two IRs bracketing **that delayed sample's**
magnitude — the selector is evaluated per-tap on `x(n−k)`, not once per output sample.
⚠ Kemp's own eq. 3 mixes per-tap `S(x(n−k))` with per-output-sample `p(x(n))` — an
internal inconsistency in the original (verifier-confirmed); the consistent form uses
`p(x(n−k))` per-tap, which is also what makes the branch factorization in §5.4 exact.

Engine realization (his Fig. 7): loop once per **input** sample; compute (S, p) once;
split x(n) into two scaled values; one pass over the length-L buffer doing **2 MACs/tap**
into a sliding partial-sum output buffer. On the SHARC's dual-fetch/MAC architecture
this hit ~100% MAC efficiency — dynamic convolution at half the throughput of plain
convolution.

### 1.3 Extensions already in the 1999 paper

- **Envelope selector:** replace instantaneous |x| with an attack/decay envelope
  follower ("we have done some tests using an envelope of the input signal as the
  selection criterion") — explicitly to avoid switching artifacts.
- **Sign-dependent IRs:** separate IR ladders for positive/negative-going impulses,
  selected by `sign(x(n−k))` (proposed, unimplemented).
- **Bilinear second axis (§5):** whole IR ladders measured at multiple settings of a
  second control (user knob, LFO, or compressor gain reduction), combined by bilinear
  interpolation over (amplitude-index, control-index). This is the direct ancestor of
  "modulatable multi-dimensional IR family selection."

### 1.4 Costs and hardware reality check

| Datum | Value |
|---|---|
| Prototype (offline), 90 MHz Pentium | ≤1000 taps; 30 s of audio took hours |
| Real-time demo, 300 MHz P-II | ~150 taps mono / ~75 stereo @ 44.1 kHz |
| FX8000 hardware | 9× 60 MHz SHARC per stereo block → 2048 taps/ch @ 50 kHz |
| Patent cost figure | 5000-tap IR @ 50 kHz ≈ 250 M MAC/s/channel |

Modern sanity check: 2048-tap direct-form dynamic convolution @ 48 kHz ≈ 400 M MAC/s —
comfortable on one SIMD core today. Full reverb-length IRs (≥1 s ≈ 48k+ taps) in direct
form are still not: ~9.4 G MAC/s. Direct-form dynamic convolution is affordable for a
**short head**, not a tail — see §7.

### 1.5 Companion dynamics work

Kemp, "Analysis and Simulation of Analogue Dynamic Compressors and Limiters in the
Digital Domain," AES 109th Convention, preprint 5185 (2000) — paywalled; algorithm
reconstructed from the corresponding patent (US 7,095,860, §6). Adds: measured gain
characteristic tables (1 kHz sine, −40→0 dBm in 1 dB steps, per ratio), empirical
attack/release capture, quasi-static IR capture at held gain-reduction operating points,
and **bilinear (j,k) runtime interpolation** across (gain-reduction set × amplitude
level) — 4 IR taps combined per output sample. Two IR-normalization conventions (gain
baked into IRs vs. normalized IRs + separate gain stage) — the decoupled form is the one
worth copying: "what the color is at this state" separated from "how loud."

### 1.6 Successors and the accuracy verdict

- **Primavera, Cecchi, Romoli, Gasparini & Piazza (EDERC 2012 + AES 133rd, 2012):**
  PCA over the amplitude-indexed IR bank (M=64 levels, 1 dB steps, 48 kHz). The L×64 IR
  matrix factors as Ĥ = V·W; keep K ≪ 64 components. Crucially the runtime realization
  is **K parallel branches of (amplitude waveshaper → static FIR)**, summed — not a
  smaller lookup bank. Validated on Aguilar bass preamp, BOSS DS-2, dbx compressor;
  MUSHRA n=10. ⚠ Secondary-source claim of K=1–3 sufficing (MSE ~2.6e-7) not confirmed
  against the paywalled primary.
- **Comunità, Steinmetz & Reiss, arXiv:2502.14405 / Frontiers Sig. Proc. 2025** (cites
  Kemp as [78], Primavera as [79]): groups Volterra series *and* dynamic convolution
  together as the classical non-differentiable black-box methods, both "sufficiently
  accurate only for weakly nonlinear systems… the same applies to dynamic convolution."
  ⚠ Verifier correction: the paper treats the two as *equally* limited — dynamic
  convolution is not framed as the fix for Volterra's limits.
- **Commercial lineage:** Sintefex FX8000/FX2000 → licensed into Focusrite Liquid
  Channel / Liquid Mix (outboard DSP). Acustica Audio Nebula ("Vectorial Volterra
  Kernels," Farina lineage §5.1) is the modern descendant, with runtime kernel
  crossfading driven by LFOs/envelope followers.

For *emulation accuracy* the literature has moved on to neural/gray-box models. For
**our** purpose — a creative level-gated reverb, not faithful device cloning — that
critique is beside the point; nobody has aimed this machinery at *spaces*.

---

## 2. The substrate: real-time partitioned convolution

The engine everything else sits on. Established results, all verified:

- **Gardner, "Efficient Convolution without Input-Output Delay," JAES 43(3):127–136
  (1995):** direct FIR head (zero latency) + geometrically growing FFT-block tail
  (N, N, 2N, 2N, 4N, …). The canonical zero-latency hybrid. (Related patent
  US 6,574,649, expired.)
- **García, "Optimal Filter Partition for Efficient Convolution with Short Input/Output
  Delay," AES 113th, paper 5660 (2002):** casts non-uniform partitioning as a Viterbi
  shortest path over states [S.Q] (block size × sub-fraction), transitions costed in
  madds (same-size block continuation = 4; new FDL of size Y = 4k·log2(2YN)+4,
  k ≈ 1.5). Worked example: 131072-tap IR, 256-sample latency → optimal 3-FDL partition
  `8×256 + 7×2048 + 7×16384` at **304 madds/sample** vs. Gardner-doubling 769, uniform
  16411. Search space 7.0e7 partitions; DP makes it trivial to re-run offline per IR
  length. Full text: `angelofarina.it/Public/AES-113/Garcia-PrePrint5660.pdf`.
- **Wefers, *Partitioned convolution algorithms for real-time auralization*, PhD thesis
  RWTH Aachen, Logos Verlag 2015 (258 pp):** the definitive treatment (OLA/OLS,
  UPOLS/NUPOLS, filter-exchange strategies §4.4/§6.11). Benchmark: 2 s RIR (88200 taps,
  44.1 kHz, B=128): UPOLS 2018 cycles/sample; minimal-load NUPOLS 204.1 (9.9×); a
  *practically schedulable* NUPOLS 240.4 (+18%, still 8.4×) that adds an intermediate
  256 segment to move work off the audio callback into background threads. UPOLS can
  absorb full-IR updates at 345 Hz (whole-IR transform ≈ 700 µs). Live PDF is
  bot-walled; fetch via Wayback copy of `publications.rwth-aachen.de/record/466561`.
- **Open source to study:** HiFi-LoFi **FFTConvolver** (C++, RT-safe, uniform +
  Gardner-style two-stage; MIT-ish) and its Rust port **neodsp/fft-convolver** — the
  natural starting reference for `rt/engine`; **jconvolver** (Adriaensen, NUP across
  priority-ranked threads); **HISSTools IR Toolbox** (ICMC 2012, `multiconvolve~`,
  BSD); **KlangFalter** (JUCE convolution reverb w/ stretch & envelope IR edits — its
  README documents *no* click-avoidance for those edits: the gap we're filling, in the
  wild); **TGM-Oldenburg/TVOLAP** (LGPL, §3.5); Csound **liveconv** (§3.4); 3DTI
  toolkit (§3.6).

**Engine-contract implication:** the audio thread must never compute an IR FFT
synchronously for large partitions. Wefers' "schedulable" partition trick + a
lock-free spectra-swap protocol is the shape of the Rust engine: FDL bank with
per-partition double-buffered spectra, background thread producing new partition
spectra, audio thread committing pointers at partition boundaries.

---

## 3. Click-free time-varying convolution: the design space (attacks L1)

### 3.1 Why it clicks (artifact taxonomy)

A convolution output is only guaranteed continuous while h is constant over the span of
the analysis frame. On a swap: **OLS** commits a hard first-order discontinuity at the
block boundary (click); **OLA** leaves stale overlap-remainder tails computed under the
old filter — Jaeger et al. 2023 show a broadband distortion burst lasting ~one IR length
before settling. Either way a step propagates through the system and smears energy
broadband ("spectral splatter"). Every technique below is a different way of paying for
smoothness.

### 3.2 Output crossfading (the classic; Lake patent)

Run old-IR and new-IR convolutions in parallel; blend outputs with complementary
envelopes `f_out + f_in = 1` (linear, or cos²/sin²). Patented by McGrath & Reilly
(Lake DSP), **US 6,421,697 B1** (filed 1999, granted 2002, now expired) with
raised-cosine fades on OLS output overlaps for head-tracked binaural. Cost: ~2× compute
during the transition (Wefers: +50–60% for OLS variants); transition length freely
choosable (8–32 samples for HRTFs, up to a full block for RIRs). ⚠ Specific embodiment
numbers from our patent fetch (N=128, O=32) unverified against claims text — mechanism
solid, digits unchecked.

### 3.3 DFT-domain crossfading (Wefers/Vorländer; independently Franck)

- **Wefers & Vorländer**, "Frequency domain filter exchange for DFT-based fast
  convolution" (AIA-DAGA 2013 p.263) → "Efficient time-varying FIR filtering using
  crossfading implemented in the DFT domain" (Forum Acusticum 2014); full derivation in
  thesis §4.4.2 (verified verbatim there — the papers themselves are access-walled).
  Constraint 2B | K, crossfade spans a full block L=B. Cyclically shift the zero-padded
  filter so valid OLS output sits at the buffer start; the discard zone then hosts a
  periodic extension of the fade envelope. A sin²/cos² envelope's K-point DFT has **3
  nonzero (real) coefficients** (bins −P, 0, +P; P = K/2B), so the crossfade becomes a
  sparse 3-tap circular convolution executed *before* the single IFFT:
  `Y(k) = (K/2)[Y0+Y1](k) + ½[(Y1−Y0)(k+P) + (Y1−Y0)(k−P)]` — 12 real ops/bin
  (3 cadd + 2 csub + 1 rmul). Saves the second IFFT: **+17–34%** during transitions
  vs. +50–60% time-domain.
- **Franck (Fraunhofer IDMT)**, "Efficient Frequency-Domain Filter Crossfading for Fast
  Convolution with Application to Binaural Synthesis," 55th AES Conf. 2014; patent
  **US 10,187,741 B2** (priority Mar 2014, granted 2019 — **in force**, ⚠ see §6).
  Independent same-mechanism work with a generalized sparse-window design method
  (constrained convex optimization over which bins get real/imag coefficients; only the
  last B samples of the window are constrained, freeing the rest). K=8 well-chosen
  coefficients ≈ dense-window accuracy; usable down to K=2–4. Benchmarks: N=512,
  B=128: ~186 → ~131 instructions/sample vs. time-domain crossfade; bigger wins when
  multiple sources sum in the frequency domain before one shared IFFT.
  ⚠ Verifier caught our fetch attributing a Brandtsegg & Saue sentence to this patent —
  corrected; the substance ("double convolver costs an extra spectral convolution +
  IFFT + crossfade ops") is accurate and appears in B&S's own wording.

### 3.4 Partition-wise incremental IR replacement — zero-cost (Brandtsegg & Saue)

**"Live Convolution with Time-Variant Impulse Response," DAFx-17 pp.239–246** (full text:
`dafx.de/paper-archive/2017/papers/DAFx17_paper_9.pdf`); journal extension *Applied
Sciences* 8(1):103 (2018). The sleeper result of this whole survey:

In a uniformly-partitioned OLA convolver, replace the IR **one partition at a time,
in load order, starting at a partition boundary** `n_T = k·N_P`. Because partition pair
(i,j) contributes to output block Y_k only if i,j ≤ k (Property 1), and to Y_{P+k} only
if i,j > k (Property 2, OLA-corrected), the convolution's own ramp-in/ramp-out does the
crossfading — the output during transition is *exactly* the mix that two overlapped
full convolutions would produce. Measured 67.2 dB SNR against that reference
(N_P=1024, N_B=2048, 1 s speech IRs @ 44.1 kHz, P≈43). **Zero added MACs, zero added
FFTs.** Costs: transition length locked to the full IR length; updates quantized to
partition boundaries; whole-buffer refill per change. Proven for OLA + uniform
partitioning; OLS and non-uniform generalizations are explicitly open questions.
Shipped as Csound `liveconv` (built on `ftconv`) — reference C source in the Csound
repo. Bonus properties we inherit: convolution can start before the IR is fully
captured/computed (latency = 1 partition), and new-IR partition FFTs spread naturally
over time instead of bursting.

### 3.5 TVOLAP — atomic switching in a WOLA-partitioned engine

**Jaeger, Simmer, Bitzer & Blau, "Time-Variant Overlap-Add in Partitions,"
arXiv:2310.00319 (2023); LGPL reference code (Python/C++/MATLAB) at
`github.com/TGM-Oldenburg/TVOLAP`; audio demos at `tgm-oldenburg.github.io/TVOLAP/`.**
Hann-windowed 50%-overlap input blocks (2L, hop L), IR in M rectangular partitions of
2L, both zero-padded to 4L; `Y(k,ℓ) = Σ_m H(k,m)·X(k,ℓ−2m)`; two-stage OLA
reconstruction. Swapping **all M partition spectra atomically** between blocks yields a
smooth transition (the analysis/synthesis windowing is the crossfade), with constant
compute — no transition-time spike. Verified numbers @ 48 kHz, N_IR=2048:

| Method | audio latency | switching latency | MFLOPS |
|---|---|---|---|
| CF-TDC (2× direct + fade) | 0 | selectable | >196.6 |
| OLA (block=N_IR) | 2048 | clicks | 6.96 |
| WOLA (block=N_IR) | 2048 | 1024 | 14.06 |
| **TVOLAP** (2L=512, M=4) | **512** | **256** | 14.59 |

Switching latency = N_IR/(2M·fs): decouples both latencies from IR length. ~2× OLA
cost (the overlap), ~13–14× cheaper than dual-convolver crossfade. BRIR-scale demo:
32768-tap IRs, 2L=1024, M=32. ⚠ Verifier note: the paper's printed WOLA formula has a
parenthesization typo (2·145+3, not 2·(145+3)); the published MFLOPS figures are correct.

### 3.6 Coefficient interpolation instead of output blending (3DTI pattern)

The 3D Tune-In Toolkit (PLOS ONE 14(3):e0211899, 2019; GitHub `3DTune-In`) gets
click-free continuously-moving HRTF rendering by **barycentrically interpolating FIR
taps among the 3 nearest measured HRIRs before each block's convolution** (ITD handled
separately). Valid whenever the IR family is *smooth in the parameter* — blend
coefficients, not outputs. This is the right template for continuous scalar knobs
(damping, level-axis position) over a precomputed IR family; it degenerates into comb
artifacts only when the interpolated IRs have misaligned reflections (see §4.2).

### 3.7 Exchange scheduling under non-uniform partitioning

Müller-Tomfelde, "Time-Varying Filter in Non-Uniform Block Convolution," DAFx-01
(origin — Wefers §6.11 reproduces it; ⚠ priority belongs to Müller-Tomfelde):
**coherent exchange** (all segments transition in one crossfade cycle; constant
transition, fluctuating response time bounded by the coarsest segment period) vs.
**asynchronous exchange** (each segment swaps at its own next execution; constant ~1
cycle response, transition can "starve" — worst case as long as the longest segment).
Power-of-two segment lengths (small LCM) bound the coherent worst case. No free lunch
between response latency and transition coherence in NUP engines — a core reason to
keep the modulatable part of the IR in the *uniform* head partitions.

### 3.8 Design-space summary

| Technique | steady-state cost | transition cost | transition length | constraint |
|---|---|---|---|---|
| Hard swap | 1× | 0 | 0 | clicks — never |
| Dual convolver + fade (§3.2) | 1× | +50–60% (2× worst) | free | patent expired; simplest |
| DFT-domain crossfade (§3.3) | 1× | +17–34% | = block | 2B\|K; Franck patent live ⚠ |
| Partition replacement (§3.4) | 1× | **+0%** | = IR length, fixed | OLA + uniform; boundary-quantized |
| TVOLAP (§3.5) | ~2× OLA always | 0 (constant) | N_IR/2M | WOLA-style engine; atomic |
| Coefficient interp (§3.6) | 1× | ~0 (per-block lerp) | continuous | IR family must be smooth |

Layered recommendation for open-conv: **partition replacement as the default swap
mechanism** on a uniform (or uniform-head NUP) engine; **coefficient interpolation** for
continuous scalar axes over smooth IR families; **DFT-domain or output crossfade** as
fallback for topology changes (IR length / partition schedule changes); TVOLAP as the
alternative engine geometry if we prefer constant-cost atomic swaps and can eat 2×.

---

## 4. Continuous "size" / stretch / morph (attacks L1's hardest case)

### 4.1 Why commercial convolvers interrupt

Altiverb's Size/Warp does genuine IR time/pitch-warping via a proprietary offline
algorithm (~50–200% range); Waves IR-1 documents Size (early-reflection acoustic
modeling), T-Time (up to 4× stretch preserving envelope/spectral trend), per-band
damping, and a direct-convolution-length CPU/latency control. All are
**recompute-the-IR-then-reload** operations — hence the interruption. Zynaptiq
Adaptiverb abandons convolution entirely (harmonic-tracking oscillator-bank resynthesis)
to get input-adaptive behavior. LiquidSonics Fusion-IR runs **multiple independently
modulated convolution engines blended continuously** from multi-sampled captures —
i.e., N-way generalized output crossfade, used for intrinsic modulation rather than a
sweepable size. (All from vendor-published material.)

### 4.2 IR warping & morphing algorithms

- **Naive interpolation between two room IRs comb-filters** — reflections at slightly
  different arrival times cancel. Kearney, Masterson & Boland (AES 35th Int. Conf.,
  2009, elib 15188): split early reflections from diffuse tail; **Dynamic Time Warping
  aligns reflections** before interpolating; treat the tail statistically
  (critical-band decorrelation).
- **Yamaha US 8,116,470 B2** (Shirakihara, filed 2009 — check status ⚠): IR
  time-stretch by overlapping Hann-windowed "base blocks," spacing block centers by
  ratio R, synthesizing interpolation blocks (averaged/time-reversed/phase-rotated
  neighbors) into the gaps — a granular stretch *of the IR itself* that avoids
  amplifying tail noise (explicitly rejects exponential-envelope multiplication for
  that reason). Runs offline → feeds §3.4's replacement pipeline.
- **Abel/Callery/Spratt US 12,361,921 B2** (priority Jul 2021 — **in force** ⚠):
  "evolving sequence of reverberation IRs" derived from input audio
  (auto-/cross-reverberation), with artifact-free transitions via panning input among
  parallel convolvers (old tails decay naturally) or constant-power output panning.
  Closest live patent to "continuously time-varying input-derived IRs" — engage with
  care (§6).

### 4.3 Alternative representations that make parameters continuous

These reproduce a measured IR in a form whose knobs modulate smoothly — escaping raw
convolution for exactly the parameters convolution can't move:

- **Modal reverb (Abel et al., CCRMA):** fit the IR as ~300–1000 damped complex
  sinusoids; each mode = complex one-pole (heterodyne→smooth→remodulate), ~1 complex
  MAC + 2 real muls/sample, ~6 samples state. Cost independent of decay length
  (a few kFLOPs/sample @ 1000 modes vs. 96k taps for a 2 s IR). Per-mode (ω, α, γ) are
  plain numbers changeable **every sample** — pitch = scale all ω; size/stretch = warp
  mode envelopes; damping = per-mode α. AES 137th (2014), DAFx-15 (stretch/pitch,
  envelope-filter tremolo suppression); patents US 9,805,704 / 10,262,645 / 11,049,482 /
  11,087,733 (Abel — status check needed ⚠); MoD-ART (arXiv 2024) extends to spatially
  non-uniform decay. Fitting a fresh user IR = damped-sinusoid estimation (roots:
  Goodwin & Vetterli's matching pursuit with damped sinusoids, ICASSP '97 / IEEE TSP
  47(7):1890–1902) — nontrivial offline work, the main engineering cost of this path.
- **Velvet-noise family (Välimäki et al., Aalto):** ternary sparse FIR (0.1–1% density)
  ⇒ multiplication-free convolution. *Filtered Velvet Noise* (Appl. Sci. 7(5):483,
  2017): segment the tail, per-segment coloration filters + sparse skeleton —
  continuously adjustable T60(f)/brightness, claimed >100× cheaper than convolution.
  *Dark Velvet Noise* (JAES 2023 / arXiv:2403.20090): time-varying pulse density
  (2000→500 pulses/s) + probabilistic routing to Q dictionary filters — decouples
  broadband decay from spectral evolution, matches a measured hall's T60(f) to 4% mean
  error with 10 filters, reproduces double-slope decays FDNs can't. DAFx-20 hybrid
  fixes FDN echo-density build-up with a velvet stage.
- **Differentiable FDN fitting:** offline gradient descent fits compact FDNs to target
  IRs — Dal Santo et al. (DAFx-23; arXiv:2402.11216), Mezza et al. (arXiv:2404.00082;
  trainable delay lengths via fractional delays, T60 errors ~2–7% on MIT IRs, N=6
  lines, ~10³ iterations); 2025 follow-ons (arXiv:2511.20380 shared-SOS efficiency;
  arXiv:2510.00238 listener-movement BRIRs). Result: a handful of physically meaningful,
  live-modulatable parameters. ⚠ One extracted FDN-vs-convolution FLOPS comparison was
  internally inconsistent; don't quote until checked against the PDF.
- Survey anchor: Välimäki, Parker, Savioja, Smith & Abel, "Fifty Years of Artificial
  Reverberation," IEEE TASLP 20(5):1421–1448 (2012).

---

## 5. Nonlinear convolution beyond Kemp (attacks L2)

### 5.1 Farina's diagonal Volterra lineage

- **Farina, "Simultaneous Measurement of Impulse Response and Distortion with a
  Swept-Sine Technique," AES 108th, paper 5093 (2000):** the ESS method.
  `x(t) = sin[ω₁L(e^{t/L}−1)]`, L = T/ln(f₂/f₁); deconvolving with the time-reversed,
  +6 dB/oct-equalized inverse yields one response with harmonic orders time-separated
  at Δt = L·ln(n).
- **Farina, Bellini & Armelloni, "Non-Linear Convolution: A New Approach for the
  Auralization of Distorting Systems," AES 110th, paper 5359 (2001)** (full text:
  `angelofarina.it/Public/Papers/154-AES110.PDF`): assume Hammerstein structure; the
  Volterra kernels collapse to diagonal 1-D kernels via sin^n identities — exact
  5th-order solution: `H1=H'1+3H'3+5H'5; H2=2jH'2+8jH'4; H3=−4H'3−20H'5; H4=−8jH'4;
  H5=16H'5`. Playback: `y = Σ_k h_k * x^k` — parallel partitioned convolutions of
  powers of the input. Real-time in 2001 on desktop CPUs; 12-subject A/B rated
  synthetic vs. real distorted recordings near-identical (1.25/5 difference).
- **Farina & Armelloni 2005** (AES Italy, paper 05014) names the two nonlinear-convolution
  families — *IR switching* (Kemp) vs. *diagonal Volterra* (Farina) — and evaluates both
  on memoryless vs. memory-bearing devices; **Farina & Farina, AES 123rd, paper 7295
  (2007)** adds runtime kernel morphing/crossfading → "not-linear, not-time-invariant
  convolver" (the Nebula architecture; up to 5th order, overlap-save partitioned).
- **Tronchin & Venturi patent US 9,171,534 B2** (Univ. Bologna, priority 2009, granted
  2015 — check status ⚠): ESS→diagonal-Volterra pipeline + Nelson–Kirkeby phase
  correction per harmonic branch.

### 5.2 Novak's synchronized swept sine / generalized Hammerstein

Novak, Simon, Kadlec & Lotton (IEEE Trans. Instrum. Meas. 2009; DAFx-09; DAFx-10
Chebyshev variant; JAES 63(10):786–798, 2015 "Synchronized Swept-Sine: Theory,
Application, and Implementation"): sweep length rounded so harmonic separations land on
exact sample boundaries; output is an N-branch generalized Hammerstein model (power or
Chebyshev nonlinearity → per-branch filter H_n via a transformation matrix). Validated
on a real overdrive pedal (fs=192 kHz, order 9, MSE 4e-5 on sine, 2e-4 on guitar).
Reference MATLAB/Python at `ant-novak.com/pages/sss/`. Identification offline; playback
trivially real-time.

### 5.3 Adaptive and analytic variants

- Pinardi et al., "Estimation of Diagonal Volterra Kernels of an Audio System During
  Normal Operation with Multiple LMS Adaptive Filters," I3DA 2023 — kernels tracked
  continuously from program material (no calibration sweep).
- Hélie, DAFx-06 — Volterra kernels derived analytically from circuit ODEs (Moog
  ladder), for when a physical model exists.
- No published work applies input-conditioned kernel generation (neural "dynamic
  convolution," Chen et al. CVPR 2020 sense) to reverb; nearest is neural IR synthesis
  per listener position (US 12,198,715). **Level-indexed room IRs is unclaimed
  territory.**

### 5.4 The load-bearing equivalence (synthesis — ours, from verified pieces)

Kemp's per-tap dynamic convolution factorizes. With triangular level-window functions
Λ_m(|x|) (width FS/M, peak 1 at level m — exactly Kemp's linear interpolation weights,
using the per-tap `p(x(n−k))` form of §1.2), define static waveshapers
`f_m(x) = x·Λ_m(|x|)`. Then:

```
y(n) = Σ_k x(n−k)·Σ_m Λ_m(|x(n−k)|)·h_m(k)  =  Σ_m ( f_m(x) * h_m )(n)
```

**Dynamic convolution ≡ a bank of M static (waveshaper → LTI convolution) branches** — a
generalized Hammerstein model with level-window nonlinearities instead of polynomials.
Consequences:

1. The "per-sample IR reselection breaks FFT convolution" objection (true of Kemp's
   direct-form view, and why the FX8000 was time-domain) **dissolves**: each branch is
   LTI, so every branch runs in a partitioned FFT convolver, and every branch composes
   with all of §3's click-free update machinery.
2. Cost is M× one convolver — so M wants to be small (3–8 perceptual level zones with
   wide overlapping windows, not Kemp's 128 measurement bins), or PCA-compressed
   (§1.6: V·W factorization = exactly this branch structure with K learned
   waveshaper/filter pairs).
3. Kemp's *envelope-follower* selector variant does **not** factorize (the selector
   depends on shared state, not the per-tap delayed sample) — that mode is genuinely
   time-varying and is served by §3's swap/crossfade machinery instead. The two
   mechanisms are complementary, not competing.
4. Farina's diagonal Volterra is the same architecture with f_m = x^m. The engine
   abstraction "N parallel (shaper → convolver) branches, summed" covers Kemp, Farina,
   Novak/Chebyshev, and PCA-reduced dynamic convolution as *presets of the shaper set*.

---

## 6. IP / freedom-to-operate notes (⚠ not legal advice; verify before release)

Expired (20-year terms elapsed; several also noted lapsed for fee non-payment):
- **US 7,039,194** (Kemp core dynamic convolution; priority 1996) — expired ~2017.
- **US 7,095,860 / WO 00/28521** (Kemp bilinear dynamics; priority 1998) — expired ~2019.
- **US 6,421,697** (Lake output-crossfade; filed 1999) — expired.
- **US 6,574,649** (Gardner) and **US 5,502,747** (McGrath FDL) — expired.
- Core papers (Gardner '95, García '02, Wefers '15, Brandtsegg & Saue '17, TVOLAP '23
  [LGPL code — reimplement from the paper for our license, or isolate]) are unencumbered
  as algorithms per se.

Likely in force — design around or scrutinize claims before shipping anything close:
- **US 10,187,741 B2** (Franck sparse-window DFT crossfade; priority 2014, ~2035).
  Note: Wefers/Vorländer's 3-coefficient sin²/cos² variant is *prior* academic work
  (AIA-DAGA 2013) — using their published method, citing it, is the conservative route.
- **US 12,361,921 B2** (Abel et al., evolving input-derived IRs + panned parallel
  convolvers; priority 2021, ~2041). Relevant if we do IR-from-input capture à la
  liveconv; the specific claimed combination is input-derived evolving IR sequences.
- **US 8,116,470** (Yamaha IR granular stretch, filed 2009) — may run to ~2029+.
- Abel modal-reverb patent family (US 9,805,704 etc.) — check claims & status if we
  build the modal tail.
- Acustica "VVKT" — commercial branding; no blocking patent surfaced in this sweep.

---

## 7. Candidate architectures for open-conv

**A. Level-gated branch convolution ("dynamic convolution for spaces") — the L2 core.**
N (≈3–8) user-loadable IRs, each assigned a level zone; input split by overlapping
smooth level-window waveshapers (§5.4) — optionally driven per-tap (Kemp-exact,
factorized) or by an attack/release envelope (Kemp's smoothed variant → becomes branch
*gains*, trivially click-free since branches are static). Each branch = one partitioned
convolver. Whisper into a cathedral, shout into a plate compressor's spring — different
rooms at different levels, crossfaded by the signal itself. Cost N×; branch IRs
swappable live via §3.4. *Novel surface: nobody has shipped level-indexed rooms.*

**B. The modulatable convolver core ("streaming IR") — the L1 core.**
Uniform-head FDL engine (García-optimal schedule offline per IR length; Wefers-style
background threads for big partitions) + **Brandtsegg–Saue partition replacement** as
the universal zero-cost update path + 3DTI-style coefficient interpolation for smooth
scalar axes + dual-engine constant-power crossfade only for partition-topology changes.
On top: a background "IR renderer" that continuously re-generates the IR under the
current knob state (Yamaha-style granular stretch for size, spectral damping EQ, etc.)
and streams partitions into the live convolver at up to ~345 Hz update rates (Wefers
§2). The IR stops being an object and becomes a *stream*. This is the engine that makes
"turn Size during playback" just work.

**C. Hybrid tail: convolved head + parametric tail.**
Keep measured early reflections + early tail as true convolution (identity of the
space); render the late tail as **dark-velvet-noise** (cheap, matches non-exponential
decay, continuously modulatable T60(f)/density) or **modal** (per-mode ω/α/γ modulation
= genuine continuous size/pitch glides with zero crossfade machinery). Splice point
~50–150 ms with energy matching. Most plausible path to *audibly continuous* size
sweeps, since §4.1 shows pure-convolution size is always recompute-and-swap under the
hood.

**D. The wacky ceiling (compose A+B+C).**
A's branch set × Kemp's bilinear second axis (branch families indexed by a macro knob
or LFO, bilinearly blended); Farina x^k branches as a "drive" bank feeding the reverb;
liveconv-style capture-the-room-from-the-input (⚠ US 12,361,921 adjacency); per-branch
streaming IRs. All of it is the same engine primitive: *parallel (shaper → updatable
partitioned convolver) branches.*

**Recommended v1:** B as the engine foundation with A (N=4 zones) as the headline
feature; C's velvet tail as the second milestone once listening tests demand continuous
size. Per the template methodology: CLI renderer + probe set first (level-staircase
probe for zone crossfades; sine bursts at zone boundaries for zipper artifacts; size
sweeps against a dual-convolver reference for transition SNR à la B&S's 67.2 dB
methodology), listening log before any plugin shell.

---

## 8. Primary sources (deep-read)

1. Kemp, *Dynamic Convolution* app note ≡ AES 106th preprint 4919 (1999). [Wayback]
2. US 7,039,194 B1 (Kemp). [Google Patents / FreePatentsOnline]
3. WO 00/28521 A1 / US 7,095,860 B1 (Kemp/Sintefex). [Google Patents]
4. Kemp, AES 109th preprint 5185 (2000). [abstract only; reconstructed via patent]
5. Primavera, Cecchi, Romoli, Gasparini, Piazza — EDERC 2012 (DOI 10.1109/EDERC.2012.6532219)
   + AES 133rd poster. [poster via SlideShare; primary paywalled]
6. Comunità, Steinmetz, Reiss — arXiv:2502.14405 / Frontiers Sig. Proc. 2025.
7. Gardner — JAES 43(3):127–136 (1995). [paywalled; content via García/Wefers ⚠ deep-read failed]
8. Wefers — PhD thesis, RWTH Aachen / Logos 2015, ISBN 978-3-8325-3943-6. [Wayback PDF]
9. García — AES 113th paper 5660 (2002). [angelofarina.it PDF]
10. Brandtsegg & Saue — DAFx-17 pp.239–246. [dafx.de PDF]
11. Wefers & Vorländer — AIA-DAGA 2013 / Forum Acusticum 2014 [access-walled; verified
    via thesis §4.4.2] + Franck US 10,187,741 B2 as accessible same-family detail.
12. Jaeger, Simmer, Bitzer, Blau — arXiv:2310.00319 (TVOLAP) + LGPL code.

**Further reading queue (surfaced, not yet deep-read):** Müller-Tomfelde DAFx-01;
Kearney et al. AES 2009 (elib 15188); Yamaha US 8,116,470; Abel modal papers
(AES 137th 2014; DAFx-15) & patents; US 12,361,921 (Abel 2021 evolving-IR);
Välimäki FVN (Appl. Sci. 7(5):483) & DVN (arXiv:2403.20090); differentiable-FDN papers
(arXiv:2402.11216, 2404.00082); Farina AES 110th PDF; Novak DAFx-10 + JAES 2015 + SSS
code; Pinardi I3DA 2023; Brandtsegg & Saue Appl. Sci. 8(1):103 (2018); FFTConvolver /
fft-convolver (Rust) / liveconv / TVOLAP / HISSTools / jconvolver / 3DTI sources;
LiquidSonics Fusion-IR & Acustica VVKT vendor docs; "Fifty Years of Artificial
Reverberation" (IEEE TASLP 2012); Yang et al. "Perceptual convolution for reverberation"
AES 115th (elib-cited by Wefers §6.13).

## 9. Verification record

66 claims adversarially checked against independent sources: 60 confirmed, 2 plausible
(exact digits locked in un-OCR'd patent figures / paywalled primary), 4 refuted. The
refutations and material corrections are marked ⚠ inline above; none reversed a
load-bearing algorithmic claim. Notable precision fixes: Comunità framing (dynamic
convolution grouped *with* Volterra as limited, not positioned as its fix);
Müller-Tomfelde priority on NUP exchange strategies; one quote misattributed to the
Franck patent (actually Brandtsegg & Saue); Sound on Sound's "127 levels" vs. the
paper's 128 (reviewer imprecision); TVOLAP WOLA-formula typesetting quirk (published
totals correct); a Kemp AES-5185 "quote" that was actually a synthesis of fragmented
patent sentences (substance correct). Full verdict log: `raw/verdicts.md`.
