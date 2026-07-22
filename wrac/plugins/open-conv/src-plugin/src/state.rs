//! Parameter state shared by the audio thread and host.
//!
//! One atomic per parameter, indexed by the spec table in `plugin/params.rs`.
//! The audio thread reads a full [`open_conv_engine::EngineParams`] snapshot
//! per block without taking any lock. No viz ring in the headless v0 shell
//! (the engine's ring is drained and discarded; the panel milestone will
//! publish it).

use atomic_float::AtomicF32;
use std::sync::atomic::Ordering;

use open_conv_engine::{CrystalShape, EngineParams, LevelMode, MAX_ZONES, ShaperMode, TailMode};

use crate::plugin::{
    PARAM_ATTACK_ID, PARAM_BLEND_X_ID, PARAM_BLEND_Y_ID, PARAM_CORNER_IDS, PARAM_CRYSTAL_ID,
    PARAM_DAMP_IDS, PARAM_BYPASS_ID, PARAM_DRY_ID, PARAM_FADE_ID, PARAM_MODE_ID, PARAM_SHAPER_ID,
    PARAM_MORPH_ID, PARAM_RELEASE_ID, PARAM_RELOAD_ID, PARAM_SIZE_ID, PARAM_SYM_ID, PARAM_TAILS_ID,
    PARAM_WET_ID, PARAM_ZONES_ID, PARAM_ZONE_GAIN_IDS, PARAM_ZONE_LEVEL_IDS, param_clamp,
    param_default, param_exists,
};

// Indexed by param id; ids 3 (Wet Sat) and 18 (IR Bank) are retired/dead.
pub(crate) const PARAM_SLOTS: usize = 37;

pub(crate) struct SharedState {
    values: [AtomicF32; PARAM_SLOTS],
}

impl SharedState {
    pub(crate) fn new() -> Self {
        let values = std::array::from_fn(|i| AtomicF32::new(param_default(i as u32)));
        Self { values }
    }

    /// Clamp + store. Returns the applied value, or None for unknown ids.
    pub(crate) fn set_parameter_value(&self, id: u32, plain: f64) -> Option<f32> {
        if !param_exists(id) {
            return None;
        }
        let v = param_clamp(id, plain as f32);
        self.values[id as usize].store(v, Ordering::Relaxed);
        Some(v)
    }

    pub(crate) fn parameter_value(&self, id: u32) -> Option<f32> {
        param_exists(id).then(|| self.values[id as usize].load(Ordering::Relaxed))
    }

    fn v(&self, id: u32) -> f32 {
        self.values[id as usize].load(Ordering::Relaxed)
    }

    pub(crate) fn bypass(&self) -> bool {
        self.v(PARAM_BYPASS_ID) >= 0.5
    }

    /// Which bank each XY corner holds (0..=3; 3 = watched folder).
    pub(crate) fn corner_banks(&self) -> [usize; 4] {
        std::array::from_fn(|i| self.v(PARAM_CORNER_IDS[i]).round().clamp(0.0, 3.0) as usize)
    }

    /// Reload trigger state; the processor watches for rising edges.
    pub(crate) fn reload_on(&self) -> bool {
        self.v(PARAM_RELOAD_ID) >= 0.5
    }

    /// Snapshot for the audio thread. Bypass is realized as wet=0/dry=1 —
    /// the dry path is latency-aligned, so this is a click-free,
    /// PDC-correct bypass. Zone centers are monotonized ascending (the
    /// engine's window function requires it; hosts allow any slider order).
    pub(crate) fn engine_params(&self) -> EngineParams {
        let mut zone_db = [0.0f64; MAX_ZONES];
        for (i, id) in PARAM_ZONE_LEVEL_IDS.iter().enumerate() {
            zone_db[i] = self.v(*id) as f64;
            if i > 0 && zone_db[i] < zone_db[i - 1] + 0.5 {
                zone_db[i] = zone_db[i - 1] + 0.5;
            }
        }
        let mut zone_gain = [1.0f64; MAX_ZONES];
        for (i, id) in PARAM_ZONE_GAIN_IDS.iter().enumerate() {
            zone_gain[i] = self.v(*id) as f64;
        }
        let mut p = EngineParams {
            n_zones: self.v(PARAM_ZONES_ID).round().clamp(1.0, MAX_ZONES as f32) as usize,
            zone_db,
            zone_gain,
            level_mode: if self.v(PARAM_MODE_ID) >= 0.5 {
                LevelMode::Envelope
            } else {
                LevelMode::Instant
            },
            attack_ms: self.v(PARAM_ATTACK_ID) as f64,
            release_ms: self.v(PARAM_RELEASE_ID) as f64,
            wet: self.v(PARAM_WET_ID) as f64,
            dry: self.v(PARAM_DRY_ID) as f64,
            size: self.v(PARAM_SIZE_ID) as f64,
            sym: self.v(PARAM_SYM_ID) as f64,
            morph: self.v(PARAM_MORPH_ID) as f64,
            fade_frames: self.v(PARAM_FADE_ID) as f64,
            tails: if self.v(PARAM_TAILS_ID) >= 0.5 {
                TailMode::Ungated
            } else {
                TailMode::Gated
            },
            ring: open_conv_engine::RING_SLOTS as f64, // always max depth
            blend_x: self.v(PARAM_BLEND_X_ID) as f64,
            blend_y: self.v(PARAM_BLEND_Y_ID) as f64,
            damp: std::array::from_fn(|i| self.v(PARAM_DAMP_IDS[i]) as f64),
            // Three-way Mode: 0 = Zones, 1 = Quartz (Chebyshev),
            // 2 = Bismuth (raw powers).
            shaper: if self.v(PARAM_SHAPER_ID) >= 0.5 {
                ShaperMode::Crystal
            } else {
                ShaperMode::Zones
            },
            drive: self.v(PARAM_CRYSTAL_ID) as f64,
            crystal_shape: if self.v(PARAM_SHAPER_ID) >= 1.5 {
                CrystalShape::RawV1
            } else {
                CrystalShape::Cheby
            },
        };
        if self.bypass() {
            p.wet = 0.0;
            p.dry = 1.0;
        }
        p
    }
}
