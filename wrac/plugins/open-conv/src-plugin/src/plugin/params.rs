//! The host-facing parameter table. Adding a parameter starts here; count,
//! info, conversions, defaults, and persistence all derive from this table.
//!
//! Parameter IDs are host/project ABI. Never renumber an existing id after
//! publishing; add new ids instead.

use std::sync::Arc;

use wrac_clap_adapter::{
    ParamFlags, ParamInfo, ParamInputEvents, PluginError, PluginParamsExtension, PluginResult,
};

use crate::state::SharedState;

pub(crate) const PARAM_BYPASS_ID: u32 = 0;
pub(crate) const PARAM_WET_ID: u32 = 1;
pub(crate) const PARAM_DRY_ID: u32 = 2;
// id 3 retired (Wet Sat — removed: no saturation stage exists)
pub(crate) const PARAM_SYM_ID: u32 = 4;
pub(crate) const PARAM_SIZE_ID: u32 = 5;
pub(crate) const PARAM_MODE_ID: u32 = 6;
pub(crate) const PARAM_ATTACK_ID: u32 = 7;
pub(crate) const PARAM_RELEASE_ID: u32 = 8;
pub(crate) const PARAM_ZONES_ID: u32 = 9;
pub(crate) const PARAM_ZONE_LEVEL_IDS: [u32; 4] = [10, 11, 12, 13];
pub(crate) const PARAM_ZONE_GAIN_IDS: [u32; 4] = [14, 15, 16, 17];
// id 18 retired (IR Bank — superseded by the XY pad corner selectors)
pub(crate) const PARAM_RELOAD_ID: u32 = 19;
pub(crate) const PARAM_MORPH_ID: u32 = 20;
pub(crate) const PARAM_CORNER_IDS: [u32; 4] = [24, 25, 26, 27];
pub(crate) const PARAM_BLEND_X_ID: u32 = 28;
pub(crate) const PARAM_BLEND_Y_ID: u32 = 29;
pub(crate) const PARAM_DAMP_IDS: [u32; 4] = [30, 31, 32, 33];
pub(crate) const PARAM_SHAPER_ID: u32 = 34;
pub(crate) const PARAM_CRYSTAL_ID: u32 = 35;
pub(crate) const PARAM_CRYSTAL_SHAPE_ID: u32 = 36;
pub(crate) const PARAM_FADE_ID: u32 = 21;
pub(crate) const PARAM_TAILS_ID: u32 = 22;

