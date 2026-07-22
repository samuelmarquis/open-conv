//! open-conv engine: a convolution reverb past the limits of convolution.
//!
//! Two ideas, one engine (see `docs/design/01-architecture.md` and
//! `docs/research/01-prior-art.md`):
//!
//! 1. **Level-gated branches** (dynamic convolution for *spaces*): Kemp's
//!    per-tap dynamic convolution factorizes exactly into a bank of static
//!    branches `y = Σ_z (f_z(x) * h_z)` where `f_z(x) = x · w_z(|x|)` are
//!    level-window waveshapers. Each branch is LTI, so full reverb-length
//!    IRs run in partitioned FFT convolution — different rooms at different
//!    input levels, crossfaded by the signal itself.
//! 2. **The IR is a stream, not an object**: all IR changes (loads, size
//!    retargets, future damping re-renders) go through stepwise partition
//!    replacement (Brandtsegg & Saue, DAFx-17): one partition per block,
//!    in load order — mathematically equivalent to a dual-convolver
//!    crossfade at zero added cost. Click-free by construction.
//!
//! Convolver core: uniform partitioned overlap-add (UPOLA), partition `P`
//! (default 256), FFT size `2P`, frequency-domain delay line per
//! (zone, channel). Reported latency = `P` samples.
//!
//! ## Threading contract
//!
//! - [`Engine::process_block`] is the RT path: allocation-free, lock-free.
//!   All rings/scratch are preallocated for `max_ir_seconds` at
//!   construction.
//! - [`Engine::set_source_ir`], [`Engine::service`],
//!   [`Engine::render_partition_set`] and [`Engine::take_retired`] are
//!   control-path (they allocate / drop). A plugin shell runs them on a
//!   worker and hands finished [`PartitionSet`]s to the audio thread via
//!   [`Engine::queue_partition_set`], which only moves memory (RT-safe);
//!   the completed/rejected sets it returns must be dropped off-thread.
//!   The offline CLI just calls everything on one thread.

pub mod banks;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::VecDeque;
use std::sync::Arc;

pub const MAX_ZONES: usize = 4;
pub const DEFAULT_PARTITION: usize = 256;
pub const DEFAULT_MAX_IR_SECONDS: f64 = 8.0;
const VIZ_CAP: usize = 16;
const SILENCE_DB: f64 = -160.0;

/// What happens to the outgoing IR when a new one arrives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TailMode {
    /// Streaming replacement (B&S) in the shared H bank: the old room's
    /// response to *future* input is progressively replaced; its response
    /// to past input completes naturally. One voice per zone.
    Gated,
    /// Parallel voice epochs: each arriving IR *freezes* the current
    /// voice — it stops hearing new input but rings out its full tail —
    /// and starts a fresh voice on input from now on. Exact input-split
    /// convolution (the parallel-convolver architecture); click-free by
    /// construction. Up to [`EngineParams::ring`] frozen voices ring per
    /// zone (capacity [`RING_SLOTS`]); on overflow the oldest fades out
    /// over ~50 ms to make room. All voices share the zone's
    /// input-spectra ring, gated by epoch age — a ringing voice costs
    /// only its adopted spectra + a tail buffer.
    Ungated,
}

/// How the zone selector reads the input level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LevelMode {
    /// Kemp's per-sample selector: weights follow the instantaneous
    /// rectified sample (per channel). Zone crossings happen at audio rate
    /// — the waveshaper-like "dynamic convolution" character.
    Instant,
    /// Kemp's/US7095860's smoothed selector: an asymmetric attack/release
    /// follower (shared across channels) drives the weights. The room
    /// follows the *dynamics*, with no waveshaping color.
    Envelope,
}

/// Flat parameter block — one field per knob; shells translate into this
/// per block (template contract). `zone_db` must be ascending.
#[derive(Clone, Copy, Debug)]
pub struct EngineParams {
    /// Active zone count, 1..=MAX_ZONES. 1 = ordinary convolver.
    pub n_zones: usize,
    /// Zone centers in dBFS (ascending). Triangular interpolation in dB
    /// between adjacent centers; extremes own everything beyond them.
    pub zone_db: [f64; MAX_ZONES],
    /// Per-zone wet gain (linear).
    pub zone_gain: [f64; MAX_ZONES],
    pub level_mode: LevelMode,
    /// Envelope-mode attack, milliseconds.
    pub attack_ms: f64,
    /// Envelope-mode release, milliseconds.
    pub release_ms: f64,
    /// Wet gain (linear).
    pub wet: f64,
    /// Dry gain (linear); dry path is latency-aligned with the wet.
    pub dry: f64,
    /// IR stretch ratio (resampling, pitch-coupled — the classic "size").
    /// Changes are honored by [`Engine::service`] via partition streaming.
    pub size: f64,
    /// Wet soft-saturation drive (tanh(w·d)/d). 0 (default) = fully
    /// linear — float headroom carries hot wet cleanly; trim Wet instead.
    /// >0 = deliberate saturation color (1 ≈ transparent below −12 dBFS).
    pub sat: f64,
    /// Symmetry, 0..1: on negative half-cycles the zone ladder is blended
    /// toward its mirror (zone z ↔ n_zones−1−z). 0 = off; 1 = full
    /// cross-fire (the former "xsign" mode). A sound-design bend of
    /// Kemp's proposed ± IR sets: even-order waveshaping whose harmonics
    /// come out *spatialized*. Applies per-sample in both level modes.
    pub sym: f64,
    /// IR transition speed: partitions replaced per frame during a
    /// streaming swap, 1..=16. 1 = strict B&S (transition = tail length,
    /// the luxurious glide); higher = proportionally faster morphs (~16 ⇒
    /// a 3 s IR swaps in ~190 ms @ P=256/48k) — still click-free (each
    /// frame changes a bounded slice of H), just a denser crossfade.
    /// Turn up when automating IR-changing params per-hit.
    pub morph: f64,
    /// Per-partition write fade in frames, 1..=MAX_FADE (Gated streaming
    /// only). 1 = hard per-partition steps (the sharp "skitter" edge);
    /// 4 ≈ 21 ms fades (default); 16 ≈ 85 ms rounded-off writes.
    pub fade_frames: f64,
    /// Old-IR policy on arrival of a new one — see [`TailMode`].
    pub tails: TailMode,
    /// Ring-out voices kept per zone in Ungated mode, 1..=RING_SLOTS
    /// (default: max). History depth, not bank size. Not exposed in the
    /// plugin (capacity is the behavior; CPU scales with actual ringing
    /// voices, i.e. with switch rate) — kept as a lab dial for CLI
    /// experiments. Safe to change during playback.
    pub ring: f64,
}

impl Default for EngineParams {
    fn default() -> Self {
        Self {
            n_zones: MAX_ZONES,
            zone_db: [-48.0, -30.0, -18.0, -6.0],
            zone_gain: [1.0; MAX_ZONES],
            level_mode: LevelMode::Envelope,
            attack_ms: 5.0,
            release_ms: 120.0,
            wet: 0.35,
            dry: 1.0,
            size: 1.0,
            sat: 0.0,
            sym: 0.0,
            morph: 1.0,
            fade_frames: 4.0,
            tails: TailMode::Gated,
            ring: RING_SLOTS as f64,
        }
    }
}

