# Probes

Small committed-in-script, generated-locally synthetic inputs — each
engineered to answer one question. Regenerate with
`python tools/make_probes.py` (48 kHz mono float32; wavs untracked).

| probe | the question |
|---|---|
| `staircase.wav` | Pink bursts −54→−6 dBFS: do the four zones read as four distinct rooms as level climbs? |
| `sineburst.wav` | 1 kHz bursts in 3 dB steps: zipper/waveshaper artifacts at zone boundaries (instant vs envelope mode)? |
| `impulses.wav` | Single-sample impulses at the zone centers + soft noise clicks: does each level fire its own space; how does the envelope selector treat one-sample transients? |
| `sweepbed.wav` | Sustained detuned pad, slow level wobble: is a `--size-sweep` render click-free; is zone-drift on sustained material musical? |
| `thumps.wav` | 55 Hz Hann bursts at zone centers + 8-step velocity ramp: the honest "impulse" for tuned/resonant banks (a 1-sample Dirac through a bounded-gain resonator is physically near-silent — Q physics, not a bug). |

Commercial-derived material and reference renders stay out of git
(`testdata/material/`, `testdata/reference/`), protocols documented when
they appear.
