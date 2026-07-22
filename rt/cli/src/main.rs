//! open-conv offline renderer — drives listening batches before any plugin
//! shell exists (CLI-first, per house methodology).
//!
//! Output is latency-compensated (one partition trimmed from the head) and
//! tail-flushed (input padded with the active tail length of silence).
//!
//! Usage:
//!   open-conv IN.wav OUT.wav [options]
//! Options (defaults in brackets):
//!   --ir0..--ir3 FILE      zone IR wav (quiet→loud; mono or stereo) —
//!                          ANY sample works as a zone's space
//!   --no-ir-norm           skip windowed-spectral normalization of
//!                          loaded IR files (on by default)
//!   --synth-irs            deterministic synthetic 4-zone bank (rooms)
//!   --bank NAME            synthetic bank flavor (implies --synth-irs):
//!                          rooms    quiet=long/dark … loud=short/bright
//!                          subdrop  tuned gliding-sine booms, loud=deepest
//!                          resoroom noise rooms + damped low modes
//!   --nzones N             active zones [as many IRs as given]
//!   --zones "a,b,c,d"      zone centers dBFS ascending [-48,-30,-18,-6]
//!   --gains "g0,g1,…"      per-zone wet gains linear [1,…]
//!   --mode instant|env|xsign  level selector [env] (xsign = legacy alias
//!                          for instant + --sym 1)
//!   --sym F                symmetry 0..1: blend the zone ladder toward
//!                          its mirror on negative half-cycles [0]
//!   --attack MS            envelope attack [5]
//!   --release MS           envelope release [120]
//!   --wet F --dry F        mix gains linear [0.35 / 1.0]
//!   --morph N              IR transition speed, partitions/frame 1..16 [1]
//!   --fade N               per-partition write fade, frames 1..16 [4]
//!   --tails gated|ungated  old-IR policy: stream-replace vs ring-out [gated]
//!   --ring N               ring-out voices kept per zone (ungated) 1..8 [8]
//!   --size F               IR stretch ratio [1.0]
//!   --size-sweep "a:b"     sweep size a→b across the input duration
//!   --partition N          partition size, power of two [256]
//!   --tail SECS            extra flush [active tail + 0.25]
//!   --normalize            peak-normalize output to 0.97
//!   --viz-dump FILE.jsonl  drain the viz ring to JSON lines

use open_conv_engine::{Engine, EngineParams, LevelMode, MAX_ZONES, TailMode};
use std::fs::File;
use std::io::{BufWriter, Write as _};

fn die(msg: &str) -> ! {
    eprintln!("open-conv: {msg}");
    std::process::exit(1);
}

fn read_wav(path: &str) -> (Vec<Vec<f32>>, f64) {
    let mut r = hound::WavReader::open(path)
        .unwrap_or_else(|e| die(&format!("cannot open {path}: {e}")));
    let spec = r.spec();
    let ch = spec.channels as usize;
    let inter: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.unwrap() as f32 * scale).collect()
        }
    };
    let n = inter.len() / ch;
    let mut chans = vec![Vec::with_capacity(n); ch];
    for (i, v) in inter.into_iter().enumerate() {
        chans[i % ch].push(v);
    }
    (chans, spec.sample_rate as f64)
}

fn write_wav(path: &str, chans: &[Vec<f32>], sr: f64) {
    let spec = hound::WavSpec {
        channels: chans.len() as u16,
        sample_rate: sr as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)
        .unwrap_or_else(|e| die(&format!("cannot create {path}: {e}")));
    let n = chans[0].len();
    for i in 0..n {
        for c in chans {
            w.write_sample(c[i]).unwrap();
        }
    }
    w.finalize().unwrap();
}

use open_conv_engine::banks::{self, windowed_spectral_norm};

fn synth_bank(bank: &str, zone: usize, sr: f64) -> Vec<Vec<f32>> {
    match banks::Bank::from_name(bank) {
        Some(b) => banks::render_bank(b, zone, sr),
        None => die(&format!("unknown bank {bank} (rooms|subdrop|resoroom)")),
    }
}

