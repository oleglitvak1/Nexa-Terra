# Milestone 6 plan: Colony gaps

Three rules turn the 45-Sol deadline and the trade cargo into sim state: the
smelter makes the ore crates, the scrubber's saturation meter counts down to
Sol 45, and Base Hypoxia runs its stages. The content and the `caretaker`
preset are retuned so a well-run colony has 10 crates by Sol 10.

The rules are in `docs/architecture.md` ("Colony gaps"). This file is the work
order; delete it, or mark it done, when the milestone closes.

## Decisions (interview, 2026-11-09)

| Question | Decision |
| --- | --- |
| Empty manifest | *The Mullein* starts empty and fills from the smelter. She may launch with 0 crates; the launch screen warns below a full hold. |
| Retune approach | Add a `PowerAbovePercent` trigger (model, evaluator, editor) so `caretaker` can cycle the smelter: off when power is low, back on when it recovers. |
| "Repairs slower" | During hypoxia a repair takes `hypoxia_repair_hours` (content): the machine stays failed until a countdown ends. Outside hypoxia repairs stay instant. |
| Stage timing | Meter plus candle supply. Degraded starts when the meter fills; Unconscious when the O2 candles (78 h) run out; Lost 26 h later. Nominal running gives Sols 45 / 48 / 49; dark hours pull it all earlier. |
| Old saves | Modules with matching IDs (`smelter_01`, `scrubber_01`) get the shipped `ore` and `scrubber` blocks on load, as the crew migration did for habs. The meter starts at the hours already elapsed. |
| While Unconscious | Only the Caretaker works: Standing Orders keep switching modules; manual repairs are refused; production stops. |
| Ore while away | Ore piles up in a ground stockpile and fills the hold when she's back on the pad (used from milestone 8). |

## What the code shows today

- The cargo is fixed: `content/mullein.ron` lists 10 crates, `Ship::new` copies
  them, and `ShipDef::check` refuses an empty manifest.
- `scripts/playthrough/a_nominal.nts` expects 10 crates after `orders caretaker`
  / `run-to 10`. It only passes because the cargo is fixed; `caretaker` loses the
  smelter on Sol 5.
- The battery budget is tight. Sols 0 to 10 are 260 hours: about 8,100 kWh
  (30 kW x 260 + 300 kWh stored). The other loads use about 4,940 kWh, leaving
  roughly 210 smelter-hours. Ten crates by Sol 10 means about 50 kg/h while
  Nominal, and the smelter must stay warm because it heats the habs.
- `Tick::sol()` counts from Sol 0 at landfall, so "full on Sol 45" is tick
  1170 (45 x 26) with one unit per running hour.
- Flight tests, `gdd_invariants::ready()`, `ship.rs` tests and `nexa-tools
  --launch` / `--sweep` (default 0 Sols on the ground) assume 10 crates on the pad.
- `hypoxia_runs_degraded_then_unconscious_then_lost` is `#[ignore]`d; this
  milestone makes it pass. `a_14_sol_audit_...` stays ignored until milestones 7-8.

## Design

**Ore (`colony`).**
- An `ore` block on the smelter's `ModuleDef`, e.g.
  `ore: Some((kg_per_hour: 50, crate_kg: 1000, cargo: "raw silicon"))`.
- The colony keeps a stockpile in kg. Each hour the smelter runs Nominal it adds
  `kg_per_hour` (half during Degraded, none from Unconscious on).
- With the ship on the pad, each full `crate_kg` becomes an `OreCrate` (100%
  integrity) on her manifest, up to `hold_crates: 10` in `mullein.ron`.
  Otherwise the ore stays in the stockpile.
- Event: `OreCrateLoaded { id, crates }`.

**Scrubber meter (new `life_support` module, owned by `Colony`).**
- A `scrubber` block on `scrubber_01`, e.g.
  `(capacity: 1170, per_running_hour: 1, per_dark_hour: 2)`. A dark hour is
  one where the scrubber is switched off or unpowered.
- Shown as a percent and Sols left. `ScrubberSaturated` fires once when full.
  Carbon Filters (milestone 8) reset it.

**Hypoxia stages.** The O2 candles are the fuel gauge for the stages.
- *Degraded* (meter full): the scrubber draws an extra `candle_draw_kw`; the
  candles last `candle_hours: 78`; ore at half rate; repairs take
  `hypoxia_repair_hours`; Ground crew gain stress at a new rate in `crew.ron`;
  `VitalsCritical` fires for each Ground member with `adult: false` (Maya).
