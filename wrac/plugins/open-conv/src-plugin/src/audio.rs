//! The audio-thread processor: drives `open_conv_engine::Engine`, plus the
//! IR worker thread — the non-RT half of the engine's threading contract.
//!
//! The worker owns the IR sources (synth banks or the watched folder
//! `~/Music/open-conv/zone{1..4}.wav`), renders [`PartitionSet`]s with an
//! [`IrRenderer`], and sends them over a channel; the audio thread's only
//! IR work is the move-only [`Engine::queue_partition_set`] handoff and
//! shipping spent/rejected sets back for off-thread dropping.
//!
//! Control protocol: desired-state reconciliation — the audio thread sends
//! coalesced Sync snapshots, the worker renders only the newest, and the
//! engine displaces in-flight swaps — latest request always wins. mpsc
//! sends allocate (control-edge frequency only; lock-free ring = panel milestone).

use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};


use open_conv_engine::{
    DEFAULT_MAX_IR_SECONDS, DEFAULT_PARTITION, Engine, IrRenderer, MAX_ZONES, PartitionSet, banks,
};
use wrac_clap_adapter::{
    AudioPortChannels, InputEvent, PluginResult, ProcessContext, ProcessStatus, Processor,
};

use crate::state::SharedState;

const MAX_CHANNELS: usize = 2;
/// Wall-clock debounce between IR re-render requests — applies to size
/// sweeps AND bank flips (batch-007 skitter report: an undebounced jittery
/// LFO on IR Bank flooded the worker with per-block reloads). Changes are
/// deferred, never dropped; ~20 Hz keeps LFO bank-skitter musical while
/// bounding worker thrash. Block-size independent, so live and offline
/// render behave identically.
const SYNC_DEBOUNCE_SECS: f64 = 0.05;

enum ToWorker {
    /// The latest desired IR state. The worker coalesces a queue of these
    /// to the newest one — desired-state reconciliation, so no change can
    /// ever be dropped by unlucky timing (the bug behind the batch-007
    /// automation non-determinism report). `load_gen` bumps on bank
    /// changes / reload edges to force a source re-read.
    Sync { bank: usize, size: f64, load_gen: u64 },
    /// Drop a spent/displaced set off the audio thread.
    Dispose(PartitionSet),
}

struct FromWorker {
    zone: usize,
    set: PartitionSet,
}

pub(crate) struct OpenConvAudioProcessor {
    shared: Arc<SharedState>,
    engine: Engine,
    channels: usize,
    max_frames: usize,
    /// Flat scratch: `channels * max_frames`, split per channel each block.
    scratch: Vec<f32>,
    to_worker: Sender<ToWorker>,
    from_worker: Receiver<FromWorker>,
    _worker: std::thread::JoinHandle<()>,
    synced_bank: usize,
    synced_size: f64,
    last_reload: bool,
    dirty_load: bool,
    load_gen: u64,
    samples_since_sync: usize,
    debounce_samples: usize,
}

impl OpenConvAudioProcessor {
    pub(crate) fn new(
        shared: Arc<SharedState>,
        channels: u32,
        sample_rate: f64,
        max_frames: u32,
    ) -> Self {
        let channels = (channels as usize).clamp(1, MAX_CHANNELS);
        let max_frames = max_frames as usize;
        let engine = Engine::new(sample_rate, channels);
        let (to_worker, worker_rx) = channel::<ToWorker>();
        let (worker_tx, from_worker) = channel::<FromWorker>();
        let worker = std::thread::Builder::new()
            .name("open-conv-ir".into())
            .spawn(move || worker_main(worker_rx, worker_tx, sample_rate))
            .expect("spawn IR worker");

        let params = shared.engine_params();
        let bank = shared.bank_index();
        let _ = to_worker.send(ToWorker::Sync {
            bank,
            size: params.size,
            load_gen: 0,
        });

        Self {
            shared,
            engine,
            channels,
            max_frames,
            scratch: vec![0.0; channels * max_frames],
            to_worker,
            from_worker,
            _worker: worker,
            synced_bank: bank,
            synced_size: params.size,
            last_reload: false,
            dirty_load: false,
            load_gen: 0,
            samples_since_sync: 0,
            debounce_samples: (SYNC_DEBOUNCE_SECS * sample_rate) as usize,
        }
    }
}