/// One analysis frame for the viz feed (CLI `--viz-dump` JSONL and the
/// future panel consume the same frames). Fixed-capacity ring, publish
/// drops on overflow — a skipped display column, never a glitch.
#[derive(Clone, Copy, Debug)]
pub struct VizFrame {
    /// Engine time at frame end, samples since construction/reset.
    pub t: u64,
    /// Peak input level in the frame, dBFS.
    pub in_peak_db: f32,
    /// Envelope follower state at frame end, dBFS.
    pub env_db: f32,
    /// Mean zone weights over the frame (channel 0 / shared env).
    pub weights: [f32; MAX_ZONES],
    /// Per-zone wet RMS over the frame (linear, pre zone_gain/wet).
    pub zone_energy: [f32; MAX_ZONES],
    /// 0..1 progress of the most-active partition swap (1 = idle/done).
    pub swap_progress: f32,
}

/// A fully rendered, FFT'd IR ready for streaming: `k` partitions of
/// spectra per IR channel. Built on the control path (allocates); consumed
/// by the RT path by move only.
pub struct PartitionSet {
    k: usize,
    ch: usize,
    bins: usize,
    /// Layout: `[(c * k + part) * bins + bin]`.
    spectra: Vec<Complex<f32>>,
    /// The `size` this set was rendered at.
    rendered_size: f64,
}

impl PartitionSet {
    pub fn partitions(&self) -> usize {
        self.k
    }
    pub fn rendered_size(&self) -> f64 {
        self.rendered_size
    }
}

/// Control-path IR → [`PartitionSet`] renderer, decoupled from [`Engine`]
/// so a plugin worker thread can own one (it allocates; never construct or
/// call on the audio thread). Obtain via [`Engine::renderer`] to guarantee
/// matching geometry, or [`IrRenderer::new`] with identical arguments.
pub struct IrRenderer {
    sr: f64,
    part: usize,
    bins: usize,
    max_parts: usize,
}

impl IrRenderer {
    /// `partition`/`max_ir_seconds` must match the target engine's
    /// construction arguments (else [`Engine::queue_partition_set`]
    /// rejects the sets).
    pub fn new(sr: f64, partition: usize, max_ir_seconds: f64) -> Self {
        Self {
            sr,
            part: partition,
            bins: partition + 1,
            max_parts: max_parts_for(sr, partition, max_ir_seconds),
        }
    }

    /// Resample (linear, pitch-coupled stretch, 1/√size energy
    /// compensation), partition, FFT. `data`: one Vec per IR channel.
    pub fn render(&self, data: &[Vec<f32>], ir_sr: f64, size: f64) -> PartitionSet {
        let size = size.clamp(0.25, 4.0);
        let ratio = (self.sr / ir_sr) * size;
        let ch = data.len();
        let src_len = data[0].len();
        let out_len = ((src_len as f64 * ratio) as usize)
            .max(1)
            .min(self.max_parts * self.part);
        let k = out_len.div_ceil(self.part);
        let bins = self.bins;
        let norm = 1.0 / (size.sqrt());
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(2 * self.part);
        let mut scratch = fft.make_scratch_vec();
        let mut spectra = vec![Complex::new(0.0f32, 0.0); ch * k * bins];
        let mut time = vec![0.0f32; 2 * self.part];
        for c in 0..ch {
            let d = &data[c];
            for part in 0..k {
                time.fill(0.0);
                for i in 0..self.part {
                    let n = part * self.part + i;
                    if n >= out_len {
                        break;
                    }
                    let pos = n as f64 / ratio;
                    let i0 = pos as usize;
                    if i0 + 1 < src_len {
                        let f = (pos - i0 as f64) as f32;
                        time[i] = (d[i0] * (1.0 - f) + d[i0 + 1] * f) * norm as f32;
                    } else if i0 < src_len {
                        time[i] = d[i0] * norm as f32;
                    }
                }
                let out = &mut spectra[(c * k + part) * bins..][..bins];
                fft.process_with_scratch(&mut time, out, &mut scratch)
                    .expect("ir fft");
            }
        }
        PartitionSet {
            k,
            ch,
            bins,
            spectra,
            rendered_size: size,
        }
    }
}

fn max_parts_for(sr: f64, partition: usize, max_ir_seconds: f64) -> usize {
    ((sr * max_ir_seconds) as usize).div_ceil(partition).max(2)
}

/// Source IR audio as loaded/generated, kept for re-rendering at new sizes.
struct SourceIr {
    /// One Vec per IR channel (1 = mono, applied to all engine channels).
    data: Vec<Vec<f32>>,
    sr: f64,
}

/// Max per-partition write-fade frames (`EngineParams::fade_frames` caps
/// here). Each written partition fades to target over `fade` frames
/// instead of stepping in one — the batch-007 "skitter" softener.
pub const MAX_FADE: usize = 16;
/// Max ring-out voices per zone in [`TailMode::Ungated`] (capacity;
/// [`EngineParams::ring`] sets the active depth).
pub const RING_SLOTS: usize = 8;
const RETIRED_SLOTS: usize = 12;

struct Pending {
    /// `None` = fade the H voice out to silence (Ungated bootstrap).
    set: Option<PartitionSet>,
    /// Next partition index to start blending (B&S load-order cursor).
    cursor: usize,
    /// max(old active_k, new k): partitions in [set.k, eff_k) are zeroed
    /// ("unloaded") as the cursor passes them.
    eff_k: usize,
    /// Cursor history: `hist[s]` = cursor position s frames ago. The range
    /// `[hist[s+1], hist[s])` is at blend stage s+1.
    hist: [usize; MAX_FADE + 1],
}

/// One input-epoch voice ([`TailMode::Ungated`]): an adopted IR that hears
/// only its epoch's slice of the shared input ring.
struct LayerVoice {
    set: PartitionSet,
    /// Live voice: frames since epoch start — hears lags `0..=age` (never
    /// reaching pre-epoch input). Frozen voice: frames since freeze (m) —
    /// hears lags `m..min(k, m + epoch_len)` (its own epoch's input only);
    /// dead (and retired) once `m >= k`.
    age: usize,
    /// Epoch length in frames, fixed at freeze (0 while live).
    epoch_len: usize,
    /// Output gain; ramps ×0.5/frame once `dying` (graceful eviction —
    /// hard-cutting a hot ring voice was the ungated "DC clicks" bug).
    gain: f32,
    dying: bool,
    /// OLA tail per channel (P samples), pooled.
    tail: Vec<Vec<f32>>,
}

