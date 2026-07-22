# Paths not taken

Deferrals, not rejections. Format per template: **what / why deferred /
what hides there / re-entry notes**. Research references point into
`docs/research/01-prior-art.md`.

## 1. TVOLAP engine geometry

- **What:** Building the convolver as TVOLAP (Jaeger et al. 2023, §3.5) —
  Hann-windowed 50%-overlap partitioned engine with atomic all-partition IR
  switching, constant compute, switching latency `N_IR/(2M·fs)`.
- **Why deferred:** Steady-state cost is ~2× plain OLA *always* (the 50%
  overlap), paid whether or not you ever switch. Brandtsegg–Saue stepwise
  replacement gives click-free swaps at +0% on the engine we need anyway.
  TVOLAP's win — transition length decoupled from IR length — matters when
  you need *fast* full-IR swaps; our streaming-IR design wants gradual
  transitions.
- **What hides there:** Atomic swaps make hard scene changes (preset jumps,
  MIDI-triggered room switches) tighter than a tail-length fade. LGPL
  reference code (GPL-compatible for us) exists in three languages.
- **Re-entry:** If listening finds tail-length transitions musically sluggish
  for preset morphs, add a TVOLAP mode or a bounded dual-engine crossfade
  (see #3) for the head partitions only.

## 2. Kemp-faithful direct-form dynamic-convolution head

- **What:** A short (1–4k tap) time-domain per-tap dynamic convolution stage
  (Kemp §1.2 exactly, 2 MACs/tap) for the early reflections, dense 32–128
  level ladder, in front of the partitioned tail.
- **Why deferred:** The branch factorization (§5.4) makes the ≤4-zone version
  of the same idea free inside the FFT engine; the dense-ladder character
  difference is speculative until we can A/B it. ~400 M MAC/s is affordable
  but not free.
- **What hides there:** The *exact* Sintefex/Focusrite texture on transients;
  per-tap selection at 128 levels may sound importantly different from 4-zone
  triangular windows on percussive material. Also sign-dependent IR sets
  (Kemp's proposed ±impulse ladders).
- **Re-entry:** Implement as an optional `head` module in the engine; A/B via
  the `impulses` probe; the IR-ladder synthesis (interpolating user IRs into
  a ladder) is the main design question.

## 3. DFT-domain / dual-convolver crossfading as the primary update path

- **What:** Wefers–Vorländer 3-coefficient DFT-domain crossfade (§3.3) or
  classic dual-convolver output crossfade (§3.2) for IR changes.
- **Why deferred:** Stepwise replacement costs nothing and its fixed
  tail-length transition is the *desired* sound for a streaming IR. The
  crossfade family costs +17–60% during transitions and needs a second
  spectra set resident. Franck's patent (US 10,187,741, in force) sits near
  the sparse-window generalization; Wefers' published 3-coefficient variant
  is the safe prior art if we ever need it.
- **What hides there:** User-tunable transition *time* (replacement can't
  shorten below tail length); needed for partition-topology changes (see #7)
  and possibly preset jumps.
- **Re-entry:** Implement Wefers' cos² 3-bin variant exactly as published
  (thesis §4.4.2, eq. 4.50); cite it; keep transition length = one block.

## 4. Full Volterra / Hammerstein nonlinear branches

- **What:** Farina diagonal-Volterra kernels (xᵏ branches, §5.1) or Novak
  synchronized-swept-sine Hammerstein identification (§5.2) as the
  level-dependence mechanism.
- **Why deferred:** Those model *measured device* nonlinearity; our v1 L2
  feature is *creative* level-gating of user-chosen rooms — zones, not
  harmonic-order kernels. Also the literature verdict (§1.6): weakly
  nonlinear accuracy only.
- **What hides there:** "Drive" as a reverb dimension — convolving powers of
  the input with different IRs is an unexplored effect (distortion that
  *spatializes* instead of clipping). ESS measurement would also let users
  sample real nonlinear spaces/gear into the zone ladder.
- **Re-entry:** The branch abstraction already fits (`f_m = xᵏ` is just
  another shaper preset). Add a `shaper: ZoneWindow | Power(k)` enum per
  branch; Novak's reference code at ant-novak.com for measurement.

## 5. Gardner zero-latency hybrid head

- **What:** Direct-form FIR for the first partition (zero I/O latency) à la
  Gardner 1995 / HISSTools / FFTConvolver's two-stage scheme.
- **Why deferred:** Reverb wet signals tolerate P=256 (5.3 ms, honestly
  reported; hosts PDC-compensate). Zero-latency quadruples head complexity
  (per-branch direct convolution × 4 zones) for a benefit that matters most
  to live monitoring.
- **What hides there:** Live-performance use (the Brandtsegg live-convolver
  use case); "Direct Convolution Length" as a user CPU/latency trade à la
  Waves IR-1.
- **Re-entry:** Add a direct head per branch of length P; the shaped inputs
  already exist, so it's a contained module.

## 6. Dense level ladders via PCA compression

- **What:** Primavera 2012 (§1.6): PCA over an M-level IR ladder → K
  waveshaper×FIR branches; dense Kemp-style ladders at K× convolver cost.
- **Why deferred:** Needs a ladder to compress — v1 zones are user-loaded
  discrete IRs, not measured ladders. ⚠ headline K=1–3 / MSE figures remain
  unverified (paywalled primary).
- **What hides there:** "Measure a real space at 64 drive levels through a
  driven speaker, PCA it, play it back exactly" — the *authentic* nonlinear
  room. Probably the most scientifically interesting future feature.
- **Re-entry:** Offline PCA in the lab (numpy SVD on the L×M IR matrix);
  runtime is literally the existing branch engine with learned shaper curves.

## 7. García-optimal / non-uniform partitioning

- **What:** Viterbi-optimal FDL partition schedules (§2, 304 vs 2102 madds
  in his example) instead of uniform P=256.
- **Why deferred:** B&S replacement is proven for *uniform* OLA; NUP filter
  exchange has the coherent-vs-async transition trade (Müller-Tomfelde,
  §3.7) — extra design surface before the sound exists. CPU headroom at v1
  scale (4 branches × 3 s IRs) doesn't demand it.
- **What hides there:** ~8–10× tail-CPU reduction (Wefers §2 numbers) —
  matters for 8-zone banks, long cathedral IRs, or laptop-battery budgets.
  García's DP is also the right tool when `size` changes IR length and the
  schedule must re-solve.
- **Re-entry:** Keep the uniform head (all modulation machinery lives
  there); add coarse background FDLs for the static tail; extend the
  replacement proof to the segment boundaries (B&S flag NUP generalization
  as open — small research contribution available here).

## 8. Overlap-save core

- **What:** OLS instead of OLA (Wefers: OLS is the cheaper primitive).
- **Why deferred:** The B&S replacement proof (Properties 1–2) is derived
  for OLA — they explicitly leave OLS as future work. Correctness proof
  before efficiency.
- **What hides there:** A few % CPU; alignment with most textbook UPOLS
  implementations.
- **Re-entry:** Re-derive the two contribution properties for OLS in the
  lab; if the residual-vs-reference SNR matches exp02, switch.

## 9. Pitch-invariant size (granular IR stretch)

- **What:** Yamaha US 8,116,470-style block-granular IR time-stretch
  (§4.2), or STFT-domain stretch, instead of v1's plain resampling
  (pitch-coupled).
- **Why deferred:** Resampling is the classic "size" sound and trivially
  correct; granular stretch has tuning surface (block size, phase
  rotation) and a possibly-live patent to design around.
- **What hides there:** Independent size and pitch knobs; stretch >2×
  without chipmunk tails; the noise-floor-preserving trick.
- **Re-entry:** Implement in the lab first against the `sweepbed` probe;
  read the actual Yamaha claims before shipping anything similar.

## 10. Parametric tail (modal / dark velvet noise / fitted FDN)

- **What:** Research §7-C: convolved head + continuously-modulatable
  parametric tail (modal per-mode ω/α/γ; DVN density/dictionary; fitted
  FDN).
- **Why deferred:** It's M4 by plan — the streaming convolver must exist
  first as both the identity of the product and the reference the
  parametric tail is fitted against. Modal fitting (damped-sinusoid
  estimation) is real offline engineering; Abel patent family status
  unchecked.
- **What hides there:** True continuous size/damping glides with zero
  transition machinery — the only architecture where "Size" is a *filter
  coefficient*, not a re-render. Probably v2's headline.
- **Re-entry:** Start with DVN (simplest, JAES 2023 numbers strong, no
  patent flag surfaced); fit T60(f) from the loaded IR automatically;
  splice at ~80 ms with energy matching; A/B against pure convolution on
  the same IR. (2026-07-22: user reviewed and deferred — "hit the other
  three first [XY blend, damp, drive], then we'll revisit." Wants to
  avoid algorithmic-reverb pitfalls; the lab A/B gate is the answer.)
- **Phase-1 results (2026-07-22, batch 009, `lab/exp04_tail_fit.py`):**
  STFT-noise tail fits were A/B'd on five IRs (noise-like, tonal boom,
  three user samples). Verdict: audibly distinct — fit-floor leakage
  reverberates content outside the true IR spectrum ("algorithmic-reverb
  highend"; user *liked* it on boom+loop), random-phase resynthesis
  blurs HF microstructure (press "lacks crisp high-end fidelity"),
  sparse material exposes the naked tail (murky: orig on kicks, hybrid
  on loop). Deferred again by user. **Re-entry shape:** likely a
  *blend dial* (sample-true ↔ resynth tail; Freeze/instant-Decay unlock
  at the parametric end) rather than a transparency claim — the user
  preferred the error twice. Before engine work: fix fit-floor gating
  (kills the leak) and HF handling; the boom case wants the modal route
  if tonal tails ever need to pass.

## 11. Reusing LGPL reference code verbatim

- **What:** Porting/vendoring TVOLAP (LGPL) or Csound `liveconv` (LGPL)
  directly rather than implementing from the papers.
- **Why deferred:** Even though GPL-3.0 lets us, the engine wants to be
  born to the template's no-alloc/streaming contract, not adapted to
  someone else's buffer topology. Papers are implementation-grade.
- **What hides there:** Battle-tested edge cases (liveconv's concurrent
  load/unload process bookkeeping is subtle).
- **Re-entry:** Consult (GPL-compatible) when our exp02 SNR falls short of
  67 dB or concurrent-swap bookkeeping grows hairy.

## 12. Webview GUI (WRAC's native mode)

- **What:** The WRAC template's webview panel.
- **Why deferred (pre-decided):** Template §7: panel-native from day one;
  delete `wrac-gain`. Inherited as project law.
- **Re-entry:** None foreseen.