impl Processor for OpenConvAudioProcessor {
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn process(&mut self, mut context: ProcessContext<'_>) -> PluginResult<ProcessStatus> {
        // 1) Drain this block's events into shared params.
        for event in context.events.input.iter() {
            if let InputEvent::ParamValue(e) = event {
                let _ = self.shared.set_parameter_value(e.param_id, e.value);
            }
        }
        let params = self.shared.engine_params();

        // 2) Control changes → one coalesced desired-state Sync, debounced
        //    by wall clock. Deferred, never dropped: dirty flags survive
        //    the debounce window (allocating send, change-rate only).
        let bank = self.shared.bank_index();
        let reload = self.shared.reload_on();
        if bank != self.synced_bank || (reload && !self.last_reload) {
            self.dirty_load = true;
        }
        self.last_reload = reload;
        let dirty_size = (params.size - self.synced_size).abs() > 1e-3;
        if (self.dirty_load || dirty_size) && self.samples_since_sync >= self.debounce_samples {
            if self.dirty_load {
                self.load_gen += 1;
                self.dirty_load = false;
            }
            self.synced_bank = bank;
            self.synced_size = params.size;
            self.samples_since_sync = 0;
            let _ = self.to_worker.send(ToWorker::Sync {
                bank,
                size: params.size,
                load_gen: self.load_gen,
            });
        }

        // 3) Accept freshly rendered partition sets (move-only handoff).
        //    Channel order is FIFO, so the latest render always lands last;
        //    an in-flight swap is displaced (engine restarts its cursor).
        while let Ok(FromWorker { zone, set }) = self.from_worker.try_recv() {
            if let Err(set) = self.engine.queue_partition_set(zone, set, &params) {
                // Geometry mismatch — should be impossible (renderer
                // built from the same engine); drop off-thread.
                wrac_log::rtwarn!("rejected partition set for zone {zone}");
                let _ = self.to_worker.send(ToWorker::Dispose(set));
            }
        }
        // Ship spent sets off-thread for dropping (displaced pendings,
        // finished/evicted ring voices, completed swaps).
        for zone in 0..MAX_ZONES {
            while let Some(set) = self.engine.take_retired(zone) {
                let _ = self.to_worker.send(ToWorker::Dispose(set));
            }
        }

        let frames = (context.frames_count as usize).min(self.max_frames);
        self.samples_since_sync = self.samples_since_sync.saturating_add(frames);

        // 4) Copy input into per-channel scratch.
        {
            let Some(mut port) = context.audio.port_pair(0) else {
                return Ok(ProcessStatus::Continue);
            };
            match port.channels()? {
                AudioPortChannels::F32(mut chans) => {
                    for ci in 0..self.channels {
                        let dst = &mut self.scratch[ci * self.max_frames..][..frames];
                        if let Some(pair) = chans.channel_pair(ci) {
                            if let Some(input) = pair.input() {
                                dst.copy_from_slice(&input[..frames]);
                            }
                        }
                    }
                }
                AudioPortChannels::F64(mut chans) => {
                    for ci in 0..self.channels {
                        let dst = &mut self.scratch[ci * self.max_frames..][..frames];
                        if let Some(pair) = chans.channel_pair(ci) {
                            if let Some(input) = pair.input() {
                                for (d, s) in dst.iter_mut().zip(input[..frames].iter()) {
                                    *d = *s as f32;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 5) Run the engine in place on the scratch channels.
        {
            let (a, b) = self.scratch.split_at_mut(self.max_frames);
            if self.channels == 1 {
                let mut io: [&mut [f32]; 1] = [&mut a[..frames]];
                self.engine.process_block(&mut io, &params);
            } else {
                let mut io: [&mut [f32]; 2] = [&mut a[..frames], &mut b[..frames]];
                self.engine.process_block(&mut io, &params);
            }
        }
        // Headless v0: drain and discard viz frames (the ring must not
        // saturate; the panel milestone will publish them instead).
        while self.engine.viz_pop().is_some() {}

        // 6) Copy scratch to the output channels.
        {
            let Some(mut port) = context.audio.port_pair(0) else {
                return Ok(ProcessStatus::Continue);
            };
            match port.channels()? {
                AudioPortChannels::F32(mut chans) => {
                    for ci in 0..chans.channel_pair_count() {
                        let src = &self.scratch[(ci.min(self.channels - 1)) * self.max_frames..]
                            [..frames];
                        if let Some(mut pair) = chans.channel_pair(ci) {
                            if let Some(output) = pair.output_mut() {
                                output[..frames].copy_from_slice(src);
                            }
                        }
                    }
                }
                AudioPortChannels::F64(mut chans) => {
                    for ci in 0..chans.channel_pair_count() {
                        let src = &self.scratch[(ci.min(self.channels - 1)) * self.max_frames..]
                            [..frames];
                        if let Some(mut pair) = chans.channel_pair(ci) {
                            if let Some(output) = pair.output_mut() {
                                for (d, s) in output[..frames].iter_mut().zip(src.iter()) {
                                    *d = *s as f64;
                                }
                            }
                        }
                    }
                }
            }
        }

        // The reverb tail keeps running.
        Ok(ProcessStatus::Continue)
    }
}

// ---------------------------------------------------------------------
// IR worker (non-RT)
// ---------------------------------------------------------------------

type Sources = [Option<(Vec<Vec<f32>>, f64)>; MAX_ZONES];

fn worker_main(rx: Receiver<ToWorker>, tx: Sender<FromWorker>, sr: f64) {
    let renderer = IrRenderer::new(sr, DEFAULT_PARTITION, DEFAULT_MAX_IR_SECONDS);
    let mut sources: Sources = std::array::from_fn(|_| None);
    let mut loaded_gen: u64 = u64::MAX; // force the first load
    while let Ok(first) = rx.recv() {
        // Coalesce everything queued right now down to the newest Sync;
        // disposals are handled inline. Reconciliation, not a task queue.
        let mut latest: Option<(usize, f64, u64)> = None;
        let mut msg = first;
        loop {
            match msg {
                ToWorker::Sync {
                    bank,
                    size,
                    load_gen,
                } => latest = Some((bank, size, load_gen)),
                ToWorker::Dispose(set) => drop(set),
            }
            match rx.try_recv() {
                Ok(m) => msg = m,
                Err(_) => break,
            }
        }
        if let Some((bank, size, load_gen)) = latest {
            if load_gen != loaded_gen {
                load_sources(bank, sr, &mut sources);
                loaded_gen = load_gen;
            }
            render_all(&renderer, &sources, sr, size, &tx);
        }
    }
}

fn render_all(
    renderer: &IrRenderer,
    sources: &Sources,
    sr: f64,
    size: f64,
    tx: &Sender<FromWorker>,
) {
    for (zone, src) in sources.iter().enumerate() {
        let set = match src {
            Some((data, ir_sr)) => renderer.render(data, *ir_sr, size),
            // Empty slot: stream a silent 1-tap IR to clear the branch.
            None => renderer.render(&[vec![0.0]], sr, size),
        };
        let _ = tx.send(FromWorker { zone, set });
    }
}

fn load_sources(bank: usize, sr: f64, sources: &mut Sources) {
    match banks::Bank::from_index(bank) {
        Some(b) => {
            for (zone, slot) in sources.iter_mut().enumerate() {
                *slot = Some((banks::render_bank(b, zone, sr), sr));
            }
        }
        None => {
            // Folder mode: ~/Music/open-conv/zone{1..4}.wav
            let dir = std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join("Music").join("open-conv"))
                .unwrap_or_else(|_| std::path::PathBuf::from("open-conv-irs"));
            let _ = std::fs::create_dir_all(&dir);
            for (zone, slot) in sources.iter_mut().enumerate() {
                let path = dir.join(format!("zone{}.wav", zone + 1));
                *slot = read_wav(&path).map(|(mut data, ir_sr)| {
                    for ch in &mut data {
                        banks::windowed_spectral_norm(ch, ir_sr);
                    }
                    (data, ir_sr)
                });
                if slot.is_none() {
                    log::info!("folder bank: no {} — zone {} silent", path.display(), zone + 1);
                }
            }
        }
    }
}

fn read_wav(path: &std::path::Path) -> Option<(Vec<Vec<f32>>, f64)> {
    let mut r = hound::WavReader::open(path).ok()?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let inter: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| s as f32 * scale)
                .collect()
        }
    };
    if inter.is_empty() || ch == 0 {
        return None;
    }
    let n = inter.len() / ch;
    let mut chans = vec![Vec::with_capacity(n); ch];
    for (i, v) in inter.into_iter().enumerate() {
        chans[i % ch].push(v);
    }
    Some((chans, spec.sample_rate as f64))
}
