# TODO

The build queue, roughly ordered. Deferred *designs* (with re-entry
notes) live in `PATHS-NOT-TAKEN.md`; this file is what's next.

## v0.2 — per-zone shape (slate accepted 2026-07-23)

- **Per-zone Attack/Release as weight ballistics.** Keep the single
  level source; each zone's *weight signal* gets its own one-pole
  attack/release — "how fast this room opens / how long it lingers."
  Fork decision on record: ballistics, not per-zone followers.
- **Per-zone Size.** `size[4]` symmetric to `damp[4]` through the
  worker Sync / render / stepwise-stream path. Automatable, same
  debounce+morph semantics as global Size today.
- **Globals become macros, not rivals.** Global Attack/Release/Size
  stay, but *compose* with the per-zone values instead of fighting for
  one slot — so one automation lane still sweeps the whole shape.
  Combine mode per control decided at build time: Size is a ratio →
  multiplicative; A/R are times → likely multiplicative too
  (constant-feel scaling across zones), but A/B additive if it feels
  wrong.
- 12 new params (ids 37–48). Zone strip = Level / Gain / Damp /
  Attack / Release / Size.

## MIDI input???

Per-zone Size in the crystal modes = each harmonic order's room tuned
independently → octaves/fifths → **Bismuth becomes a chord machine.**
MIDI could set the order-room tunings (or transpose the root). Revisit
after v0.2 ships per-zone Size and the tunings prove out by hand
first. ⚠ ABI note: the AU is registered `aufx` (no MIDI); shipping
MIDI input means `aumf`, which is a public-identity change — decide
before, not after, a release that carries it.

## Panel

Native panel awaits the family design system
(`open-plugin-template/design-lab`). Photoshop layout in progress
(user). `VizFrame` / `--viz-dump` JSONL is the agreed data contract.

## Licensing / commercialization (conversation of 2026-07-23)

Current state: GPL-3.0-or-later **by choice, not obligation** — the
entire dependency stack is permissive (WRAC MIT, clap-wrapper MIT,
CLAP MIT, VST3 SDK 2025 checkout MIT throughout, AudioUnitSDK
Apache-2.0, crates MIT/Apache). Sole copyright holder ⇒ future
versions can take any license. v0.1.0 as released stays GPL forever
(irrevocable), which is fine.

- [ ] **CLA-or-no-outside-code decision BEFORE merging any outside
  PR.** One merged GPL patch from a stranger and every future
  relicense needs their consent.
- [ ] **FTO review of Abel US 12,361,921** (evolving input-derived
  IRs) vs the Ungated epoch-voice mechanism, before money changes
  hands. (Kemp US 7,039,194 / 7,095,860 expired; Franck US 10,187,741
  unused by design; Yamaha US 8,116,470 unimplemented, PNT #9.)
- [ ] **US patent window on our own novel bits** (epoch-voice ring
  sharing, Bismuth tempered-homogeneity law, weight ballistics once
  built): the 12-month inventor grace period runs from first public
  disclosure (2026-07-21 push) → **decide by ~July 2027**. Most
  non-US jurisdictions are absolute-novelty and already foreclosed.
- [ ] **Rename before there are users.** "open-conv" is a temp name.
  Plugin identity strings (bundle id, plugin_id, FourCCs, VST3 CID)
  are host ABI (template §8): a rename after people have sessions
  orphans those sessions. New name ⇒ new identity ⇒ do it while
  n(users) ≈ 2. Trademark the final name — trademark is separate from
  the code license and is the one moat that survives publication.
- [ ] **If the repo goes private, unpublish the v0.1.0 release in the
  same motion.** GPLv3 ties source availability to binary
  availability for network conveyance — stopping both together is
  clean; a downloadable binary with vanished source is not. (Past
  downloaders keep their GPL rights to what they already got;
  irrevocable, and fine.)

## Parked

- `PATHS-NOT-TAKEN.md` #1–#12 (#10, the parametric tail, has the
  freshest re-entry notes: blend-dial shape, fit-floor gating first).
- M4 CPU throughput measurement (`docs/DSP.md` §13 lists it as
  unmeasured).
