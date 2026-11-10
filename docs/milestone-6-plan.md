# Milestone 6 plan: Colony gaps

Three rules turn the 45-Sol deadline and the trade cargo into sim state: the
smelter makes the ore crates, the scrubber's saturation meter counts down to
Sol 45, and Base Hypoxia runs its stages. Alongside them, the Caretaker gets
easier to use, so that meeting the trade target never needs hand-built logic:
a battery guard on the smelter, named order presets in plain words, and a
forecast before launch.

The rules are in `docs/architecture.md` ("Colony gaps" and "Making the
Caretaker approachable"). This file is the work order; delete it, or mark it
done, when the milestone closes.

## Decisions

Interview, 2026-11-09, then revised the same day after a difficulty review:
building rule cards felt like coding, and the first plan made it worse by
asking the preset to cycle the smelter with a new `PowerAbovePercent` trigger.
That trigger is dropped.

| Question | Decision |
| --- | --- |
| Empty manifest | *The Mullein* starts empty and fills from the smelter. She may launch with 0 crates; the launch screen warns below a full hold. |
| Reaching 10 crates by Sol 10 | A **battery guard** on the smelter: a module setting, not a card. It pauses the smelter below one battery percentage and resumes it above another. The player turns two dials. No new trigger. |
| Orders for the average player | Named **postures** in plain words (*Protect the habs*, *Balanced*, *Max ore*) loaded from the orders screen are the normal path. Building cards stays, as the advanced option. |
| Feedback before leaving | A **forecast** on the launch screen and the orders screen: the sim runs a copy of the world ahead and summarises what happens if nobody touches the base. |
| "Repairs slower" | During hypoxia a repair takes `hypoxia_repair_hours` (content): the machine stays failed until a countdown ends. Outside hypoxia repairs stay instant. |
| Stage timing | Meter plus candle supply. Degraded starts when the meter fills; Unconscious when the O2 candles (78 h) run out; Lost 26 h later. Nominal running gives Sols 45 / 48 / 49; dark hours pull it all earlier. |
| Old saves | Modules with matching IDs (`smelter_01`, `scrubber_01`) get the shipped `ore`, `guard` and `scrubber` blocks on load, as the crew migration did for habs. The meter starts at the hours already elapsed. |
| While Unconscious | Only automation works: Standing Orders and the guard keep switching; manual repairs are refused; production stops. |
| Ore while away | Ore piles up in a ground stockpile and fills the hold when she's back on the pad (used from milestone 8). |
| What a posture sets | Its cards **and** guard settings: loading one sets both, so one pick gives the whole setup. A posture may leave the guard alone by not naming it. |
| Sol 45 boundary | The meter fills at the end of the 1,170th running hour; Degraded's effects start the hour after. A run of exactly 45 Sols sees `ScrubberSaturated` and nothing else of hypoxia. |
| Keeping steps green | Steps 2 to 9 change no balance: the shipped manifest stays pre-loaded with 10 crates and the smelter ships with no guard. Step 10 is the one balance commit (see *Test suite impact*). |
| Base log | Readable at a glance (seen in the step-2 playthrough: from Sol 8 the `caretaker` run is almost all `ORDER 2 FIRED` / `ORDER 3 FIRED`, every two hours). A fired card says what it did in plain words, routine repeats fold into one counted line, and anything that changes the colony's state always gets its own line. Presentation only; the sim's events don't change. |
| Plain-language failure causes | What a card did is in plain words from M6 (base log). Chains of cause are deferred to milestone 8's Black Box ("smelter froze on Sol 5: card 1 shut it off at 25% power and nothing restarted it"). M6 keeps the events the Black Box will need. |
| Playtest early | Show the orders screen (postures, guard, forecast) to one or two non-programmers at the end of M6, not only at the M8 playtest. |

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
- A switched-off smelter freezes (its band starts at -5.0 C against -40.0 C
  outside), so a guard that pauses it for long goes Offline. The tuning has to
  keep pauses short, or the guard needs a keep-warm rule (step 10).
- The orders screen opens on the postures and loads one (step 4); building and
  deleting cards sit under Advanced.
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

**Battery guard (`colony`).**
- A `guard` block on a module's def gives its shipped setting, e.g. on the
  smelter `guard: Some((pause_below_percent: 25, resume_above_percent: 60))`.
  The current setting lives in the module's state, so it is saved.
- Each hour, before power is routed, a guarded module that is switched on
  pauses when its power network's stored charge is below `pause_below` and
  resumes when it is above `resume_above`. Between the two it keeps its state,
  so it doesn't flutter.
- Paused is not switched off: `Deactivate` still turns it off, and the guard
  never turns on a module the player or a card switched off.
- `Command::SetGuard { module, guard: Option<Guard> }` (in `nexa-types`)
  changes or clears it; refused unless `pause_below < resume_above <= 100`.
- Events: `GuardPaused { module }` and `GuardResumed { module }`.

**Postures (`orders`).**
- Each preset in `content/standing_orders.ron` gains a display `name` and a
  plain-language `summary` (one or two lines, no command syntax).
- A preset may also name guard settings, by module:
  `guards: { "smelter_01": Some((pause_below_percent: 20, resume_above_percent: 50)) }`
  (`None` clears a guard; a module left out keeps its current guard).
- Shipped postures: `protect-habs` (today's three cards, no guard: the habs stay
  warm, the smelter is given up), `caretaker` (keeps its ID, shown as *Balanced*:
  the habs stay warm and the guarded smelter keeps working) and `max-ore` (the
  smelter first, the habs at risk).
- Loading one replaces the cards and applies its guards, through the same
  checks as `World::add_order` and `Command::SetGuard` (the orders screen asks
  first if any cards are set). A preset naming an unknown or unguardable
  module fails the content check.

**Forecast (new `forecast` module).**
- `World::forecast(&self, until_sol) -> Forecast` copies the world and ticks
  the copy with nobody touching it, to `until_sol` (45 by default) or until the
  colony is lost. Colony ticks use no RNG, so the result is exact for the base;
  Slip delays are unknown and aren't part of it.
- `Forecast` holds: each machine's first failure (module, state, Sol); each
  Ground crew member's peak stress level; crates loaded and ore stockpiled by
  the horizon; the Sol the scrubber fills; the hypoxia stage reached.
- Cost: at most 1,170 ticks of a 9-module colony, run when the screen opens or
  the orders change, not every frame.

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
  `CommandError::CrewUnconscious`; Standing Orders and the guard still switch.
- *Lost* (`unconscious_hours_to_loss: 26` later): the colony stops stepping,
  `ColonyLost { sol }` fires and the sim builds a typed `Autopsy` (meter-full
  Sol, candles-out Sol, last vitals per Ground crew member, last orders fired).
  No text in the sim; milestone 8 shows it on the cockpit terminal.
- Events: `HypoxiaStageChanged { stage }`. The stages run whoever is on the
  Ground; crew effects apply only to Ground crew. The voyage catch-up already
  ticks the colony, so hypoxia during a voyage needs only a test.

## Test suite impact

About 300 tests today; roughly 25 change and 40 to 50 are added. Grid,
machines, parts, `check`, RNG, hazard content, Split rules, crew stress,
card mechanics, terminal/UI widgets and the determinism tests are untouched.

| Group | Tests | What happens | When |
| --- | --- | --- | --- |
| Assume 10 crates aboard | `flight/tests.rs` (20), `voyage_view/tests.rs` (9), the flight tests in `gdd_invariants`, `ship.rs` (5), the dump assertion in `nexa-tools/src/slip.rs` | Load cargo explicitly through `Ship::load_crates` and the `ready()` helper. `ship.rs`'s `EmptyManifest` test flips: empty is valid, the new check is `hold_crates` | Step 3, before the manifest empties |
| Card identity | `keying_in_the_caretaker_preset_saves_both_habs`, `orders_view/tests.rs` preset loop, the `remove_order` test in `lib.rs`, the shipped-preset tests in `orders.rs`, the `shutdown smelter_01` screen text | Re-point from `caretaker` to `protect-habs`, which keeps today's three cards | Step 2, when presets get names |
| Shipped balance | `unattended_ash_plain_loses_both_habs_by_sol_10`, `unattended_ash_plain_breaks_the_crew`, `caretaker_orders_keep_both_habs_through_sol_10`, `caretaker_orders_keep_the_crew_calm`, gdd `caretaker_keeps_the_habs_nominal_with_no_wear_to_sol_45`, the margin-flutter test in `orders.rs`, `a_card_built_mid_run_starts_from_the_live_reading`, multi-Sol crew-screen and catch-up tests | Re-baselined on purpose; the diff is the record of the balance change. The caretaker ones move to the *Balanced* targets; the unattended and flutter ones keep their point (nobody tending the habs fails; a card without a margin chatters) on the new numbers | Step 10 only |
| Scripts | `a_nominal` `expect crates 10`, `d_prospector` `expect crates 7` | Now check real smelter output | Step 10 |
| Sol 45 boundary | The two 45-Sol caretaker tests | Unchanged, given the boundary decision above | Step 8 adds a test pinning it |
| Exhaustive matches | Base log, driver, tools | New `SimEvent` / `Command` variants are compile errors, not failures | As each lands |
| Saves | Round-trip tests | Survive (serde defaults); add a pre-M6 save fixture | Step 2 |

Runtime cost is small: a forecast or a 50-Sol run is about 1,300 ticks of a
9-module colony.

## Steps

Each step ends green on test, clippy, fmt and both WASM builds, then commits.
Each sim step also ships its own screen lines and script keys, so every
hands-on session shows what has been built so far.

1. **Docs.** Record the decisions in `docs/architecture.md` and the Standing
   Orders paragraph of `docs/game-design.md`. *(Done with this plan.)*
2. **Content and save shape.** *(Done.)* `ore`, `guard` and `scrubber` blocks on
   `ModuleDef`; `hold_crates` on `ShipDef`; allow an empty manifest (the
   shipped one stays at 10 crates until step 10); `name`, `summary` and
   `guards` on presets; add `protect-habs` as a copy of today's `caretaker` and
   re-point the card-identity tests at it. No balance changes: the smelter
   ships with no guard and `caretaker`'s cards are unchanged. Content checks: at most one ore producer
   and one scrubber, nonzero rates, crate mass > 0, guard thresholds ordered,
   every preset has a name and summary, preset guards name guardable modules. New state fields get serde defaults;
   pre-M6 saves get the shipped blocks by module ID. Tests for parsing,
   rejection and old-save loading.
3. **Fixture fallout.** *(Done.)* A `Ship::load_crates` helper so flight tests,
   `gdd_invariants::ready()` and `ship.rs` tests start with 10 crates
   explicitly. `nexa-tools --launch` / `--sweep` default to 10 Sols under
   `caretaker`. Test that dumping is unavailable with fewer than 3 crates.
   Behaviour is unchanged: the fixtures load the same 10 crates the shipped
   manifest still holds.
4. **Postures in the game.** *(Done.)* Moved up from the screens step after the first
   hands-on session: the orders screen was still the IF/THEN builder. Modules
   are worded by an optional `name` on `ModuleDef` in the colony content
   (`the smelter`, `hab 1`), falling back to the ID; pre-M6 saves get the
   shipped names with the other blocks. Guards are checked, not applied, until step 6.
   - `StandingOrder::describe()` next to `render()`: a card in plain words,
     e.g. *"When power drops below 25%: shut down the smelter, start heater 2,
     send heat to hab 1."* The base log reuses it in step 11.
   - `O` opens on a posture picker: each posture's name and summary; `Enter`
     loads it. Loading replaces the cards, after a confirm if any are set.
     `World::load_posture` does the replace in the sim; `load_preset` and the
     script `orders` verb use it.
   - The card list shows each card's plain-words line first and its
     `IF ... THEN ...` line second, dimmer. Building and deleting cards move
     behind an "Advanced" key.
   - Tests: picking *Protect the habs* loads its three cards and the habs
     survive Sol 10; loading over existing cards asks first and replaces them;
     every shipped card has a plain-words line; the advanced builder still keys
     in a posture by hand.
5. **Smelter ore** (tests first, on a test colony): production only while
   running and Nominal; the crate boundary; the hold cap (on shipped content the
   hold is already full, so ore only stockpiles until step 10); stockpiling
   while away; loading resumes back on the pad; determinism.
   - Screens: the colony status shows `ORE 640/1000 kg  CRATES 10/10`; the
     launch confirmation shows crates aboard and warns below a full hold.
   - Scripts: `expect ore KG`.
6. **Battery guard** (tests first): pauses below and resumes above, holds
   between; never overrides a switch-off; `SetGuard` validation; runs before
   Standing Orders so a card can still override it in the same hour; saved and
   loaded; loading a posture applies its guards.
   - Screens: a guard marker on a paused module; the guard's two dials in 5%
     steps on the module detail. Hand-keyed posture tests set the dials as well
     as the cards.
   - Scripts: a `guard MODULE PAUSE RESUME|off` verb.
7. **Scrubber meter.** Full exactly at tick 1170 running every hour; one hour
   earlier per dark hour; `ScrubberSaturated` once.
   - Screens: the colony status adds `SCRUBBER 23% (34 Sols)`.
   - Scripts: `expect scrubber PCT`.
8. **Hypoxia stages.** Degraded, Unconscious, Lost and the pending repair.
   Un-ignore `hypoxia_runs_degraded_then_unconscious_then_lost` (Sols 45-47, 48,
   49). Pin the boundary: a run of exactly 45 Sols sees `ScrubberSaturated` and
   no hypoxia effects. Test the stages during a voyage catch-up, and that dark
   hours pull them earlier.
   - Screens: a hypoxia banner and base-log lines.
   - Scripts: `expect hypoxia none|degraded|unconscious|lost`; screenshot
     point `hypoxia`; new `scripts/playthrough/e_hypoxia.nts` that never
     launches and runs past Sol 49; a `runs_to_the_end` case in
     `playthroughs.rs`.
9. **Autopsy and forecast.** The `Autopsy` is built at loss and kept in the
   save; `nexa-tools` prints it. `World::forecast` with tests: it leaves the
   real world untouched, matches actually running the same Sols tick for tick,
   and reports the shipped `protect-habs` posture losing the smelter on Sol 5.
   - Screens: a COLONY LOST panel with the autopsy summary; a one-line
     forecast on the orders screen that updates when the posture or cards
     change; the forecast summary on the launch confirmation.
   - Scripts: screenshot points `forecast` and `lost`.
10. **Retune and postures: the one balance commit.** Empty the shipped
    manifest, give the smelter its shipped guard, give `caretaker` its
    *Balanced* cards and guards, add `max-ore`, and re-baseline the
    shipped-balance tests and the two script expectations (`a_nominal`'s
    `expect crates 10`, `d_prospector`'s `expect crates 7`) in this same
    commit. Add ore, crates-by-Sol, guard pauses and scrubber columns to the
    `nexa-tools` table. Tune `kg_per_hour`, the smelter's shipped guard and the
    three postures so that, each checked by a test:
    - *Balanced* (`caretaker`) loads 10 crates by Sol 10 and keeps both habs
      Nominal with no wear through Sol 45, and the crew calm (the existing
      caretaker tests stay green);
    - *Protect the habs* keeps the habs Nominal and loads fewer crates;
    - *Max ore* loads 10 crates sooner and lets a hab get cold;
    - no orders at all, with only the shipped guard, still loads a few crates;
    - the M5 sweep numbers hold.

    If a paused smelter freezes before the battery recovers, add a keep-warm
    rule to the guard (resume early when the module nears the bottom of its
    band) rather than asking players for a card. Write the numbers into the
    architecture doc's tuning note.
11. **A readable base log** (`colony_view.rs`):
    - A fired card says what it did, using `describe()`, e.g.
      `CARETAKER: SHUT DOWN SMELTER_01, STARTED HEATER_02, HEAT TO HAB_01`,
      not `ORDER 1 FIRED`.
    - Routine lines (card firings, guard pauses and resumes, Sol starts) fold:
      a line whose text matches one of the last few routine lines updates
      that line's count and time span instead of adding one, e.g.
      `S008 H18-S009 H24 CARETAKER: HEAT TO HAB_02 x12`. Sol starts don't
      break a fold.
    - Anything that changes the colony's state (a machine state change, a
      stress level, a crate loaded, the scrubber, hypoxia, a refused order)
      always gets its own line and ends the fold, so it is never buried.
    - Tests: the shipped `caretaker` run from Sol 8 to Sol 10 fits on one
      screen of log; a machine failure in the middle of a fold still shows
      as its own line; the voyage catch-up summary still counts every firing.
12. **Close out.** Mark M6 done in the roadmap; update `scripts/README.md`.
    Check the new screens with `--script ... --screenshot-at
    forecast,hypoxia,lost`. Run the early playtest with one or two
    non-programmers and write down what they got stuck on.

## Done when

- A new player can pick *Balanced*, launch on Sol 10 with 10 crates and see a
  forecast that the base holds to Sol 45, without building a card.
- The base log of a `caretaker` run reads in plain words, with routine card
  firings folded instead of filling the screen.
- The meter is full on Sol 45 under nominal running.
- Hypoxia goes Degraded, Unconscious, Lost on Sols 45, 48, 49 and leaves an autopsy.
- The four existing playthroughs reach their `dock` line; `e_hypoxia` plays to its end.
- Test, clippy, fmt and both WASM builds are green.