struct Branch {
    /// Active IR spectra: `[ch][max_parts * bins]`, zero beyond
    /// `active_k` (invariant relied on during swaps that grow k).
    h: Vec<Vec<Complex<f32>>>,
    active_k: usize,
    /// Which source-IR channel feeds engine channel c: `min(c, ir_ch-1)`.
    ir_ch: usize,
    /// Rendered-size bookkeeping for `service`.
    rendered_size: f64,
    /// Ring of past shaped-input spectra: `[ch][max_parts * bins]`.
    x_ring: Vec<Vec<Complex<f32>>>,
    /// Ring slot holding the most recent frame's spectrum.
    head: usize,
    /// Overlap-add tail, `[ch][P]`.
    tail: Vec<Vec<f32>>,
    pending: Option<Pending>,
    /// Ungated voices: the live epoch + frozen ring-outs.
    live: Option<LayerVoice>,
    ringing: [Option<LayerVoice>; RING_SLOTS],
    /// Spare tail buffers for voices (bounded: live + rings), zeroed on take.
    tail_pool: Vec<Vec<Vec<f32>>>,
    /// Spent sets awaiting off-thread disposal (drain via `take_retired`).
    retired: [Option<PartitionSet>; RETIRED_SLOTS],
}

impl Branch {
    /// Park a spent set for off-thread disposal. Callers drain every
    /// block; capacity is sized well above per-frame worst case. If
    /// somehow full, the set leaks into the newest slot's place — never
    /// dropped on the RT thread.
    fn retire_push(&mut self, set: PartitionSet) {
        for slot in &mut self.retired {
            if slot.is_none() {
                *slot = Some(set);
                return;
            }
        }
        // Full: replace slot 0 (control path failed to drain for many
        // frames). The displaced set is dropped here as a last resort.
        self.retired[0] = Some(set);
    }

    /// Freeze the live voice into a ring slot (it stops hearing input and
    /// rings out). Slot placement prefers: free slot → quietest *dying*
    /// voice (already faded ≈ inaudible) → oldest voice (last resort;
    /// rapid-switch storms should be absorbed by the proactive marking
    /// below before this can hard-cut anything hot).
    fn freeze_live(&mut self, cap: usize) {
        let cap = cap.clamp(1, RING_SLOTS);
        let Some(mut v) = self.live.take() else {
            return;
        };
        v.epoch_len = v.age; // frames of input this epoch heard
        v.age = 1; // next frame is 1 frame post-freeze
        let mut free = None;
        let mut quietest_dying: Option<(usize, f32)> = None;
        let mut oldest = (0usize, 0usize); // (slot, age)
        for (i, slot) in self.ringing.iter().take(cap).enumerate() {
            match slot {
                None => {
                    free = Some(i);
                    break;
                }
                Some(r) => {
                    if r.dying && quietest_dying.is_none_or(|(_, g)| r.gain < g) {
                        quietest_dying = Some((i, r.gain));
                    }
                    if r.age >= oldest.1 {
                        oldest = (i, r.age);
                    }
                }
            }
        }
        let i = free
            .or(quietest_dying.map(|(i, _)| i))
            .unwrap_or(oldest.0);
        if let Some(evicted) = self.ringing[i].take() {
            self.tail_pool.push(evicted.tail);
            self.retire_push(evicted.set);
        }
        self.ringing[i] = Some(v);
        // Proactive: with no free slots left (within cap), start fading
        // the oldest healthy voice now so the *next* freeze finds a quiet
        // victim. (age > 1 excludes the voice frozen just above.)
        if self.ringing.iter().take(cap).all(|r| r.is_some()) {
            if let Some(r) = self
                .ringing
                .iter_mut()
                .take(cap)
                .flatten()
                .filter(|r| !r.dying && r.age > 1)
                .max_by_key(|r| r.age)
            {
                r.dying = true;
            }
        }
    }
}

impl Branch {
    fn swap_progress(&self) -> f32 {
        match &self.pending {
            Some(p) if p.eff_k > 0 => p.cursor as f32 / p.eff_k as f32,
            _ => 1.0,
        }
    }
}

pub struct Engine {
    sr: f64,
    channels: usize,
    part: usize,
    bins: usize,
    max_parts: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    ifft_scratch: Vec<Complex<f32>>,
    branches: Vec<Branch>,
    sources: Vec<Option<SourceIr>>,
    /// Input accumulator, `[ch][P]`.
    in_fifo: Vec<Vec<f32>>,
    fifo_fill: usize,
    /// Output FIFO, primed with `P` zeros (= reported latency).
    out_fifo: Vec<VecDeque<f32>>,
    /// Shaped branch inputs, `[zone][ch][P]`.
    shaped: Vec<Vec<Vec<f32>>>,
    /// Per-frame wet accumulator, `[ch][P]`.
    wet: Vec<Vec<f32>>,
    /// FFT/IFFT scratch buffers.
    time_buf: Vec<f32>,
    acc_buf: Vec<Complex<f32>>,
    out_time: Vec<f32>,
    env: f64,
    t: u64,
    viz: VecDeque<VizFrame>,
}

impl Engine {
    pub fn new(sr: f64, channels: usize) -> Self {
        Self::new_sized(sr, channels, DEFAULT_PARTITION, DEFAULT_MAX_IR_SECONDS)
    }

    /// `partition` must be a power of two ≥ 32. `max_ir_seconds` bounds the
    /// preallocated spectra rings (post-stretch IR length is clamped to it).
    pub fn new_sized(sr: f64, channels: usize, partition: usize, max_ir_seconds: f64) -> Self {
        assert!(partition.is_power_of_two() && partition >= 32);
        assert!(channels >= 1);
        let bins = partition + 1; // FFT size 2P → P+1 complex bins
        let max_parts = max_parts_for(sr, partition, max_ir_seconds);
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(2 * partition);
        let ifft = planner.plan_fft_inverse(2 * partition);
        let fft_scratch = fft.make_scratch_vec();
        let ifft_scratch = ifft.make_scratch_vec();
        let zero_spec = vec![Complex::new(0.0f32, 0.0); max_parts * bins];
        let branches = (0..MAX_ZONES)
            .map(|_| Branch {
                h: vec![zero_spec.clone(); channels],
                active_k: 0,
                ir_ch: 1,
                rendered_size: 1.0,
                x_ring: vec![zero_spec.clone(); channels],
                head: 0,
                tail: vec![vec![0.0; partition]; channels],
                pending: None,
                live: None,
                ringing: std::array::from_fn(|_| None),
                tail_pool: {
                    let mut p = Vec::with_capacity(RING_SLOTS + 2);
                    for _ in 0..RING_SLOTS + 1 {
                        p.push(vec![vec![0.0; partition]; channels]);
                    }
                    p
                },
                retired: std::array::from_fn(|_| None),
            })
            .collect();
        let mut out_fifo = vec![VecDeque::with_capacity(2 * partition + 1); channels];
        for f in &mut out_fifo {
            for _ in 0..partition {
                f.push_back(0.0);
            }
        }
        Self {
            sr,
            channels,
            part: partition,
            bins,
            max_parts,
            fft,
            ifft,
            fft_scratch,
            ifft_scratch,
            branches,
            sources: (0..MAX_ZONES).map(|_| None).collect(),
            in_fifo: vec![vec![0.0; partition]; channels],
            fifo_fill: 0,
            out_fifo,
            shaped: vec![vec![vec![0.0; partition]; channels]; MAX_ZONES],
            wet: vec![vec![0.0; partition]; channels],
            time_buf: vec![0.0; 2 * partition],
            acc_buf: vec![Complex::new(0.0, 0.0); bins],
            out_time: vec![0.0; 2 * partition],
            env: 0.0,
            t: 0,
            viz: VecDeque::with_capacity(VIZ_CAP),
        }
    }

