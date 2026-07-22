//! Synthetic IR banks + IR normalization, shared by the CLI renderer and
//! the plugin's worker thread. All deterministic (seeded xorshift), all
//! normalized by [`windowed_spectral_norm`] (see LISTENING-LOG — the
//! maintainer's local, untracked listening diary — Defect 001
//! for why that specific law).

/// The built-in bank set, quiet zone (0) → loud zone (3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bank {
    /// Exp-decay noise rooms: quiet = long/dark … loud = short/bright.
    Rooms,
    /// 808-shaped tuned booms with downward glide; loud = deepest/hardest.
    Subdrop,
    /// Filtered-noise chambers + damped low modes; loud = deepest.
    Resoroom,
}

impl Bank {
    pub fn from_name(name: &str) -> Option<Bank> {
        match name {
            "rooms" => Some(Bank::Rooms),
            "subdrop" => Some(Bank::Subdrop),
            "resoroom" => Some(Bank::Resoroom),
            _ => None,
        }
    }
    pub fn from_index(i: usize) -> Option<Bank> {
        [Bank::Rooms, Bank::Subdrop, Bank::Resoroom].get(i).copied()
    }
}

/// Render one zone of a bank (stereo-decorrelated pair) at `sr`.
pub fn render_bank(bank: Bank, zone: usize, sr: f64) -> Vec<Vec<f32>> {
    match bank {
        Bank::Rooms => bank_rooms(zone, sr),
        Bank::Subdrop => bank_subdrop(zone, sr),
        Bank::Resoroom => bank_resoroom(zone, sr),
    }
}

/// xorshift64* — deterministic synth IRs, no rand dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f64;
        (v / (1u64 << 24) as f64 * 2.0 - 1.0) as f32
    }
}

/// Normalize by the max *short-time* spectral magnitude (≈85 ms windows,
/// 50% hop), targeting +12 dB burst gain at resonance.
///
/// The normalization saga (see LISTENING-LOG batches 003/004): energy
/// norm exploded (+15..24 dBFS) on sustained input at a tonal IR's own
/// frequency (coherent accumulation); global-spectral norm (max|H|=1)
/// buried the wet 40..70 dB down for transient input — a resonator with
/// bounded steady-state gain has a physically tiny impulse response
/// (Q-factor physics). Both were single-global-statistic mistakes.
/// Bounding the ~85 ms burst gain matches what percussive material
/// actually excites; the engine's wet tanh absorbs the residual
/// long-coherence headroom musically.
pub fn windowed_spectral_norm(h: &mut [f32], sr: f64) {
    let target = 10.0f32.powf(6.0 / 20.0); // +6 dB burst gain (was +12; "prefer it clean" verdict)
    let w = (((0.085 * sr) as usize).next_power_of_two()).min(h.len().next_power_of_two());
    let n = 2 * w;
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut spec = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut buf = vec![0.0f32; n];
    let mut peak = 0.0f32;
    let mut start = 0usize;
    while start < h.len() {
        buf.fill(0.0);
        let end = (start + w).min(h.len());
        buf[..end - start].copy_from_slice(&h[start..end]);
        fft.process_with_scratch(&mut buf, &mut spec, &mut scratch)
            .expect("norm fft");
        peak = spec.iter().map(|c| c.norm()).fold(peak, f32::max);
        start += w / 2;
    }
    let g = target / peak.max(1e-12);
    for v in h {
        *v *= g;
    }
}

/// `rooms`: exp-decay noise, quiet zones long & dark, loud zones short &
/// bright. Stereo-decorrelated, energy-normalized. (Batches 001/002.)
fn bank_rooms(zone: usize, sr: f64) -> Vec<Vec<f32>> {
    let t60 = [3.2, 1.8, 0.9, 0.45][zone];
    let cutoff = [2500.0, 4500.0, 8000.0, 13000.0][zone];
    let len = ((t60 * 1.3) * sr) as usize;
    let alpha = 1.0 - (-2.0 * std::f64::consts::PI * cutoff / sr).exp();
    let decay = 10.0f64.powf(-3.0 / (t60 * sr));
    (0..2usize)
        .map(|c| {
            let mut rng = Rng(0x9E3779B97F4A7C15 ^ ((zone as u64) << 8) ^ c as u64);
            let mut lp = 0.0f64;
            let mut env = 1.0f64;
            let mut h: Vec<f32> = (0..len)
                .map(|_| {
                    lp += alpha * (rng.next() as f64 - lp);
                    let v = (lp * env) as f32;
                    env *= decay;
                    v
                })
                .collect();
            windowed_spectral_norm(&mut h, sr);
            h
        })
        .collect()
}