- *Unconscious* (candles out): production stops; `Repair` is refused with
  `CommandError::CrewUnconscious`; Standing Orders still switch modules.
- *Lost* (`unconscious_hours_to_loss: 26` later): the colony stops stepping,
  `ColonyLost { sol }` fires and the sim builds a typed `Autopsy` (meter-full
  Sol, candles-out Sol, last vitals per Ground crew member, last orders fired).
  No text in the sim; milestone 8 shows it on the cockpit terminal.
- Events: `HypoxiaStageChanged { stage }`. The stages run whoever is on the
  Ground; crew effects apply only to Ground crew. The voyage catch-up already
  ticks the colony, so hypoxia during a voyage needs only a test.

## Steps

Each step ends green on test, clippy, fmt and both WASM builds, then commits.

1. **Docs.** Record the decisions in `docs/architecture.md` ("Colony gaps",
   module table gains `life_support`, orders section gains `PowerAbovePercent`).
   *(Done with this plan.)*
2. **Content and save shape.** `ore` and `scrubber` blocks on `ModuleDef`;
   `hold_crates` on `ShipDef`; allow and ship an empty manifest. Content checks:
   at most one ore producer and one scrubber, nonzero rates, crate mass > 0. New
   state fields get serde defaults; pre-M6 saves get the shipped blocks by
   module ID. Tests for parsing, rejection and old-save loading.
3. **Fixture fallout.** A `Ship::load_crates` helper so flight tests,
   `gdd_invariants::ready()` and `ship.rs` tests start with 10 crates
   explicitly. `nexa-tools --launch` / `--sweep` default to 10 Sols under
   `caretaker`. Test that dumping is unavailable with fewer than 3 crates.
4. **Smelter ore** (tests first): production only while running and Nominal;
   the crate boundary; the hold cap; stockpiling while away; loading resumes
   back on the pad; determinism.
5. **Scrubber meter.** Full exactly at tick 1170 running every hour; one hour
   earlier per dark hour; `ScrubberSaturated` once.
6. **`PowerAbovePercent` trigger.** Enum variant, evaluator with the power
   re-arm margin, rendering, save check, editor entry. Tests mirror
   `PowerBelowPercent`.
7. **Hypoxia stages.** Degraded, Unconscious, Lost and the pending repair.
   Un-ignore `hypoxia_runs_degraded_then_unconscious_then_lost` (Sols 45-47, 48,
   49). Test the stages during a voyage catch-up, and that dark hours pull them
   earlier.
8. **Autopsy record.** Built at loss, kept in the save; `nexa-tools` prints it.
9. **Retune.** Add ore, crates-by-Sol and scrubber columns to the `nexa-tools`
   table. Tune `kg_per_hour` and the `caretaker` preset (cycling the smelter on
   `PowerAbovePercent`) so that: `caretaker` has 10 crates by Sol 10 and keeps
   both habs Nominal with no wear through Sol 45 (existing caretaker tests stay
   green); an unattended colony gets clearly fewer; the M5 sweep numbers hold.
   Write the numbers into the architecture doc's tuning note.
10. **Scripts.** `expect` keys `ore KG`, `scrubber PCT`,
    `hypoxia none|degraded|unconscious|lost`; screenshot points `hypoxia` and
    `lost`; new `scripts/playthrough/e_hypoxia.nts` that never launches and
    runs past Sol 49; a `runs_to_the_end` case in `playthroughs.rs`.
    `a_nominal`'s `expect crates 10` and `d_prospector`'s `expect crates 7`
    pass on real output.
11. **Game screens.** Colony status `ORE 640/1000 kg  CRATES 6/10  SCRUBBER 23%
    (34 Sols)`; hypoxia banner and base-log lines; a COLONY LOST panel with the
    autopsy summary; launch confirmation shows crates and warns below a full
    hold; `PowerAbovePercent` in the orders editor. Check with
    `--script ... --screenshot-at hypoxia,lost`.
12. **Close out.** Mark M6 done in the roadmap; update `scripts/README.md`.

## Done when

- `caretaker` loads 10 crates by Sol 10 with both habs Nominal.
- The meter is full on Sol 45 under nominal running.
- Hypoxia goes Degraded, Unconscious, Lost on Sols 45, 48, 49 and leaves an autopsy.
- The four existing playthroughs reach their `dock` line; `e_hypoxia` plays to its end.
- Test, clippy, fmt and both WASM builds are green.