    pub fn latency_samples(&self) -> usize {
        self.part
    }

    pub fn partition(&self) -> usize {
        self.part
    }

    /// Longest active tail in samples (max over zones of active_k · P).
    pub fn tail_samples(&self) -> usize {
        self.branches
            .iter()
            .map(|b| {
                let k = match &b.pending {
                    Some(p) => p.eff_k.max(b.active_k),
                    None => b.active_k,
                };
                k * self.part
            })
            .max()
            .unwrap_or(0)
    }

    /// Clears all signal state (rings, tails, FIFOs, envelope). Keeps
    /// loaded IRs; an in-flight partition swap is completed instantly
    /// (reset is already a discontinuity). Not RT-cheap: zeroes the rings.
    pub fn reset(&mut self) {
        let channels = self.channels;
        let bins = self.bins;
        for b in &mut self.branches {
            // Fast-forward pending swap.
            if let Some(p) = b.pending.take() {
                let k_new = p.set.as_ref().map(|s| s.k).unwrap_or(0);
                for c in 0..channels {
                    // From 0, not cursor: finalize any partially blended
                    // partitions too (reset is a discontinuity anyway).
                    for part in 0..p.eff_k {
                        let dst = &mut b.h[c][part * bins..(part + 1) * bins];
                        if part < k_new {
                            let s = p.set.as_ref().unwrap();
                            let src_c = c.min(s.ch - 1);
                            dst.copy_from_slice(
                                &s.spectra[(src_c * k_new + part) * bins..][..bins],
                            );
                        } else {
                            dst.fill(Complex::new(0.0, 0.0));
                        }
                    }
                }
                b.active_k = k_new;
                if let Some(s) = p.set {
                    b.rendered_size = s.rendered_size;
                    b.retire_push(s);
                }
            }
            // Epoch voices: reset is a discontinuity — retire them all.
            if let Some(v) = b.live.take() {
                b.tail_pool.push(v.tail);
                b.retire_push(v.set);
            }
            for i in 0..RING_SLOTS {
                if let Some(v) = b.ringing[i].take() {
                    b.tail_pool.push(v.tail);
                    b.retire_push(v.set);
                }
            }
            for c in 0..channels {
                b.x_ring[c].fill(Complex::new(0.0, 0.0));
                b.tail[c].fill(0.0);
            }
            b.head = 0;
        }
        for f in &mut self.in_fifo {
            f.fill(0.0);
        }
        self.fifo_fill = 0;
        for f in &mut self.out_fifo {
            f.clear();
            for _ in 0..self.part {
                f.push_back(0.0);
            }
        }
        self.env = 0.0;
        self.t = 0;
        self.viz.clear();
    }

    pub fn viz_pop(&mut self) -> Option<VizFrame> {
        self.viz.pop_front()
    }

    // ---------------------------------------------------------------
    // Control path (allocates; never call from the audio thread)
    // ---------------------------------------------------------------

    /// Load/replace the source IR for a zone. `data`: one Vec per IR
    /// channel (mono broadcasts). Renders at `size` and either installs
    /// immediately (branch was empty) or streams via partition replacement.
    pub fn set_source_ir(&mut self, zone: usize, data: Vec<Vec<f32>>, ir_sr: f64, size: f64) {
        assert!(zone < MAX_ZONES);
        assert!(!data.is_empty() && !data[0].is_empty());
        let src = SourceIr { data, sr: ir_sr };
        let set = self.render_partition_set(&src, size);
        self.sources[zone] = Some(src);
        let b = &mut self.branches[zone];
        b.ir_ch = set.ch;
        if b.active_k == 0 && b.pending.is_none() {
            // Fresh branch: direct install.
            let bins = self.bins;
            for c in 0..self.channels {
                let src_c = c.min(set.ch - 1);
                for part in 0..set.k {
                    b.h[c][part * bins..(part + 1) * bins]
                        .copy_from_slice(&set.spectra[(src_c * set.k + part) * bins..][..bins]);
                }
            }
            b.active_k = set.k;
            b.rendered_size = set.rendered_size;
        } else {
            // Control path: rejected/retired sets all drop here. Loads
            // always stream via the Gated path (creative tail modes apply
            // to *changes*, not first installs).
            if let Err(set) = self.queue_partition_set(zone, set, &EngineParams::default()) {
                drop(set);
            }
            while self.take_retired(zone).is_some() {}
        }
    }

    /// Honor control-path consequences of `params` (currently: `size`
    /// retargeting). Cheap when nothing changed. Call between blocks from
    /// the CLI, or from a worker thread in a plugin shell.
    pub fn service(&mut self, p: &EngineParams) {
        let size = p.size.clamp(0.25, 4.0);
        let ungated = p.tails == TailMode::Ungated;
        for zone in 0..MAX_ZONES {
            while self.take_retired(zone).is_some() {}
            let needs = {
                let b = &self.branches[zone];
                let current = if ungated {
                    // New epoch per retarget; age-gate (~100 ms) bounds
                    // voice churn during offline sweeps.
                    b.live
                        .as_ref()
                        .filter(|v| v.age > 19)
                        .map(|v| v.set.rendered_size)
                        .or((b.pending.is_none() && b.active_k > 0)
                            .then_some(b.rendered_size))
                } else {
                    (b.pending.is_none() && b.active_k > 0).then_some(b.rendered_size)
                };
                self.sources[zone].is_some()
                    && current.is_some_and(|cs| (cs - size).abs() > 1e-3)
            };
            if needs {
                let src = self.sources[zone].take().unwrap();
                let set = self.render_partition_set(&src, size);
                self.sources[zone] = Some(src);
                let _ = self.queue_partition_set(zone, set, p);
            }
        }
    }

    /// A standalone renderer matching this engine's geometry — hand it to
    /// a worker thread; its [`PartitionSet`]s are accepted by
    /// [`Engine::queue_partition_set`].
    pub fn renderer(&self) -> IrRenderer {
        IrRenderer {
            sr: self.sr,
            part: self.part,
            bins: self.bins,
            max_parts: self.max_parts,
        }
    }

    fn render_partition_set(&self, src: &SourceIr, size: f64) -> PartitionSet {
        self.renderer().render(&src.data, src.sr, size)
    }

    /// Take one spent set for off-thread dropping (call until `None`).
    pub fn take_retired(&mut self, zone: usize) -> Option<PartitionSet> {
        self.branches[zone]
            .retired
            .iter_mut()
            .find_map(|slot| slot.take())
    }

    // ---------------------------------------------------------------
    // RT-safe handoff
    // ---------------------------------------------------------------

