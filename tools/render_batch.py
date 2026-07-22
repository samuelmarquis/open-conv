#!/usr/bin/env python3
"""Batch renderer + machine gates for listening batches.

Usage (repo root, inside nix develop):
    python tools/render_batch.py batch003 batch004

Per batch spec below this renders every file, then enforces two gates that
exist because each caught a real shipped defect (see LISTENING-LOG):

- PEAK gate: any render peaking above -1 dBFS gets its --wet auto-trimmed
  and re-rendered (up to 3 iterations). Peaks alone once hid defect #2.
- WET-PRESENCE gate: representative configs are re-rendered with --dry 0
  into out/.check/ and their wet RMS compared against the dry source;
  outside -36..+6 dB fails the batch. This is the gate that would have
  caught batches 003/004 shipping with inaudible reverb.

Dry controls (--wet 0) are rendered into every batch folder (standing
convention: Finder-friendly, sample-aligned with the batch's primary
config).
"""

import os
import subprocess
import sys

import numpy as np
import soundfile as sf

BIN = "rt/target/release/open-conv"
MAT = "testdata/material"
PRB = "testdata/probes"
PEAK_CEIL_DB = -1.0
# All batch renders (incl. dry controls) carry dry at 0.7 (-3 dB): the wet
# is the star of this instrument and needs headroom; uniform dry keeps
# every A/B level-matched.
DRY = 0.7

SRC = {
    "kicks": f"{MAT}/kicks.wav",
    "loop": f"{MAT}/loop_aphex90.wav",
    "scrape": f"{MAT}/scrape.wav",
    "impulses": f"{PRB}/impulses.wav",
    "staircase": f"{PRB}/staircase.wav",
    "sineburst": f"{PRB}/sineburst.wav",
    "sweepbed": f"{PRB}/sweepbed.wav",
    "thumps": f"{PRB}/thumps.wav",
}

IRD = f"{MAT}/irs"
USER_IRS = ["--ir0", f"{IRD}/ir_murky.wav", "--ir1", f"{IRD}/ir_prism.wav",
            "--ir2", f"{IRD}/ir_press.wav", "--ir3", f"{IRD}/ir_scrapebit.wav"]

# (out_name, source_key, wet, extra_args, wet_check)
BATCHES = {
    "batch003": [
        ("b003_kicks_subdrop_instant", "kicks", 0.6,
         ["--bank", "subdrop", "--mode", "instant", "--viz-dump", "out/batch003/kicks_subdrop.jsonl"], True),
        ("b003_kicks_subdrop_env", "kicks", 0.6, ["--bank", "subdrop", "--mode", "env", "--release", "200"], False),
        ("b003_kicks_subdrop_xsign", "kicks", 0.6, ["--bank", "subdrop", "--mode", "xsign"], False),
        ("b003_kicks_subdrop_size", "kicks", 0.6,
         ["--bank", "subdrop", "--mode", "instant", "--size-sweep", "0.7:1.8"], False),
        ("b003_kicks_resoroom_instant", "kicks", 0.5, ["--bank", "resoroom", "--mode", "instant"], True),
        ("b003_loop_subdrop_instant", "loop", 0.5, ["--bank", "subdrop", "--mode", "instant"], True),
        ("b003_loop_resoroom_xsign", "loop", 0.5, ["--bank", "resoroom", "--mode", "xsign"], False),
        ("b003_scrape_subdrop_instant", "scrape", 0.5, ["--bank", "subdrop", "--mode", "instant"], True),
        # dry controls: <source>_dry so Finder sorts them beside their
        # source's variants (standing convention)
        ("b003_kicks_dry", "kicks", 0.0, ["--bank", "subdrop"], False),
        ("b003_loop_dry", "loop", 0.0, ["--bank", "subdrop"], False),
        ("b003_scrape_dry", "scrape", 0.0, ["--bank", "subdrop"], False),
    ],
    "batch004": (
        [(f"b004_{s}_subdrop_{m}", s, 0.5, ["--bank", "subdrop", "--mode", mm] + ex, chk)
         for s in ("scrape", "loop")
         for (m, mm, ex, chk) in [
             ("instant", "instant", [], True),
             ("env", "env", ["--release", "200"], False),
             ("xsign", "xsign", [], False),
             ("size", "instant", ["--size-sweep", "0.7:1.8"], False),
         ]]
        + [(f"b004_{s}_resoroom_instant", s, 0.5, ["--bank", "resoroom", "--mode", "instant"], s == "scrape")
           for s in ("scrape", "loop")]
        + [
            ("b004_thumps_subdrop_instant", "thumps", 0.6, ["--bank", "subdrop", "--mode", "instant"], True),
            ("b004_thumps_subdrop_xsign", "thumps", 0.6, ["--bank", "subdrop", "--mode", "xsign"], False),
            ("b004_thumps_resoroom_instant", "thumps", 0.6, ["--bank", "resoroom", "--mode", "instant"], False),
            ("b004_imp_subdrop_instant", "impulses", 0.6, ["--bank", "subdrop", "--mode", "instant"], True),
            ("b004_imp_resoroom_instant", "impulses", 0.6, ["--bank", "resoroom", "--mode", "instant"], False),
            ("b004_stair_subdrop_instant", "staircase", 0.6, ["--bank", "subdrop", "--mode", "instant"], False),
            ("b004_stair_subdrop_env", "staircase", 0.6, ["--bank", "subdrop", "--mode", "env", "--release", "200"], False),
            # sineburst is 1 kHz — in-band for rooms, NOT for subdrop
            # (a sub bank fed 1 kHz is silent by design; caught by the
            # wet-presence gate). The subdrop xsign isolator is thumps.
            ("b004_sine_rooms_instant", "sineburst", 0.6, ["--bank", "rooms", "--mode", "instant"], True),
            ("b004_sine_rooms_env", "sineburst", 0.6, ["--bank", "rooms", "--mode", "env"], False),
            ("b004_sweepbed_subdrop_size", "sweepbed", 0.6,
             ["--bank", "subdrop", "--mode", "instant", "--size-sweep", "0.7:1.8"], False),
        ]
        + [(f"b004_{stem}_dry", s, 0.0, ["--bank", "subdrop"], False)
           for (stem, s) in [("scrape", "scrape"), ("loop", "loop"),
                             ("imp", "impulses"), ("stair", "staircase"),
                             ("sine", "sineburst"), ("sweepbed", "sweepbed"),
                             ("thumps", "thumps")]]
    ),
    # symmetry knob ladder (verdict: xsign promoted from mode to 0..1 dial)
    "batch005": (
        [(f"b005_scrape_subdrop_sym{s:03d}", "scrape", 0.5,
          ["--bank", "subdrop", "--mode", "instant", "--sym", f"{s/100:.2f}"], s == 100)
         for s in (0, 35, 70, 100)]
        + [
            ("b005_loop_subdrop_sym050", "loop", 0.5,
             ["--bank", "subdrop", "--mode", "instant", "--sym", "0.5"], False),
            # envelope + sym: the meat without the instant-mode fizz?
            ("b005_loop_subdrop_envsym", "loop", 0.5,
             ["--bank", "subdrop", "--mode", "env", "--release", "200", "--sym", "1.0"], True),
            ("b005_kicks_subdrop_envsym", "kicks", 0.5,
             ["--bank", "subdrop", "--mode", "env", "--release", "200", "--sym", "1.0"], False),
            ("b005_thumps_subdrop_sym050", "thumps", 0.6,
             ["--bank", "subdrop", "--mode", "instant", "--sym", "0.5"], False),
        ]
        + [(f"b005_{s}_dry", s, 0.0, ["--bank", "subdrop"], False)
           for s in ("scrape", "loop", "kicks", "thumps")]
    ),
    # user-sample IR bank: zone spaces sliced from ~/Dropbox/Samples
    # (murky sustain / prism scrambler / transient press / beat scrape),
    # quiet->loud. Proves "drop any sample in" end to end.
    "batch006": (
        [(f"b006_{src}_yourbank_{m}", src, 0.5, USER_IRS + ["--mode", mm] + ex, chk)
         for src in ("loop", "kicks", "scrape")
         for (m, mm, ex, chk) in [
             ("instant", "instant", [], src == "loop"),
             ("envsym", "env", ["--release", "200", "--sym", "1.0"], False),
             ("sym050", "instant", ["--sym", "0.5"], False),
             ("size", "instant", ["--size-sweep", "0.8:1.6"], False),
         ]]
        + [("b006_thumps_yourbank_instant", "thumps", 0.6, USER_IRS + ["--mode", "instant"], True)]
        + [(f"b006_{s}_dry", s, 0.0, USER_IRS, False)
           for s in ("loop", "kicks", "scrape", "thumps")]
    ),
}


