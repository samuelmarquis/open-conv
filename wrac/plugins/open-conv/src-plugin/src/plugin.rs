//! The plugin contract as seen by the host. Headless: no GUI extension is
//! implemented, so hosts fall back to their generic parameter editor; no
//! note ports (plain `aufx` audio effect).

use std::sync::Arc;

mod audio_ports;
mod params;
mod state;

pub(crate) use params::{
    PARAM_ATTACK_ID, PARAM_BANK_ID, PARAM_BYPASS_ID, PARAM_DRY_ID, PARAM_FADE_ID, PARAM_MODE_ID,
    PARAM_MORPH_ID, PARAM_RELEASE_ID, PARAM_RELOAD_ID, PARAM_SIZE_ID, PARAM_SYM_ID, PARAM_TAILS_ID,
    PARAM_WET_ID, PARAM_ZONES_ID, PARAM_ZONE_GAIN_IDS, PARAM_ZONE_LEVEL_IDS, param_clamp,
    param_default, param_exists, parameter_infos,
};

use audio_ports::{AudioLayoutStore, OpenConvAudioPorts, OpenConvConfigurableAudioPorts};
use params::OpenConvParamsExtension;
use state::OpenConvStateExtension;
use wrac_clap_adapter::{
    AaxDescriptor, AaxStemConfig, ActivateContext, Auv2Descriptor, PluginAudioPortsExtension,
    PluginConfigurableAudioPortsExtension, PluginCore, PluginCoreContext, PluginDescriptor,
    PluginEntry, PluginFactory, PluginFeature, PluginLatencyExtension, PluginParamsExtension,
    PluginResult, PluginStateExtension, Processor, Vst3Descriptor,
};

use crate::audio::OpenConvAudioProcessor;
use crate::state::SharedState;

// Generated from [package.metadata.wrac] in src-plugin/Cargo.toml.
include!(concat!(env!("OUT_DIR"), "/wrac_plugin_products.rs"));

pub(crate) static PLUGIN_ENTRY: OpenConvEntry = OpenConvEntry;

pub(crate) struct OpenConvEntry;

impl PluginEntry for OpenConvEntry {
    fn plugin_factory(&self) -> Option<&dyn PluginFactory> {
        Some(&OPEN_CONV_FACTORY)
    }
}

static OPEN_CONV_FACTORY: OpenConvFactory = OpenConvFactory;

struct OpenConvFactory;

impl PluginFactory for OpenConvFactory {
    fn plugin_count(&self) -> u32 {
        PLUGIN_DESCRIPTORS.len() as u32
    }

    fn plugin_descriptor(&self, index: u32) -> Option<PluginDescriptor> {
        PLUGIN_DESCRIPTORS.get(index as usize).copied()
    }

    fn create_plugin(
        &self,
        plugin_id: &str,
        context: PluginCoreContext,
    ) -> Option<Box<dyn PluginCore>> {
        PLUGIN_DESCRIPTORS
            .iter()
            .find(|descriptor| descriptor.id == plugin_id)
            .map(|descriptor| create_plugin_core(context, *descriptor))
    }
}

/// Constant algorithmic latency: one convolver partition.
struct OpenConvLatency;

impl PluginLatencyExtension for OpenConvLatency {
    fn latency_frames(&self) -> u32 {
        open_conv_engine::DEFAULT_PARTITION as u32
    }
}

pub(crate) struct OpenConvPlugin {
    descriptor: PluginDescriptor,
    shared: Arc<SharedState>,
    audio_layout: Arc<AudioLayoutStore>,
    audio_ports: Arc<OpenConvAudioPorts>,
    configurable_audio_ports: Arc<OpenConvConfigurableAudioPorts>,
    params: Arc<OpenConvParamsExtension>,
    state_extension: Arc<OpenConvStateExtension>,
    latency: Arc<OpenConvLatency>,
}

impl OpenConvPlugin {
    pub(crate) fn new(_context: PluginCoreContext, descriptor: PluginDescriptor) -> Self {
        let shared = Arc::new(SharedState::new());
        let audio_layout = Arc::new(AudioLayoutStore::new(2));
        let audio_ports = Arc::new(OpenConvAudioPorts::new(audio_layout.clone()));
        let configurable_audio_ports =
            Arc::new(OpenConvConfigurableAudioPorts::new(audio_layout.clone()));
        let params = Arc::new(OpenConvParamsExtension::new(shared.clone()));
        let state_extension = Arc::new(OpenConvStateExtension::new(shared.clone()));

        Self {
            descriptor,
            shared,
            audio_layout,
            audio_ports,
            configurable_audio_ports,
            params,
            state_extension,
            latency: Arc::new(OpenConvLatency),
        }
    }
}

pub(crate) fn create_plugin_core(
    context: PluginCoreContext,
    descriptor: PluginDescriptor,
) -> Box<dyn PluginCore> {
    wrac_log::init!(descriptor.name);
    log::debug!(
        "creating plugin core: id={}, name={}",
        descriptor.id,
        descriptor.name
    );
    Box::new(OpenConvPlugin::new(context, descriptor))
}

impl PluginCore for OpenConvPlugin {
    fn activate(&mut self, context: ActivateContext) -> PluginResult<Box<dyn Processor>> {
        let audio_channel_count = self.audio_layout.channel_count();
        log::debug!(
            "activating: plugin_id={}, sample_rate={}, max_frames={}, channels={}",
            self.descriptor.id,
            context.sample_rate,
            context.max_frames_count,
            audio_channel_count
        );
        Ok(Box::new(OpenConvAudioProcessor::new(
            self.shared.clone(),
            audio_channel_count,
            context.sample_rate,
            context.max_frames_count,
        )))
    }

    fn deactivate(&mut self, _processor: Box<dyn Processor>) -> PluginResult<()> {
        // Dropping the processor here (host non-RT context) tears down the
        // worker thread and frees the engine off the audio thread.
        Ok(())
    }

    fn audio_ports(&self) -> Option<Arc<dyn PluginAudioPortsExtension>> {
        Some(self.audio_ports.clone())
    }

    fn configurable_audio_ports(&self) -> Option<Arc<dyn PluginConfigurableAudioPortsExtension>> {
        Some(self.configurable_audio_ports.clone())
    }

    fn params(&self) -> Option<Arc<dyn PluginParamsExtension>> {
        Some(self.params.clone())
    }

    fn state(&self) -> Option<Arc<dyn PluginStateExtension>> {
        Some(self.state_extension.clone())
    }

    fn latency(&self) -> Option<Arc<dyn PluginLatencyExtension>> {
        Some(self.latency.clone())
    }
}