    /// Hand a rendered IR to a zone. Move-only, no allocation; spent sets
    /// (displaced pendings, evicted/finished ring voices, completed swaps)
    /// park in the retired queue — drain [`Engine::take_retired`] until
    /// `None` from the control side. `Err(set)` only for geometry
    /// mismatches.
    ///
    /// `ungated == false` ([`TailMode::Gated`]): B&S streaming replacement
    /// into the shared H bank; an in-flight swap is displaced (cursor
    /// restarts — latest wins).
    ///
    /// `ungated == true` ([`TailMode::Ungated`]): the current live voice
    /// freezes and rings out; `set` becomes a fresh input-epoch voice
    /// effective immediately. If the H-bank voice is audible (bootstrap
    /// from Gated mode), it is streamed out to silence via the normal
    /// fade machinery.
    pub fn queue_partition_set(
        &mut self,
        zone: usize,
        set: PartitionSet,
        p: &EngineParams,
    ) -> Result<(), PartitionSet> {
        let ungated = p.tails == TailMode::Ungated;
        let ring_cap = (p.ring.round() as usize).clamp(1, RING_SLOTS);
        let b = &mut self.branches[zone];
        if set.bins != self.bins || set.k > self.max_parts || set.ch == 0 {
            return Err(set);
        }
        b.ir_ch = set.ch;
        if ungated {
            // Displace any H-bank stream; fade the H voice to silence.
            if let Some(p) = b.pending.take() {
                if let Some(s) = p.set {
                    b.retire_push(s);
                }
            }
            if b.active_k > 0 {
                b.pending = Some(Pending {
                    set: None,
                    cursor: 0,
                    eff_k: b.active_k,
                    hist: [0; MAX_FADE + 1],
                });
            }
            b.freeze_live(ring_cap);
            let mut tail = b
                .tail_pool
                .pop()
                .expect("tail pool sized for live+rings");
            for t in &mut tail {
                t.fill(0.0);
            }
            b.live = Some(LayerVoice {
                set,
                age: 0,
                epoch_len: 0,
                gain: 1.0,
                dying: false,
                tail,
            });
        } else {
            // A live epoch voice rings out rather than vanishing.
            b.freeze_live(ring_cap);
            if let Some(p) = b.pending.take() {
                if let Some(s) = p.set {
                    b.retire_push(s);
                }
            }
            let eff_k = set.k.max(b.active_k);
            b.active_k = eff_k; // stale-beyond-old-k partitions are zero
            b.pending = Some(Pending {
                set: Some(set),
                cursor: 0,
                eff_k,
                hist: [0; MAX_FADE + 1],
            });
        }
        Ok(())
    }

    // ---------------------------------------------------------------
    // RT path
    // ---------------------------------------------------------------

    /// In-place processing of `io[channel][sample]`. Allocation-free.
    pub fn process_block(&mut self, io: &mut [&mut [f32]], p: &EngineParams) {
        let channels = self.channels.min(io.len());
        if channels == 0 {
            return;
        }
        let n = io[0].len();
        for i in 0..n {
            for c in 0..channels {
                self.in_fifo[c][self.fifo_fill] = io[c][i];
            }
            self.fifo_fill += 1;
            if self.fifo_fill == self.part {
                self.process_frame(p, channels);
                self.fifo_fill = 0;
            }
            for c in 0..channels {
                io[c][i] = self.out_fifo[c].pop_front().unwrap_or(0.0);
            }
        }
    }