def run(args):
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"render failed: {' '.join(args)}\n{r.stderr}")


def peak_db(path):
    x, _ = sf.read(path)
    return 20 * np.log10(np.max(np.abs(x)) + 1e-12)


def rms(x):
    return np.sqrt(np.mean(np.asarray(x, float) ** 2))


def main(batch_names):
    failures = []
    for bname in batch_names:
        spec = BATCHES[bname]
        outdir = f"out/{bname}"
        os.makedirs(outdir, exist_ok=True)
        os.makedirs("out/.check", exist_ok=True)
        print(f"== {bname}: {len(spec)} renders ==")
        for name, src, wet, extra, wet_check in spec:
            out = f"{outdir}/{name}.wav"
            w = wet
            for attempt in range(8):
                run([BIN, SRC[src], out, "--wet", f"{w:.3f}", "--dry", str(DRY)] + extra)
                p = peak_db(out)
                if p <= PEAK_CEIL_DB or wet == 0.0:
                    break
                # over-relaxed step: wet feeds a tanh, so output responds
                # sub-linearly to drive trims
                w *= 10 ** (1.5 * (PEAK_CEIL_DB - p) / 20)
            trim = f" (wet {wet}->{w:.2f})" if w != wet else ""
            print(f"  {name}: peak {p:+6.2f} dBFS{trim}")
            if p > PEAK_CEIL_DB + 0.25:  # trim converges asymptotically; ±¼ dB is fine
                failures.append(f"{name}: peak {p:+.2f}")
            if wet_check:
                chk = f"out/.check/{bname}_{name}.wav"
                run([BIN, SRC[src], chk, "--wet", f"{w:.3f}", "--dry", "0"] + extra)
                # (wet-presence measured at the shipped wet level)
                wv, _ = sf.read(chk)
                dv, _ = sf.read(SRC[src])
                rel = 20 * np.log10(rms(wv) / max(rms(dv), 1e-12))
                ok = -36 < rel < 6
                print(f"    wet-presence: {rel:+.1f} dB {'PASS' if ok else 'FAIL'}")
                if not ok:
                    failures.append(f"{name}: wet-presence {rel:+.1f} dB")
    if failures:
        sys.exit("GATE FAILURES:\n  " + "\n  ".join(failures))
    print("ALL GATES PASS")


if __name__ == "__main__":
    main(sys.argv[1:] or list(BATCHES))