/// How a parameter formats/parses its value text.
#[derive(Debug, Clone, Copy)]
enum Format {
    Percent,
    Milliseconds,
    Decibels,
    /// Unit-free ratio (size/sat), 2 decimals.
    Ratio,
    Integer,
    Choice(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
struct ParameterSpec {
    info: ParamInfo,
    format: Format,
}

const fn flags(stepped: bool, is_enum: bool, is_bypass: bool) -> ParamFlags {
    ParamFlags {
        is_stepped: stepped,
        is_periodic: false,
        is_hidden: false,
        is_readonly: false,
        is_bypass,
        is_automatable: true,
        is_automatable_per_note_id: false,
        is_automatable_per_key: false,
        is_automatable_per_channel: false,
        is_automatable_per_port: false,
        is_modulatable: false,
        is_modulatable_per_note_id: false,
        is_modulatable_per_key: false,
        is_modulatable_per_channel: false,
        is_modulatable_per_port: false,
        requires_process: false,
        is_enum,
    }
}

const fn continuous(
    id: u32,
    name: &'static str,
    module: &'static str,
    min: f64,
    max: f64,
    default: f64,
    format: Format,
) -> ParameterSpec {
    ParameterSpec {
        info: ParamInfo {
            id,
            name,
            module,
            min_value: min,
            max_value: max,
            default_value: default,
            flags: flags(false, false, false),
        },
        format,
    }
}

const fn choice(
    id: u32,
    name: &'static str,
    module: &'static str,
    names: &'static [&'static str],
    default: f64,
    is_bypass: bool,
) -> ParameterSpec {
    ParameterSpec {
        info: ParamInfo {
            id,
            name,
            module,
            min_value: 0.0,
            max_value: (names.len() - 1) as f64,
            default_value: default,
            flags: flags(true, true, is_bypass),
        },
        format: Format::Choice(names),
    }
}

const fn stepped_int(
    id: u32,
    name: &'static str,
    module: &'static str,
    min: f64,
    max: f64,
    default: f64,
) -> ParameterSpec {
    ParameterSpec {
        info: ParamInfo {
            id,
            name,
            module,
            min_value: min,
            max_value: max,
            default_value: default,
            flags: flags(true, false, false),
        },
        format: Format::Integer,
    }
}

const OFF_ON: &[&str] = &["Off", "On"];

// Host domain == plain domain for every parameter (CLAP allows arbitrary
// ranges; wrappers normalize internally). Conversions are clamp-identity.
const PARAM_SPECS: &[ParameterSpec] = &[
    choice(PARAM_BYPASS_ID, "Bypass", "", OFF_ON, 0.0, true),
    continuous(PARAM_WET_ID, "Wet", "mix", 0.0, 1.0, 0.35, Format::Percent),
    continuous(PARAM_DRY_ID, "Dry", "mix", 0.0, 1.0, 1.0, Format::Percent),
    continuous(PARAM_SYM_ID, "Symmetry", "selector", 0.0, 1.0, 0.0, Format::Percent),
    continuous(PARAM_SIZE_ID, "Size", "ir", 0.25, 4.0, 1.0, Format::Ratio),
    choice(PARAM_MODE_ID, "Selector", "selector", &["Instant", "Envelope"], 1.0, false),
    continuous(PARAM_ATTACK_ID, "Attack", "selector", 0.1, 50.0, 5.0, Format::Milliseconds),
    continuous(PARAM_RELEASE_ID, "Release", "selector", 5.0, 1000.0, 120.0, Format::Milliseconds),
    stepped_int(PARAM_ZONES_ID, "Zones", "zones", 1.0, 4.0, 4.0),
    continuous(PARAM_ZONE_LEVEL_IDS[0], "Zone 1 Level", "zones", -70.0, 0.0, -48.0, Format::Decibels),
    continuous(PARAM_ZONE_LEVEL_IDS[1], "Zone 2 Level", "zones", -70.0, 0.0, -30.0, Format::Decibels),
    continuous(PARAM_ZONE_LEVEL_IDS[2], "Zone 3 Level", "zones", -70.0, 0.0, -18.0, Format::Decibels),
    continuous(PARAM_ZONE_LEVEL_IDS[3], "Zone 4 Level", "zones", -70.0, 0.0, -6.0, Format::Decibels),
    continuous(PARAM_ZONE_GAIN_IDS[0], "Zone 1 Gain", "zones", 0.0, 2.0, 1.0, Format::Percent),
    continuous(PARAM_ZONE_GAIN_IDS[1], "Zone 2 Gain", "zones", 0.0, 2.0, 1.0, Format::Percent),
    continuous(PARAM_ZONE_GAIN_IDS[2], "Zone 3 Gain", "zones", 0.0, 2.0, 1.0, Format::Percent),
    continuous(PARAM_ZONE_GAIN_IDS[3], "Zone 4 Gain", "zones", 0.0, 2.0, 1.0, Format::Percent),
    // Trigger semantics: the worker reloads on every rising edge.
    choice(PARAM_RELOAD_ID, "Reload IRs", "ir", OFF_ON, 0.0, false),
    // IR transition speed (partitions/frame). 1 = tail-length glide;
    // 16 = ~200 ms snap - turn up when automating Size/Bank per hit.
    continuous(PARAM_MORPH_ID, "Morph Speed", "ir", 1.0, 16.0, 1.0, Format::Ratio),
    // Per-partition write fade (frames, ~5.3 ms each). 1 = sharp steps
    // (skitter edge), 4 = default rounding, 16 = maximal smear.
    stepped_int(PARAM_FADE_ID, "Transition Fade", "ir", 1.0, 16.0, 4.0),
    // Old-IR policy: Gated = streaming replacement (morphs); Ungated =
    // every IR change lets the old room ring out fully while the new one
    // starts fresh (parallel voices, up to 3 ringing per zone).
    choice(PARAM_TAILS_ID, "Tails", "ir", &["Gated", "Ungated"], 0.0, false),
    // The XY pad: four banks in the corners, Blend X/Y is the ball.
    // "Folder" watches ~/Music/open-conv/zone{1..4}.wav.
    choice(PARAM_CORNER_IDS[0], "Pad NW", "pad", BANKS, 1.0, false),
    choice(PARAM_CORNER_IDS[1], "Pad NE", "pad", BANKS, 0.0, false),
    choice(PARAM_CORNER_IDS[2], "Pad SW", "pad", BANKS, 2.0, false),
    choice(PARAM_CORNER_IDS[3], "Pad SE", "pad", BANKS, 3.0, false),
    continuous(PARAM_BLEND_X_ID, "Blend X", "pad", 0.0, 1.0, 0.0, Format::Percent),
    continuous(PARAM_BLEND_Y_ID, "Blend Y", "pad", 0.0, 1.0, 0.0, Format::Percent),
    continuous(PARAM_DAMP_IDS[0], "Zone 1 Damp", "zones", 0.0, 1.0, 0.0, Format::Percent),
    continuous(PARAM_DAMP_IDS[1], "Zone 2 Damp", "zones", 0.0, 1.0, 0.0, Format::Percent),
    continuous(PARAM_DAMP_IDS[2], "Zone 3 Damp", "zones", 0.0, 1.0, 0.0, Format::Percent),
    continuous(PARAM_DAMP_IDS[3], "Zone 4 Damp", "zones", 0.0, 1.0, 0.0, Format::Percent),
    // Crystalize: slots become harmonic orders 1..4, each with its own
    // room — clean arithmetic harmonics, no clipping anywhere.
    choice(PARAM_SHAPER_ID, "Mode", "ir", &["Zones", "Crystalize"], 0.0, false),
    continuous(PARAM_CRYSTAL_ID, "Crystal Gain", "ir", 1.0, 8.0, 2.0, Format::Ratio),
    // A/B surface: Cheby = bounded, harmonically pure at full drive.
    // Raw v1 = the original unbounded power law — levels CAN run away.
    choice(
        PARAM_CRYSTAL_SHAPE_ID,
        "Crystal Shape",
        "ir",
        &["Cheby", "Raw v1"],
        0.0,
        false,
    ),
];

const BANKS: &[&str] = &["Rooms", "Subdrop", "Resoroom", "Folder"];

fn param_spec(id: u32) -> PluginResult<&'static ParameterSpec> {
    PARAM_SPECS
        .iter()
        .find(|spec| spec.info.id == id)
        .ok_or(PluginError::InvalidParameter)
}