fn parse_list(s: &str) -> Vec<f64> {
    s.split(',')
        .map(|t| t.trim().parse().unwrap_or_else(|_| die(&format!("bad number in list: {t}"))))
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pos = Vec::new();
    let mut irs: [Option<String>; MAX_ZONES] = [None, None, None, None];
    let mut synth_irs = false;
    let mut ir_norm = true;
    let mut bank = String::from("rooms");
    let mut nzones: Option<usize> = None;
    let mut zones: Option<Vec<f64>> = None;
    let mut gains: Option<Vec<f64>> = None;
    let mut mode = LevelMode::Envelope;
    let mut attack = 5.0;
    let mut release = 120.0;
    let mut wet = 0.35;
    let mut sym: Option<f64> = None;
    let mut morph = 1.0;
    let mut fade = 4.0;
    let mut ring = 8.0;
    let mut tails = TailMode::Gated;
    let mut sym_from_mode = false;
    let mut dry = 1.0;
    let mut size = 1.0;
    let mut size_sweep: Option<(f64, f64)> = None;
    let mut partition = 256usize;
    let mut tail: Option<f64> = None;
    let mut normalize = false;
    let mut viz_dump: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut val = || {
            i += 1;
            args.get(i)
                .unwrap_or_else(|| die(&format!("{a} needs a value")))
                .clone()
        };
        match a {
            "--ir" | "--ir0" => irs[0] = Some(val()),
            "--ir1" => irs[1] = Some(val()),
            "--ir2" => irs[2] = Some(val()),
            "--ir3" => irs[3] = Some(val()),
            "--synth-irs" => synth_irs = true,
            "--no-ir-norm" => ir_norm = false,
            "--bank" => {
                bank = val();
                synth_irs = true;
            }
            "--nzones" => nzones = Some(val().parse().unwrap_or_else(|_| die("bad --nzones"))),
            "--zones" => zones = Some(parse_list(&val())),
            "--gains" => gains = Some(parse_list(&val())),
            "--mode" => {
                mode = match val().as_str() {
                    "instant" => LevelMode::Instant,
                    "env" | "envelope" => LevelMode::Envelope,
                    "xsign" => {
                        // legacy alias: instant with full symmetry
                        sym_from_mode = true;
                        LevelMode::Instant
                    }
                    m => die(&format!("unknown mode {m}")),
                }
            }
            "--attack" => attack = val().parse().unwrap_or_else(|_| die("bad --attack")),
            "--release" => release = val().parse().unwrap_or_else(|_| die("bad --release")),
            "--wet" => wet = val().parse().unwrap_or_else(|_| die("bad --wet")),
            "--sym" => sym = Some(val().parse().unwrap_or_else(|_| die("bad --sym"))),
            "--morph" => morph = val().parse().unwrap_or_else(|_| die("bad --morph")),
            "--fade" => fade = val().parse().unwrap_or_else(|_| die("bad --fade")),
            "--ring" => ring = val().parse().unwrap_or_else(|_| die("bad --ring")),
            "--tails" => {
                tails = match val().as_str() {
                    "gated" => TailMode::Gated,
                    "ungated" => TailMode::Ungated,
                    t => die(&format!("unknown tails mode {t}")),
                }
            }
            "--dry" => dry = val().parse().unwrap_or_else(|_| die("bad --dry")),
            "--size" => size = val().parse().unwrap_or_else(|_| die("bad --size")),
            "--size-sweep" => {
                let v = val();
                let (a, b) = v
                    .split_once(':')
                    .unwrap_or_else(|| die("--size-sweep wants a:b"));
                size_sweep = Some((
                    a.parse().unwrap_or_else(|_| die("bad sweep start")),
                    b.parse().unwrap_or_else(|_| die("bad sweep end")),
                ));
            }
            "--partition" => partition = val().parse().unwrap_or_else(|_| die("bad --partition")),
            "--tail" => tail = Some(val().parse().unwrap_or_else(|_| die("bad --tail"))),
            "--normalize" => normalize = true,
            "--viz-dump" => viz_dump = Some(val()),
            _ if a.starts_with("--") => die(&format!("unknown flag {a}")),
            _ => pos.push(a.to_string()),
        }
        i += 1;
    }
    if pos.len() != 2 {
        die("usage: open-conv IN.wav OUT.wav [options] (see source header)");
    }

    let (input, sr) = read_wav(&pos[0]);
    let in_ch = input.len().min(2);
    let n_in = input[0].len();

    // --- IR bank (read first: IR width co-decides engine width) ---------
    let mut ir_data: Vec<Option<(Vec<Vec<f32>>, f64)>> = (0..MAX_ZONES).map(|_| None).collect();
    let mut n_loaded = 0usize;
    let mut ir_ch = 1usize;
    for z in 0..MAX_ZONES {
        if let Some(path) = &irs[z] {
            let (mut ir, ir_sr) = read_wav(path);
            if ir_norm {
                // Same law as the synth banks (Defect 001): bound the
                // ~85 ms burst gain so arbitrary dropped-in samples land
                // at a sane wet level regardless of their own loudness.
                for ch in &mut ir {
                    windowed_spectral_norm(ch, ir_sr);
                }
            }
            ir_ch = ir_ch.max(ir.len().min(2));
            ir_data[z] = Some((ir, ir_sr));
            n_loaded = n_loaded.max(z + 1);
        } else if synth_irs {
            let ir = synth_bank(&bank, z, sr); // stereo-decorrelated pair
            ir_ch = ir_ch.max(ir.len().min(2));
            ir_data[z] = Some((ir, sr));
            n_loaded = n_loaded.max(z + 1);
        }
    }
    if n_loaded == 0 {
        die("no IRs: give --ir0..--ir3 or --synth-irs");
    }

    // Mono input through a stereo IR renders stereo (dry broadcast to
    // both channels, wet decorrelated per channel) — a mono-in/stereo-out
    // convolution reverb, not a hard-panned mono file.
    let channels = in_ch.max(ir_ch).min(2);

    // Max stretched IR length bounds the prealloc.
    let max_sweep = size_sweep.map(|(a, b)| a.max(b)).unwrap_or(size).max(size);
    let mut engine = Engine::new_sized(sr, channels, partition, 8.0 * max_sweep.max(1.0));
    for (z, slot) in ir_data.iter_mut().enumerate() {
        if let Some((ir, ir_sr)) = slot.take() {
            engine.set_source_ir(z, ir, ir_sr, size);
        }
    }

    let mut p = EngineParams {
        n_zones: nzones.unwrap_or(n_loaded).clamp(1, MAX_ZONES),
        level_mode: mode,
        attack_ms: attack,
        release_ms: release,
        wet,
        sym: sym.unwrap_or(if sym_from_mode { 1.0 } else { 0.0 }),
        morph,
        fade_frames: fade,
        tails,
        ring,
        dry,
        size,
        ..Default::default()
    };
    if let Some(zs) = zones {
        for (i, v) in zs.iter().take(MAX_ZONES).enumerate() {
            p.zone_db[i] = *v;
        }
    }
    if let Some(gs) = gains {
        for (i, v) in gs.iter().take(MAX_ZONES).enumerate() {
            p.zone_gain[i] = *v;
        }
    }

    // --- render ----------------------------------------------------------
    let tail_secs = tail.unwrap_or(engine.tail_samples() as f64 / sr + 0.25);
    let n_total = n_in + (tail_secs * sr) as usize + engine.latency_samples();
    let mut out = vec![Vec::with_capacity(n_total); channels];
    let mut viz = viz_dump.map(|path| {
        BufWriter::new(File::create(&path).unwrap_or_else(|e| die(&format!("viz-dump: {e}"))))
    });

    let hop = partition;
    let mut buf = vec![vec![0.0f32; hop]; channels];
    let mut done = 0usize;
    while done < n_total {
        let n = hop.min(n_total - done);
        for c in 0..channels {
            let src = c.min(in_ch - 1); // broadcast mono input
            for j in 0..n {
                let idx = done + j;
                buf[c][j] = if idx < n_in { input[src][idx] } else { 0.0 };
            }
            for j in n..hop {
                buf[c][j] = 0.0;
            }
        }
        if let Some((a, b)) = size_sweep {
            let t = (done as f64 / n_in.max(1) as f64).min(1.0);
            p.size = a + (b - a) * t;
        }
        engine.service(&p);
        {
            let mut io: Vec<&mut [f32]> = buf.iter_mut().map(|c| c.as_mut_slice()).collect();
            engine.process_block(&mut io, &p);
        }
        for c in 0..channels {
            out[c].extend_from_slice(&buf[c][..n]);
        }
        if let Some(w) = &mut viz {
            while let Some(f) = engine.viz_pop() {
                writeln!(
                    w,
                    "{{\"t\":{},\"in_peak_db\":{:.2},\"env_db\":{:.2},\"weights\":[{:.4},{:.4},{:.4},{:.4}],\"zone_energy\":[{:.6},{:.6},{:.6},{:.6}],\"swap_progress\":{:.4}}}",
                    f.t, f.in_peak_db, f.env_db,
                    f.weights[0], f.weights[1], f.weights[2], f.weights[3],
                    f.zone_energy[0], f.zone_energy[1], f.zone_energy[2], f.zone_energy[3],
                    f.swap_progress
                )
                .unwrap();
            }
        }
        done += n;
    }

    // Latency trim (zero-tail flush already included in n_total).
    let lat = engine.latency_samples();
    for c in &mut out {
        c.drain(..lat);
    }

    if normalize {
        let peak = out
            .iter()
            .flat_map(|c| c.iter())
            .fold(0.0f32, |m, &v| m.max(v.abs()));
        if peak > 0.0 {
            let g = 0.97 / peak;
            for c in &mut out {
                for v in c {
                    *v *= g;
                }
            }
        }
    }

    write_wav(&pos[1], &out, sr);
    eprintln!(
        "open-conv: {} ch, {:.1}s in → {:.1}s out, {} zones, latency {} smp, tail {:.2}s",
        channels,
        n_in as f64 / sr,
        out[0].len() as f64 / sr,
        p.n_zones,
        lat,
        tail_secs
    );
}