/// `subdrop`: 808-shaped tuned boom per zone — downward-gliding sine +
/// 2nd harmonic + click punch + faint noise body. Louder zones are DEEPER
/// and harder (velocity picks the boom). Slight inter-channel detune for
/// width. Size (resampling) retunes the whole bank. v2: shorter/punchier
/// than the batch-003 pure sines (1.4–1.9 s oscillators were the
/// pathological case for any normalization — see LISTENING-LOG).
fn bank_subdrop(zone: usize, sr: f64) -> Vec<Vec<f32>> {
    // zone 0 = quiet … zone 3 = loud
    let f_start = [130.0, 100.0, 76.0, 56.0][zone];
    let f_end = [92.0, 70.0, 52.0, 36.0][zone];
    let t60 = [0.5, 0.55, 0.65, 0.8][zone];
    let glide_tau = [0.10, 0.09, 0.08, 0.06][zone]; // s, fast drop
    let click = [0.06, 0.12, 0.2, 0.3][zone]; // punch amount
    let len = ((t60 * 1.2) * sr) as usize;
    let decay = 10.0f64.powf(-3.0 / (t60 * sr));
    (0..2usize)
        .map(|c| {
            let mut rng = Rng(0xD1CE ^ ((zone as u64) << 8) ^ c as u64);
            let detune = 1.0 + 0.0015 * (c as f64 - 0.5) * 2.0; // ±0.15%
            let mut phase = rng.next() as f64 * 0.5;
            let mut env = 1.0f64;
            let mut lp = 0.0f64;
            let alpha = 1.0 - (-2.0 * std::f64::consts::PI * 900.0 / sr).exp();
            let mut h: Vec<f32> = (0..len)
                .map(|i| {
                    let t = i as f64 / sr;
                    let f = (f_end + (f_start - f_end) * (-t / glide_tau).exp()) * detune;
                    phase += f / sr;
                    let ph = std::f64::consts::TAU * phase;
                    let mut v = ph.sin() * env;
                    v += 0.30 * (2.0 * ph).sin() * env * env; // 2nd harmonic, faster decay
                    lp += alpha * (rng.next() as f64 - lp);
                    v += 0.04 * lp * env; // faint noise body
                    if t < 0.012 {
                        let cw = 1.0 - t / 0.012;
                        v += click * rng.next() as f64 * cw * cw;
                    }
                    env *= decay;
                    v as f32
                })
                .collect();
            windowed_spectral_norm(&mut h, sr);
            h
        })
        .collect()
}

/// `resoroom`: filtered-noise room + three damped low-frequency modes —
/// boomy resonant chambers, deeper as zones get louder.
fn bank_resoroom(zone: usize, sr: f64) -> Vec<Vec<f32>> {
    let t60 = [2.6, 2.0, 1.6, 1.3][zone];
    let cutoff = [1800.0, 2600.0, 3600.0, 5200.0][zone];
    let scale = [1.9, 1.5, 1.2, 1.0][zone]; // mode pitch scale (quiet=higher)
    let modes = [44.0, 67.0, 89.0];
    let mode_t60 = [1.9, 1.4, 1.0];
    let len = ((t60 * 1.2) * sr) as usize;
    let alpha = 1.0 - (-2.0 * std::f64::consts::PI * cutoff / sr).exp();
    let decay = 10.0f64.powf(-3.0 / (t60 * sr));
    (0..2usize)
        .map(|c| {
            let mut rng = Rng(0xB0043 ^ ((zone as u64) << 8) ^ c as u64);
            let mut lp = 0.0f64;
            let mut env = 1.0f64;
            let phases: Vec<f64> = modes.iter().map(|_| rng.next() as f64).collect();
            let mut h: Vec<f32> = (0..len)
                .map(|i| {
                    let t = i as f64 / sr;
                    lp += alpha * (rng.next() as f64 - lp);
                    let mut v = 0.55 * lp * env;
                    for (m, (&f, &mt)) in modes.iter().zip(mode_t60.iter()).enumerate() {
                        let md = 10.0f64.powf(-3.0 * t / mt);
                        let fc = f * scale * (1.0 + 0.002 * (c as f64 - 0.5));
                        v += 0.3 * (std::f64::consts::TAU * (fc * t + phases[m])).sin() * md;
                    }
                    env *= decay;
                    v as f32
                })
                .collect();
            windowed_spectral_norm(&mut h, sr);
            h
        })
        .collect()
}