    fn process_frame(&mut self, p: &EngineParams, channels: usize) {
        let part = self.part;
        let n_zones = p.n_zones.clamp(1, MAX_ZONES);
        let att = time_coeff(p.attack_ms, self.sr);
        let rel = time_coeff(p.release_ms, self.sr);

        // --- level detection & branch input shaping -------------------
        let mut w = [0.0f32; MAX_ZONES];
        let mut w_acc = [0.0f32; MAX_ZONES];
        let mut peak = 0.0f32;
        for i in 0..part {
            let mut frame_peak = 0.0f32;
            for c in 0..channels {
                frame_peak = frame_peak.max(self.in_fifo[c][i].abs());
            }
            peak = peak.max(frame_peak);
            let fp = frame_peak as f64;
            self.env = if fp > self.env {
                att * self.env + (1.0 - att) * fp
            } else {
                rel * self.env + (1.0 - rel) * fp
            };
            let sym = p.sym.clamp(0.0, 1.0) as f32;
            match p.level_mode {
                LevelMode::Envelope => {
                    zone_weights(db(self.env), p, n_zones, &mut w);
                    for c in 0..channels {
                        let x = self.in_fifo[c][i];
                        let mut we = w;
                        if sym > 0.0 && x < 0.0 {
                            mirror_weights(&mut we, n_zones, sym);
                        }
                        if c == 0 {
                            for z in 0..n_zones {
                                w_acc[z] += we[z];
                            }
                        }
                        for z in 0..n_zones {
                            self.shaped[z][c][i] = x * we[z];
                        }
                    }
                }
                LevelMode::Instant => {
                    for c in 0..channels {
                        let x = self.in_fifo[c][i];
                        zone_weights(db(x.abs() as f64), p, n_zones, &mut w);
                        if sym > 0.0 && x < 0.0 {
                            mirror_weights(&mut w, n_zones, sym);
                        }
                        if c == 0 {
                            for z in 0..n_zones {
                                w_acc[z] += w[z];
                            }
                        }
                        for z in 0..n_zones {
                            self.shaped[z][c][i] = x * w[z];
                        }
                    }
                }
            }
        }
        for z in n_zones..MAX_ZONES {
            for c in 0..channels {
                self.shaped[z][c].fill(0.0);
            }
        }

        // --- branch convolutions --------------------------------------
        for c in 0..channels {
            self.wet[c].fill(0.0);
        }
        let mut zone_energy = [0.0f32; MAX_ZONES];
        let mut swap_progress = 1.0f32;
        for z in 0..MAX_ZONES {
            let (active, has_pending, has_voices) = {
                let b = &self.branches[z];
                (
                    b.active_k > 0,
                    b.pending.is_some(),
                    b.live.is_some() || b.ringing.iter().any(|r| r.is_some()),
                )
            };
            if !active && !has_pending && !has_voices {
                continue;
            }
            let zg = if z < n_zones {
                (p.zone_gain[z] * p.wet) as f32
            } else {
                0.0
            };

            // Advance the streaming swap by `morph` partitions (B&S cursor,
            // rate-scaled). Each written partition fades to target over
            // WRITE_STAGES frames (¼→½→¾→1 via the in-place recurrence
            // h += (T−h)/(STAGES+1−s)) — the skitter softener. A completed
            // swap retires only when the caller has drained the previous
            // retired set (no RT drops).
            {
                let bins = self.bins;
                let steps = (p.morph.round() as usize).clamp(1, 16);
                let fade = (p.fade_frames.round() as usize).clamp(1, MAX_FADE);
                let b = &mut self.branches[z];
                let mut done = false;
                if let Some(pend) = &mut b.pending {
                    let k_new = pend.set.as_ref().map(|s| s.k).unwrap_or(0);
                    for s in (1..=MAX_FADE).rev() {
                        pend.hist[s] = pend.hist[s - 1];
                    }
                    pend.cursor = (pend.cursor + steps).min(pend.eff_k);
                    pend.hist[0] = pend.cursor;
                    for s in 1..=MAX_FADE {
                        let lo = pend.hist[s];
                        let hi = pend.hist[s - 1];
                        if lo >= hi {
                            continue;
                        }
                        // Stage factor lifts the lerp chain k/fade; stages
                        // at/past `fade` finalize exactly (also covers a
                        // fade decrease mid-stream).
                        let f = if s >= fade {
                            1.0f32
                        } else {
                            1.0 / (fade + 1 - s) as f32
                        };
                        for part_i in lo..hi {
                            for c in 0..channels {
                                let dst = &mut b.h[c][part_i * bins..(part_i + 1) * bins];
                                if part_i < k_new {
                                    let set = pend.set.as_ref().unwrap();
                                    let src_c = c.min(set.ch - 1);
                                    let t =
                                        &set.spectra[(src_c * k_new + part_i) * bins..][..bins];
                                    if f >= 1.0 {
                                        dst.copy_from_slice(t);
                                    } else {
                                        for (d, &tv) in dst.iter_mut().zip(t) {
                                            *d += (tv - *d) * f;
                                        }
                                    }
                                } else if f >= 1.0 {
                                    dst.fill(Complex::new(0.0, 0.0));
                                } else {
                                    for d in dst.iter_mut() {
                                        *d -= *d * f; // fade out (unload)
                                    }
                                }
                            }
                        }
                    }
                    if pend.cursor >= pend.eff_k && pend.hist[MAX_FADE] >= pend.eff_k {
                        done = true;
                    }
                    swap_progress = swap_progress.min(b.swap_progress());
                }
                if done {
                    let pend = b.pending.take().unwrap();
                    b.active_k = pend.set.as_ref().map(|s| s.k).unwrap_or(0);
                    if let Some(s) = pend.set {
                        b.rendered_size = s.rendered_size;
                        b.retire_push(s);
                    }
                }
            }

            let (head, active_k) = {
                let b = &mut self.branches[z];
                b.head = (b.head + 1) % self.max_parts;
                (b.head, b.active_k)
            };
            for c in 0..channels {
                // FFT of the shaped input block into the ring.
                self.time_buf[..part].copy_from_slice(&self.shaped[z][c]);
                self.time_buf[part..].fill(0.0);
                {
                    let b = &mut self.branches[z];
                    let slot = &mut b.x_ring[c][head * self.bins..(head + 1) * self.bins];
                    self.fft
                        .process_with_scratch(&mut self.time_buf, slot, &mut self.fft_scratch)
                        .expect("fft");
                }
                // Frequency-domain delay line accumulation.
                self.acc_buf.fill(Complex::new(0.0, 0.0));
                {
                    let b = &self.branches[z];
                    for kp in 0..active_k {
                        let slot = (head + self.max_parts - kp) % self.max_parts;
                        let x = &b.x_ring[c][slot * self.bins..(slot + 1) * self.bins];
                        let h = &b.h[c][kp * self.bins..(kp + 1) * self.bins];
                        for ((a, &xv), &hv) in self.acc_buf.iter_mut().zip(x).zip(h) {
                            *a += xv * hv;
                        }
                    }
                }
                // Back to time; overlap-add.
                self.acc_buf[0].im = 0.0;
                self.acc_buf[self.bins - 1].im = 0.0;
                self.ifft
                    .process_with_scratch(
                        &mut self.acc_buf,
                        &mut self.out_time,
                        &mut self.ifft_scratch,
                    )
                    .expect("ifft");
                let scale = 1.0 / (2 * part) as f32;
                let b = &mut self.branches[z];
                let mut e = 0.0f32;
                for i in 0..part {
                    let s = self.out_time[i] * scale + b.tail[c][i];
                    b.tail[c][i] = self.out_time[part + i] * scale;
                    self.wet[c][i] += s * zg;
                    e += s * s;
                }
                if c == 0 {
                    zone_energy[z] = (e / part as f32).sqrt();
                }

                // --- epoch voices (Ungated): share this branch's x_ring,
                //     gated by age. vi 0 = live, 1..= frozen ring-outs.
                for vi in 0..=RING_SLOTS {
                    let range = {
                        let b = &self.branches[z];
                        let v = if vi == 0 {
                            b.live.as_ref()
                        } else {
                            b.ringing[vi - 1].as_ref()
                        };
                        v.map(|v| {
                            let k = v.set.k;
                            if vi == 0 {
                                (0usize, k.min(v.age + 1)) // live: lags 0..=age
                            } else {
                                // frozen: its own epoch's input only
                                (v.age.min(k), k.min(v.age + v.epoch_len))
                            }
                        })
                    };
                    let Some((lo, hi)) = range else { continue };
                    if lo < hi {
                        self.acc_buf.fill(Complex::new(0.0, 0.0));
                        {
                            let b = &self.branches[z];
                            let v = if vi == 0 {
                                b.live.as_ref().unwrap()
                            } else {
                                b.ringing[vi - 1].as_ref().unwrap()
                            };
                            let k = v.set.k;
                            let src_c = c.min(v.set.ch - 1);
                            for j in lo..hi {
                                let slot = (head + self.max_parts - j) % self.max_parts;
                                let x = &b.x_ring[c][slot * self.bins..(slot + 1) * self.bins];
                                let h = &v.set.spectra[(src_c * k + j) * self.bins..][..self.bins];
                                for ((a, &xv), &hv) in self.acc_buf.iter_mut().zip(x).zip(h) {
                                    *a += xv * hv;
                                }
                            }
                        }
                        self.acc_buf[0].im = 0.0;
                        self.acc_buf[self.bins - 1].im = 0.0;
                        self.ifft
                            .process_with_scratch(
                                &mut self.acc_buf,
                                &mut self.out_time,
                                &mut self.ifft_scratch,
                            )
                            .expect("ifft");
                        let b = &mut self.branches[z];
                        let v = if vi == 0 {
                            b.live.as_mut().unwrap()
                        } else {
                            b.ringing[vi - 1].as_mut().unwrap()
                        };
                        // Dying voices ramp IN-frame (a per-frame gain
                        // step is itself a staircase of clicklets).
                        let g0 = zg * v.gain;
                        let g1 = if v.dying { g0 * 0.5 } else { g0 };
                        let gstep = (g1 - g0) / part as f32;
                        for i in 0..part {
                            let s = self.out_time[i] * scale + v.tail[c][i];
                            v.tail[c][i] = self.out_time[part + i] * scale;
                            self.wet[c][i] += s * (g0 + gstep * (i as f32 + 1.0));
                        }
                    } else {
                        // No conv contribution left: flush the residual
                        // OLA tail once (voice dies at frame end).
                        let b = &mut self.branches[z];
                        let v = if vi == 0 {
                            b.live.as_mut().unwrap()
                        } else {
                            b.ringing[vi - 1].as_mut().unwrap()
                        };
                        let g0 = zg * v.gain;
                        let g1 = if v.dying { g0 * 0.5 } else { g0 };
                        let gstep = (g1 - g0) / part as f32;
                        for i in 0..part {
                            self.wet[c][i] += v.tail[c][i] * (g0 + gstep * (i as f32 + 1.0));
                            v.tail[c][i] = 0.0;
                        }
                    }
                }
            }

            // --- voice ages & deaths (once per frame, after all channels)
            {
                let ring_cap = (p.ring.round() as usize).clamp(1, RING_SLOTS);
                let b = &mut self.branches[z];
                if let Some(v) = &mut b.live {
                    v.age += 1;
                }
                for i in 0..RING_SLOTS {
                    let dead = matches!(
                        &b.ringing[i],
                        Some(v) if v.age >= v.set.k || v.gain < 1e-4
                    );
                    if dead {
                        let v = b.ringing[i].take().unwrap();
                        b.tail_pool.push(v.tail);
                        b.retire_push(v.set);
                    } else if let Some(v) = &mut b.ringing[i] {
                        v.age += 1;
                        if i >= ring_cap {
                            v.dying = true; // cap reduced mid-ring
                        }
                        if v.dying {
                            v.gain *= 0.5; // −6 dB/frame ≈ 50 ms fade-out
                        }
                    }
                }
            }
        }

        // --- mix to output FIFO ---------------------------------------
        let dry = p.dry as f32;
        let sat = p.sat.max(0.0) as f32;
        for c in 0..channels {
            for i in 0..part {
                let mut w = self.wet[c][i];
                if sat > 0.0 {
                    w = (w * sat).tanh() / sat;
                }
                self.out_fifo[c].push_back(self.in_fifo[c][i] * dry + w);
            }
        }
        self.t += part as u64;

        // --- viz -------------------------------------------------------
        if self.viz.len() < VIZ_CAP {
            let inv = 1.0 / part as f32;
            let mut weights = [0.0f32; MAX_ZONES];
            for z in 0..MAX_ZONES {
                weights[z] = w_acc[z] * inv;
            }
            self.viz.push_back(VizFrame {
                t: self.t,
                in_peak_db: db(peak as f64) as f32,
                env_db: db(self.env) as f32,
                weights,
                zone_energy,
                swap_progress,
            });
        }
    }
}

