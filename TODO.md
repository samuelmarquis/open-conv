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

## Parked

- `PATHS-NOT-TAKEN.md` #1–#12 (#10, the parametric tail, has the
  freshest re-entry notes: blend-dial shape, fit-floor gating first).
- M4 CPU throughput measurement (`docs/DSP.md` §13 lists it as
  unmeasured).