pub(crate) fn param_exists(id: u32) -> bool {
    PARAM_SPECS.iter().any(|spec| spec.info.id == id)
}

pub(crate) fn param_clamp(id: u32, value: f32) -> f32 {
    match param_spec(id) {
        Ok(spec) => value.clamp(spec.info.min_value as f32, spec.info.max_value as f32),
        Err(_) => value,
    }
}

pub(crate) fn param_default(id: u32) -> f32 {
    param_spec(id).map(|s| s.info.default_value as f32).unwrap_or(0.0)
}

pub(crate) fn parameter_infos() -> impl Iterator<Item = ParamInfo> {
    PARAM_SPECS.iter().map(|spec| spec.info)
}

fn value_to_text(spec: &ParameterSpec, value: f64) -> String {
    match spec.format {
        Format::Percent => format!("{:.0} %", value * 100.0),
        Format::Milliseconds => format!("{value:.1} ms"),
        Format::Decibels => format!("{value:.1} dB"),
        Format::Ratio => format!("{value:.2}x"),
        Format::Integer => format!("{value:.0}"),
        Format::Choice(names) => {
            let idx = (value.round() as usize).min(names.len() - 1);
            names[idx].to_string()
        }
    }
}

fn text_to_plain(spec: &ParameterSpec, text: &str) -> PluginResult<f64> {
    let text = text.trim();
    if let Format::Choice(names) = spec.format {
        if let Some(idx) = names.iter().position(|n| n.eq_ignore_ascii_case(text)) {
            return Ok(idx as f64);
        }
    }
    let numeric: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    let mut v: f64 = numeric.parse().map_err(|_| PluginError::InvalidParameter)?;
    if matches!(spec.format, Format::Percent) {
        v /= 100.0;
    }
    Ok(v.clamp(spec.info.min_value, spec.info.max_value))
}

pub(crate) struct OpenConvParamsExtension {
    shared: Arc<SharedState>,
}

impl OpenConvParamsExtension {
    pub(crate) fn new(shared: Arc<SharedState>) -> Self {
        Self { shared }
    }
}

impl PluginParamsExtension for OpenConvParamsExtension {
    fn param_count(&self) -> u32 {
        PARAM_SPECS.len() as u32
    }

    fn param_info(&self, index: u32) -> Option<ParamInfo> {
        PARAM_SPECS.get(index as usize).map(|spec| spec.info)
    }

    fn param_value(&self, param_id: u32) -> PluginResult<f64> {
        param_spec(param_id)?;
        self.shared
            .parameter_value(param_id)
            .map(f64::from)
            .ok_or(PluginError::InvalidParameter)
    }

    fn apply_param_events(&self, events: ParamInputEvents<'_>) -> PluginResult<()> {
        for event in events.values() {
            if self
                .shared
                .set_parameter_value(event.param_id, event.value)
                .is_none()
            {
                wrac_log::rtwarn!(
                    "params.flush: ignoring unknown parameter id={} value={}",
                    event.param_id,
                    event.value
                );
            }
        }
        Ok(())
    }

    fn value_to_text(&self, param_id: u32, value: f64) -> PluginResult<String> {
        Ok(value_to_text(param_spec(param_id)?, value))
    }

    fn text_to_value(&self, param_id: u32, text: &str) -> PluginResult<f64> {
        text_to_plain(param_spec(param_id)?, text)
    }
}