fn db(lin: f64) -> f64 {
    if lin <= 1e-8 {
        SILENCE_DB
    } else {
        20.0 * lin.log10()
    }
}

fn time_coeff(ms: f64, sr: f64) -> f64 {
    let samples = (ms.max(0.01) / 1000.0) * sr;
    (-1.0 / samples).exp()
}

/// Blend zone weights toward their mirror (z ↔ n−1−z) by `sym` ∈ (0,1].
/// Preserves partition of unity (a convex mix of two partitions of unity).
fn mirror_weights(w: &mut [f32; MAX_ZONES], n_zones: usize, sym: f32) {
    let mut m = [0.0f32; MAX_ZONES];
    for z in 0..n_zones {
        m[z] = (1.0 - sym) * w[z] + sym * w[n_zones - 1 - z];
    }
    w[..n_zones].copy_from_slice(&m[..n_zones]);
}

/// Triangular zone windows in dB space (partition of unity over the active
/// zones). `zone_db` ascending; extremes own everything beyond them.
fn zone_weights(level_db: f64, p: &EngineParams, n_zones: usize, w: &mut [f32; MAX_ZONES]) {
    *w = [0.0; MAX_ZONES];
    if n_zones == 1 {
        w[0] = 1.0;
        return;
    }
    let c = &p.zone_db;
    if level_db <= c[0] {
        w[0] = 1.0;
        return;
    }
    if level_db >= c[n_zones - 1] {
        w[n_zones - 1] = 1.0;
        return;
    }
    for i in 0..n_zones - 1 {
        if level_db < c[i + 1] {
            let span = (c[i + 1] - c[i]).max(1e-9);
            let f = ((level_db - c[i]) / span) as f32;
            w[i] = 1.0 - f;
            w[i + 1] = f;
            return;
        }
    }
    w[n_zones - 1] = 1.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_conv(x: &[f32], h: &[f32]) -> Vec<f32> {
        let mut y = vec![0.0f64; x.len() + h.len() - 1];
        for (i, &xi) in x.iter().enumerate() {
            for (j, &hj) in h.iter().enumerate() {
                y[i + j] += xi as f64 * hj as f64;
            }
        }
        y.into_iter().map(|v| v as f32).collect()
    }

    fn rng_seq(seed: u64, n: usize) -> Vec<f32> {
        // xorshift64*, deterministic test signals
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s >> 12;
                s ^= s << 25;
                s ^= s >> 27;
                let v = (s.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f64;
                (v / (1u64 << 24) as f64 * 2.0 - 1.0) as f32 * 0.5
            })
            .collect()
    }

    fn run_engine(e: &mut Engine, p: &EngineParams, x: &[f32], extra: usize) -> Vec<f32> {
        let mut input = x.to_vec();
        input.extend(std::iter::repeat(0.0).take(extra + e.latency_samples()));
        let mut out = Vec::with_capacity(input.len());
        for chunk in input.chunks(64) {
            let mut buf = chunk.to_vec();
            let mut io = [buf.as_mut_slice()];
            e.process_block(&mut io, p);
            out.extend_from_slice(&buf);
        }
        out.split_off(e.latency_samples()) // trim latency
    }

    fn single_zone_params() -> EngineParams {
        EngineParams {
            n_zones: 1,
            wet: 1.0,
            dry: 0.0,
            sat: 0.0, // linear: these tests compare against exact convolution
            ..Default::default()
        }
    }

    #[test]
    fn single_zone_matches_naive_convolution() {
        let sr = 48000.0;
        let ir = rng_seq(7, 700);
        let x = rng_seq(42, 2000);
        let mut e = Engine::new_sized(sr, 1, 64, 0.1);
        e.set_source_ir(0, vec![ir.clone()], sr, 1.0);
        let p = single_zone_params();
        let y = run_engine(&mut e, &p, &x, ir.len());
        let yref = naive_conv(&x, &ir);
        let n = yref.len().min(y.len());
        let mut max_err = 0.0f32;
        let mut max_ref = 0.0f32;
        for i in 0..n {
            max_err = max_err.max((y[i] - yref[i]).abs());
            max_ref = max_ref.max(yref[i].abs());
        }
        assert!(
            max_err / max_ref < 1e-4,
            "rel err {}",
            max_err / max_ref
        );
    }

    #[test]
    fn stepwise_swap_settles_to_new_ir() {
        let sr = 48000.0;
        let ir_a = rng_seq(7, 512);
        let ir_b = rng_seq(9, 512);
        let mut e = Engine::new_sized(sr, 1, 64, 0.1);
        e.set_source_ir(0, vec![ir_a], sr, 1.0);
        let p = single_zone_params();
        // Run some signal through A, then queue B and keep running until
        // the swap completes plus one full tail; then verify fresh input
        // convolves with B.
        let warm = rng_seq(3, 1024);
        let _ = run_engine(&mut e, &p, &warm, 0);
        e.set_source_ir(0, vec![ir_b.clone()], sr, 1.0);
        // flush: transition (k parts × P) + old tail
        let silence = vec![0.0f32; 512 * 4];
        let _ = run_engine(&mut e, &p, &silence, 0);
        let x = rng_seq(11, 800);
        let y = run_engine(&mut e, &p, &x, 512);
        let yref = naive_conv(&x, &ir_b);
        let n = yref.len().min(y.len());
        let mut err = 0.0f64;
        let mut refe = 0.0f64;
        for i in 0..n {
            err += ((y[i] - yref[i]) as f64).powi(2);
            refe += (yref[i] as f64).powi(2);
        }
        let snr = 10.0 * (refe / err.max(1e-30)).log10();
        assert!(snr > 60.0, "post-swap SNR {snr} dB");
    }

    #[test]
    fn displaced_swap_settles_to_newest_ir() {
        // A → (B displaced mid-stream by C) must settle exactly to C, with
        // the 4-stage blended writes fully finalized.
        let sr = 48000.0;
        let ir_a = rng_seq(7, 512);
        let ir_b = rng_seq(9, 512);
        let ir_c = rng_seq(13, 384);
        let mut e = Engine::new_sized(sr, 1, 64, 0.1);
        e.set_source_ir(0, vec![ir_a], sr, 1.0);
        let p = single_zone_params();
        let warm = rng_seq(3, 1024);
        let _ = run_engine(&mut e, &p, &warm, 0);
        e.set_source_ir(0, vec![ir_b], sr, 1.0); // starts streaming B
        let mid = rng_seq(5, 128); // let B's stream advance a couple frames
        let _ = run_engine(&mut e, &p, &mid, 0);
        e.set_source_ir(0, vec![ir_c.clone()], sr, 1.0); // displace with C
        let silence = vec![0.0f32; 512 * 4];
        let _ = run_engine(&mut e, &p, &silence, 0);
        let x = rng_seq(11, 800);
        let y = run_engine(&mut e, &p, &x, 384);
        let yref = naive_conv(&x, &ir_c);
        let n = yref.len().min(y.len());
        let mut err = 0.0f64;
        let mut refe = 0.0f64;
        for i in 0..n {
            err += ((y[i] - yref[i]) as f64).powi(2);
            refe += (yref[i] as f64).powi(2);
        }
        let snr = 10.0 * (refe / err.max(1e-30)).log10();
        assert!(snr > 60.0, "post-displacement SNR {snr} dB");
    }

    #[test]
    fn ungated_epochs_are_exact_input_split() {
        // Ungated switching = exact parallel convolution: old voice hears
        // input only before the switch (rings out fully), new voice only
        // after. y == conv(x_pre, A) + conv(x_post, B), no crossfade.
        let sr = 48000.0;
        let part = 64usize;
        let ir_a = rng_seq(7, 512);
        let ir_b = rng_seq(9, 512);
        let mut e = Engine::new_sized(sr, 1, part, 0.1);
        let r = e.renderer();
        let set_a = r.render(&[ir_a.clone()], sr, 1.0);
        let set_b = r.render(&[ir_b.clone()], sr, 1.0);
        let p = EngineParams {
            tails: TailMode::Ungated,
            ..single_zone_params()
        };
        e.queue_partition_set(0, set_a, &p).ok().unwrap();
        let x = rng_seq(21, 2048);
        let n_t = 1024; // frame-aligned switch
        let mut input = x.clone();
        input.extend(std::iter::repeat(0.0).take(1024 + part)); // tail+latency
        let mut out = Vec::new();
        let mut set_b = Some(set_b);
        for (i, chunk) in input.chunks(part).enumerate() {
            if i * part == n_t {
                e.queue_partition_set(0, set_b.take().unwrap(), &p)
                    .ok()
                    .unwrap();
            }
            let mut buf = chunk.to_vec();
            let mut io = [buf.as_mut_slice()];
            e.process_block(&mut io, &p);
            out.extend_from_slice(&buf);
        }
        let y = out.split_off(part); // trim latency
        let mut xa = x.clone();
        xa[n_t..].fill(0.0);
        let mut xb = x.clone();
        xb[..n_t].fill(0.0);
        let ya = naive_conv(&xa, &ir_a);
        let yb = naive_conv(&xb, &ir_b);
        let n = y.len().min(ya.len().max(yb.len()));
        let mut err = 0.0f64;
        let mut refe = 0.0f64;
        for i in 0..n {
            let r = *ya.get(i).unwrap_or(&0.0) as f64 + *yb.get(i).unwrap_or(&0.0) as f64;
            err += (y[i] as f64 - r).powi(2);
            refe += r.powi(2);
        }
        let snr = 10.0 * (refe / err.max(1e-30)).log10();
        assert!(snr > 80.0, "ungated input-split SNR {snr} dB");
    }

    #[test]
    fn ungated_eviction_storm_stays_smooth() {
        // Switching far faster than voices decay must not hard-cut hot
        // ring voices (the "DC clicks" report): evictions ramp out.
        // Smooth tonal input + tonal IRs ⇒ any click is a huge outlier
        // in the second difference.
        let sr = 48000.0;
        let part = 64usize;
        let mut e = Engine::new_sized(sr, 1, part, 0.1);
        let r = e.renderer();
        let mk_ir = |f0: f64| -> Vec<f32> {
            (0..2048)
                .map(|i| {
                    let t = i as f64 / sr;
                    ((std::f64::consts::TAU * f0 * t).sin()
                        * (-t * 18.0).exp()) as f32
                        * 0.01
                })
                .collect()
        };
        let freqs = [55.0, 70.0, 90.0, 110.0];
        let p = EngineParams {
            tails: TailMode::Ungated,
            ..single_zone_params()
        };
        let n = part * 160;
        let x: Vec<f32> = (0..n)
            .map(|i| ((std::f64::consts::TAU * 150.0 * i as f64 / sr).sin() * 0.5) as f32)
            .collect();
        let mut out = Vec::with_capacity(n);
        for (fi, chunk) in x.chunks(part).enumerate() {
            if fi % 10 == 0 {
                // every 10 frames ≈ the plugin's 50 ms debounce floor —
                // still far faster than these voices decay (τ≈2700 smp,
                // ringing for 32-partition spans)
                let set = r.render(&[mk_ir(freqs[(fi / 10) % 4])], sr, 1.0);
                e.queue_partition_set(0, set, &p).ok().unwrap();
            }
            let mut buf = chunk.to_vec();
            let mut io = [buf.as_mut_slice()];
            e.process_block(&mut io, &p);
            out.extend_from_slice(&buf);
        }
        let d2: Vec<f32> = out
            .windows(3)
            .skip(2 * part) // settle past latency
            .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
            .collect();
        let max_d2 = d2.iter().fold(0.0f32, |m, &v| m.max(v));
        assert!(
            max_d2 < 0.02,
            "eviction discontinuity: max |Δ²| = {max_d2} (hard cuts land ≈ 0.2+)"
        );
    }

    #[test]
    fn zone_weights_partition_of_unity() {
        let p = EngineParams::default();
        let mut w = [0.0f32; MAX_ZONES];
        for i in 0..200 {
            let ldb = -70.0 + i as f64 * 0.4;
            zone_weights(ldb, &p, 4, &mut w);
            let s: f32 = w.iter().sum();
            assert!((s - 1.0).abs() < 1e-6, "sum {} at {} dB", s, ldb);
        }
    }

    #[test]
    fn latency_is_one_partition() {
        let mut e = Engine::new_sized(48000.0, 1, 128, 0.05);
        e.set_source_ir(0, vec![vec![1.0]], 48000.0, 1.0); // identity IR
        let p = single_zone_params();
        let mut x = vec![0.0f32; 512];
        x[0] = 1.0;
        let mut io_buf = x.clone();
        let mut io = [io_buf.as_mut_slice()];
        e.process_block(&mut io, &p);
        // impulse should emerge exactly latency_samples later
        let idx = io_buf
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap()
            .0;
        assert_eq!(idx, e.latency_samples());
    }
}
