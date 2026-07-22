# OpenConv — parameter reference

*The plugin's manual (`manual_url` points here). Hosts' generic UIs can't
display per-parameter descriptions — no CLAP/VST3/AU API exists for it —
so this document is the info text until the native panel (with hover
captions) lands. Parameters are listed in host order; the `module` column
is CLAP's grouping path (Bitwig shows groups; Ableton lists flat).*

**The two ideas, in one paragraph:** OpenConv splits your signal into up
to four *level zones* (quiet → loud) and convolves each zone with its own
impulse response — the input's own level crossfades between rooms
("dynamic convolution" pointed at spaces). And the IR is a *stream*, not
a static object: every IR change (bank switch, size sweep, sample reload)
streams into the running convolver partition-by-partition, so nothing
ever clicks — transitions are the reverb's own decay, at a speed you set
with Morph Speed.

| # | name | module | range (default) | what it does |
|---|------|--------|-----------------|--------------|
| 1 | **Bypass** | — | Off/On (Off) | Click-free, latency-aligned bypass: wet is muted, dry passes at unity through the same 256-sample delay the host already compensates. No PDC jump. |
| 2 | **Wet** | mix | 0–100% (35%) | Level of the convolved signal, after per-zone gains, *before* Wet Sat. |
| 3 | **Dry** | mix | 0–100% (100%) | Unprocessed signal level, delayed 256 samples to stay sample-aligned with the wet. |
| 4 | **Wet Sat** | mix | 0.00×–4.00× (0.00×) | tanh drive on the summed wet. **0 (default) = fully clean/linear** — hot wet rides float headroom, trim Wet to taste. Raise it only when you *want* saturation color (1 ≈ transparent below −12 dBFS, more = deliberate sub squash). |
| 5 | **Symmetry** | selector | 0–100% (0%) | Negative half-cycles fire the zone ladder *mirrored* (zone 1 ↔ zone 4…). 0% = off; 100% = full cross-fire (the "xsign" sound); anywhere between is a continuous timbre blend. Even-order waveshaping whose harmonics come out spatialized — on subs, each cycle alternates rooms at audio rate. Works in both Selector modes. |
| 6 | **Size** | ir | 0.25×–4.00× (1.00×) | IR stretch by resampling — pitch-coupled, like a classic convolution "size": 2× is an octave down and twice as long. On tuned banks (Subdrop) this is a *tuning* knob. Sweeps are click-free: each change re-renders in the background and streams in at Morph Speed. |
| 7 | **Selector** | selector | Instant / Envelope (Envelope) | How the input level picks zones. **Instant**: per-sample rectified level — zone crossings happen at audio rate, adding a waveshaper-like color (the classic dynamic-convolution texture). **Envelope**: an attack/release follower drives the zones — the *room follows your dynamics*, no waveshaping. |
| 8 | **Attack** | selector | 0.1–50 ms (5 ms) | Envelope-mode rise time. Short = hits jump to loud zones immediately; longer = transients "read" quieter than they are. |
| 9 | **Release** | selector | 5–1000 ms (120 ms) | Envelope-mode fall time. Long releases make each hit's decay *sweep down through the rooms*; short ones snap back to the quiet zone between hits. |
| 10 | **Zones** | zones | 1–4 (4) | Active zone count. 1 = an ordinary (single-IR) convolver — useful as a reference/regression setting. |
| 11–14 | **Zone 1–4 Level** | zones | −70–0 dB (−48/−30/−18/−6) | The dB centers of the zones' triangular crossfade windows, quiet → loud. Below Zone 1's center everything is Zone 1; above the top center everything is the top zone. Slider order is safe: the engine keeps centers ascending internally (≥ 0.5 dB apart). Packing centers into your material's crest range makes the ladder *move* more. |
| 15–18 | **Zone 1–4 Gain** | zones | 0–200% (100%) | Per-zone wet trim. Sample-based IRs (Folder bank) are usually denser than the designed banks and want lower gains; also your per-zone balance tool (e.g. duck the loud-zone room, feature the ghost-note room). |
| 19 | **IR Bank** | ir | Rooms / Subdrop / Resoroom / Folder (Subdrop) | Which IR set fills the zones. **Rooms**: noise rooms, quiet = long/dark → loud = short/bright. **Subdrop**: 808-shaped tuned booms with downward pitch glide, loud = deepest/hardest — velocity picks the boom; Size retunes it. **Resoroom**: noise chambers + damped low modes (dub weight). **Folder**: your own samples from `~/Music/open-conv/zone1.wav … zone4.wav` — any wav becomes a zone's space (normalized automatically). |
| 20 | **Reload IRs** | ir | Off/On (Off) | Edge-triggered: each Off→On flip re-reads the Folder bank from disk (and re-renders whatever bank is active). Drop new samples in the folder, flip this, hear them stream in. Leaving it On is harmless; only the rising edge acts. |
| 21 | **Morph Speed** | ir | 1.00×–16.00× (1.00×) | IR transition rate (Gated tails only): convolver partitions replaced per 256-sample frame. **1×** = strict streaming (a transition takes the whole tail length — the luxurious glide). **16×** ≈ 200 ms for a 3 s IR — use 12–16× when automating Size/Bank per hit so changes track the hits. Middle values (4–8×) are their own sound: each hit catches the previous hit's room still morphing. Always click-free; this only sets how wide the in-between window is. |
| 22 | **Transition Fade** | ir | 1–16 frames (4) | How sharply each replaced partition lands (Gated tails only): each write fades over N frames (~5.3 ms each). **1** = hard per-partition steps — the "skitter" edge, on purpose. **4** = default rounding (~21 ms). **16** ≈ 85 ms maximal smear. Interacts with Morph Speed: fast morphs + fade 1 = maximum grit. |
| 23 | **Tails** | ir | Gated / Ungated (Gated) | What happens to the old room when a new IR arrives. **Gated**: streaming replacement — the room *morphs* (one voice per zone; Morph/Fade shape it). **Ungated**: the old room is frozen and **rings out its entire tail naturally** while the new room starts fresh — exact parallel convolution, click-free by construction; Morph/Fade don't apply. Up to 8 old rooms ring per zone (history depth, not bank size — a bank's 4 samples are the 4 level zones); switch faster than they decay and the oldest fades out over ~50 ms to make room (never hard-cut). With a jittery Bank LFO this is reverb-cloud territory: every flip leaves the previous space hanging in the air. |

## Practical notes

- **Level is a parameter.** Zone selection reads absolute input level —
  gain-stage *into* OpenConv deliberately. A clip gain of ±6 dB moves the
  material to a different part of the ladder; that's an intended
  performance control, not a calibration chore.
- **Automating Size + IR Bank:** fully supported and reconciled — the
  latest requested state always wins, live and offline render behave the
  same. Set Morph Speed high enough that transitions fit between your
  automation points, or low to let them smear intentionally.
- **Folder-bank workflow:** files are read on Reload's rising edge (and
  on switching into Folder). Mono or stereo wavs, any sample rate, any
  length up to 8 s at current Size. Missing `zoneN.wav` = that zone
  silent.
- **Latency:** constant 256 samples, reported to the host — PDC handles
  it; the Dry path inside the plugin is already aligned.
