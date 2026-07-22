//! OpenConv plugin shell (WRAC stack): thin host contract around
//! `open-conv-engine`. Headless v0 — no GUI extension, so hosts present
//! their generic parameter editor; the native panel is a later milestone.
//!
//! Nothing algorithmic lives here (template law). The one structural
//! addition over the opq reference shell is the IR worker thread in
//! `audio.rs`: bank synthesis / folder wav loading / size re-renders all
//! happen off the audio thread, streaming [`open_conv_engine::PartitionSet`]s
//! into the engine via its move-only handoff.

mod audio;
mod plugin;
mod state;

// Export the CLAP entry point. The adapter owns the C ABI and calls the
// safe Rust entry.
wrac_clap_adapter::export_clap_entry! {
    entry: &crate::plugin::PLUGIN_ENTRY,
}
