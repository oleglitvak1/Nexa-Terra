# Nexa Terra — Technical Architecture

This repo copy is the source of truth. An older "Nexa Terra — Technical Architecture" doc in Claude (Nov 2026) is no longer kept in sync; when they differ, this file wins.

## Summary and key decisions

Nexa Terra is built in Rust on macroquad, with no game engine. It ships as a desktop app first, adds a browser (WASM) build later, and is designed from day one so a shared online universe can be added without rewriting the game.

| Decision | Choice | Why |
| --- | --- | --- |
| Language | Rust (stable) | Compiler catches AI-generated mistakes; enums fit the game's state machines; safe long-term refactoring; one language for client, web build and server |
| Graphics / input / audio | macroquad (library, not engine) | Thin, code-first, native and WASM from one codebase, shader support for the CRT look |
| First platform | Desktop (Windows, macOS, Linux) | Simplest to develop and debug |
| Later platform | Browser via WASM | Shareable playtest link; kept possible by the portability rules below |
| Presentation start | Terminal look inside a macroquad window | One front end that grows from text to CRT shader to sprites; the terminal stays in the final game as the ship console |
| Visual target | Tier 2: stylized 2D, amber CRT, vector panels | Mood of the spec at a fraction of 3D cost; mostly producible in code |
| Multiplayer | Shared universe, async, planned not built | Each player owns a colony; stations, economy and Echoes are shared; sync happens at docking |
| Content | Data files (RON), not code | Fast tuning and AI-friendly edits |

## Guiding principles

Seven rules keep every later layer (visuals, web, multiplayer) additive instead of a rewrite.

1. **The simulation knows nothing about presentation.** The core takes commands and exposes state. It never draws, plays sound, reads input or touches files.
2. **Content is data.** Machines, parts, crew, events, dialogue and tuning numbers live in data files. Code defines rules; data defines the world.
3. **Determinism.** Same seed + same commands = same result, on every platform. Seeded RNG owned by the sim, fixed-step Sol ticks, no wall-clock time inside the sim.
4. **Integer and fixed-point math for anything shared or economic.** Credits, cargo mass, fuel, prices, wear and temperatures use integers in explicit units (e.g. grams, centi-degrees, whole liters). Floats only in presentation.
5. **Everything is text.** Code, data, shaders, SVGs and asset-generator scripts all live in the repo, so an AI can read and edit any part of the game.
6. **Assets and sounds are referenced by ID with fallbacks.** The game asks for `machine.smelter`, never a file path, so art can be upgraded one piece at a time.
7. **The universe sits behind one interface.** Anything outside the player's colony (stations, markets, other ships, Echoes) is reached only through the `Universe` trait, offline first and online later.

## Workspace and crate layout

One Cargo workspace with four crates now and a fifth (`nexa-server`) later; dependencies only point downward toward `nexa-types`, and only `nexa-game` knows macroquad exists.

```text
        nexa-game            nexa-tools            nexa-server (later)
  desktop + WASM binary   headless dev tools    shared universe service
  macroquad: window,      balance runs,         axum + Postgres,
  input, audio, terminal  content checks        replays voyages via sim
            \                    |                    /
             v                   v                   v
   +-----------------------------------------------------------+
   | nexa-sim: pure, deterministic rules (grid, machines, crew, |
   | orders, flight, station, echoes, epochs). No graphics, no  |
   | file I/O, no clock. Includes OfflineUniverse. WASM-safe.   |
   +-----------------------------------------------------------+
                                 |
                                 v
   +-----------------------------------------------------------+
   | nexa-types: shared serde data: IDs, commands, events,      |
   | universe messages (LaunchContext, LaunchBriefing, ...)     |
   +-----------------------------------------------------------+
```

`nexa-game` and `nexa-server` also use `nexa-types` directly. Inside `nexa-game`, a `platform` module isolates storage, timing and anything that differs between desktop and web.

```text
nexa-terra/
  Cargo.toml          # workspace
  CLAUDE.md           # rules for AI-assisted work
  docs/               # game-design.md, architecture.md
  crates/
    nexa-types/  nexa-sim/  nexa-game/  nexa-tools/  nexa-server/ (later)
  content/            # RON game data
  scripts/            # playthrough scripts (.nts)
  assets/             # manifest.ron, fonts, sprites, shaders (empty until Layer 1)
  audio/              # cues.ron, generated WAVs (empty until Layer 1)
```

## Simulation core design

The sim is a pure state machine: `World` + `Command` → new `World` + a list of `SimEvent`s. The front end sends commands and renders state and events; it never mutates the world directly.

```rust
impl World {
    pub fn apply(&mut self, cmd: &Command) -> Result<Vec<SimEvent>, CommandError>;
    pub fn tick(&mut self) -> Vec<SimEvent>; // one fixed step
    pub fn add_order(&mut self, order: StandingOrder) -> Result<usize, OrderError>;
    pub fn confirm_split(&mut self, assignments: &BTreeMap<CrewId, Assignment>) -> Result<Vec<SimEvent>, SplitError>;
    pub fn launch(&mut self, universe: &mut dyn Universe) -> Result<Vec<SimEvent>, LaunchError>;
    pub fn advance_voyage(&mut self) -> Result<VoyageStep, FlightError>; // one Sol of The Slip
    pub fn resolve_hazard(&mut self, option_id: &str) -> Result<Vec<SimEvent>, FlightError>;
}
```

`Command` (in `nexa-types`) covers the colony actions a player or a Standing Order can take; the Split, orders and flight have their own `World` methods with typed errors. `SimEvent` lives in `nexa-sim`.

The universe is reached only at the sync points (launch now, docking with Scrap Barge 77 in milestone 7), so it is passed to the call that syncs, not to every tick. `tick` stays universe-free.

**Time.** The base unit is a fixed tick (26 ticks per Sol: one per hour of Nexa Terra's 26-hour day). The sim never reads the clock. The front end decides how fast ticks advance (paused, 1x, fast-forward), so the colony runs real-time with pause while flight and station phases advance in discrete event steps.

**Modules inside `nexa-sim`** (*planned* marks modules not built yet):

| Module | Owns | Key types |
| --- | --- | --- |
| `grid` | Tile map parsed from the landing content; conduit and heat-line networks | `Grid`, `Tile`, `Networks` |
| `colony` | Modules placed on the grid, built from a content file; the hourly power, heat and wear step (milestone 6 adds smelter ore output, the ore stockpile and battery guards) | `Colony`, `ColonyDef`, `Module`, `ModuleDef`, `ModuleKind` (Utility, Hab) |
| `machines` | Machine state, tolerance bands, wear, failures | `Machine`, `MachineState` (Nominal, Overheat, Freeze, Burnout, Offline), `ToleranceBand` |
| `parts` | Wear parts and inventory | `PartKind` (TungstenFilament, VitonSeal, SilicateBaffleMesh, CarbonFilter, ...), `Inventory` |
| `check` | The one skill-check function every hazard (and later haggle) uses | `check()`, `CheckDef`, `CheckTuning`, `Roll`, `Outcome` (Success, Partial, Fail) |
| `ship` | *The Mullein*: tank, ECM pods, cargo manifest, where she is | `Ship`, `ShipDef`, `OreCrate`, `ShipLocation` (Pad, InSlip, Holding) |
| `crew` | Six named specialists, skills, stress, morale, assignments, the Split rules | `CrewId`, `CrewMember`, `Skill`, `Trait`, `Assignment` (Ground, Flight), `SplitError` |
| `orders` | Standing Orders rule cards: typed model + evaluator, run each tick from `World::tick` | `Trigger`, `Action`, `StandingOrder`, `Orders`, `render()`, `evaluate()` |
| `hazards` | Slip hazard content: options, costs, tolls, check tuning, delay ranges and the cap | `HazardsDef`, `HazardDef`, `HazardOption`, `Toll`, `DelayKind` |
| `flight` | The Slip: launch, transit Sol by Sol, resolving hazards, delays and the random-delay cap, arrival | `Voyage`, `VoyageStep`, `Resolution`, `LaunchError`, `FlightError` |
| `universe` | The `Universe` trait and `OfflineUniverse` | `Universe`, `OfflineUniverse` |
| `script` | Playthrough scripts: parse `.nts` lines and run them headless | `Step`, `Expect`, `Line`, `RunError` |
| `station` (*planned*) | Docking, haggling, integrity vs. heat | `HaggleSession`, `VendorPatience`, `Leverage`, `Reputation` |
| `echoes` (*planned*) | Logged moral choices and their delayed payoffs | `EchoSeed`, `EchoTrigger` |
| `forecast` (*planned*, milestone 6) | Running a copy of the world ahead to show what happens if nobody touches the base | `Forecast` |
| `life_support` (*planned*, milestone 6) | Scrubber saturation meter, O2 candles, Base Hypoxia stages, the autopsy record | `Scrubber`, `HypoxiaStage` (None, Degraded, Unconscious, Lost), `Autopsy` |
| `epochs` (*planned*) | Atmosphere state and epoch milestones | `Atmosphere` (pressure, temp, toxicity, water), `Epoch` |
| `blackbox` (*planned*) | Log of everything that happened while the crew was away | `LogEntry`, `Reconciliation` |
| `rng` | The only source of randomness | `SimRng` (SplitMix64: seeded, serializable, no extra crate) |

**Colony grid model.** The colony steps once per tick (one planetary hour), in three phases. All numbers are integers and live in the content file.

- **Power (battery bank).** Touching conduit tiles form a power network. Each hour, a network's generators plus its stored battery charge supply its consumers in content-file order; a consumer that doesn't fit what's left goes dark for that hour. Surplus charges the batteries, a deficit drains them. `grid.power %` is total stored charge over total capacity.
- **Heat (cold pull + heat lines).** Running modules pour heat into their heat line's pool, shared evenly among the line's working (not failed) machines, or sent entirely to one module when the player diverts heat (`DivertHeat`). Then every module loses a fixed percentage of its gap to the outside temperature. A module off any heat line keeps its own heat.
- **Wear meter.** A machine gains wear (basis points per hour) only while out of its tolerance band. At 100% it fails: Overheat becomes Burnout, Freeze becomes Offline. A failed machine draws no power and makes no heat. A repair uses one part (Burnout: Tungsten Heating Filament; Offline: Viton Seal) and resets wear to 0.

**Colony gaps (milestone 6, decided, not built).** Three rules turn the 45-Sol deadline and the trade cargo into sim state. There is no oxygen network on the grid: the scrubber is a meter, not a flow. The work order is `docs/milestone-6-plan.md`.

- **Smelter ore.** Each hour the smelter runs Nominal it adds its `ore` rate (kg) to a ground stockpile. While *The Mullein* is on the pad, each full crate (1,000 kg of raw silicon, 100% integrity) is loaded onto her manifest, up to `hold_crates` (10) in `content/mullein.ron`; otherwise ore stays in the stockpile and loads when she is back on the pad. Her manifest starts empty and she may launch with any number of crates, including none. Target: a well-run colony fills the hold by the nominal Sol-10 launch while keeping the habs warm, without the player building any card. The rate lives in `content/landing_ash_plain.ron`, and the content, the smelter's battery guard and the `caretaker` preset (which today loses the smelter on Sol 5) are retuned until the target is reachable (see *Making the Caretaker approachable* below).
- **Scrubber saturation.** `scrubber_01` gets a whole-number saturation meter (`life_support`) that gains one unit each hour it runs and more each hour it is off or unpowered; its capacity (1,170 = 45 Sols x 26) makes it full on Sol 45 under nominal running. Installing Carbon Filters (milestone 8) resets it. The rates live in content.
- **Base Hypoxia.** Driven by the meter and the O2 candle supply, so dark hours pull every stage earlier. *Degraded* starts when the meter fills: O2 candles burn (extra scrubber draw) for 78 hours, the smelter produces at half rate, a repair takes several hours instead of being instant, Ground crew stress climbs, and Ground crew with `adult: false` (Maya) go vitals-critical. *Unconscious* starts when the candles run out (Sol 48): production stops and repairs are refused, but Standing Orders and battery guards still switch modules. *Lost* follows 26 hours later (Sol 49): the colony stops and the sim records a typed autopsy (milestone 8 shows it on the cockpit terminal). Stages apply whoever is on the Ground; crew effects only to Ground crew. Saves from before milestone 6 get the shipped `ore`, `guard` and `scrubber` blocks for matching module IDs.

**Making the Caretaker approachable (milestone 6, decided, not built).** Building rule cards reads like programming, and keeping the smelter alive with cards alone needs hour-by-hour control a small card set can't give. So the slice's goals must be reachable without building a card, and cards stay as the advanced option. Three pieces:

- **Battery guard.** A module setting, not a card: the smelter pauses itself when its power network's stored charge falls below one percentage and resumes above another, holding its state in between so it doesn't flutter. The shipped values are a `guard` block in the module's content; the current setting is module state (saved) and the player changes it with two dials (`Command::SetGuard`). It runs each hour before Standing Orders, so a card can still override it that hour. Paused is not switched off: the guard never turns on a module the player or a card switched off. Events: `GuardPaused`, `GuardResumed`. If a paused smelter would freeze before the battery recovers, the guard gets a keep-warm rule rather than asking the player for a card.
- **Postures.** Order presets carry a display `name` and a plain-language `summary`, and the orders screen offers them first: *Protect the habs* (`protect-habs`, today's three cards: the smelter is given up), *Balanced* (`caretaker`: habs warm, guarded smelter working) and *Max ore* (`max-ore`: the smelter first, habs at risk). Loading one replaces the cards. The card builder sits under them as "advanced".
- **Forecast.** `World::forecast(until_sol)` (module `forecast`) ticks a copy of the world with nobody touching it, to Sol 45 or until the colony is lost, and returns a `Forecast`: each machine's first failure, each Ground crew member's peak stress level, crates and stockpiled ore, the Sol the scrubber fills and the hypoxia stage reached. Colony ticks draw no randomness, so the base forecast is exact; Slip delays are unknown and left out. The launch confirmation and the orders screen show it, recomputed when the orders change rather than every frame.

Plain-language failure causes in the log ("smelter froze on Sol 5: card 1 shut it off at 25% power and nothing restarted it") come with milestone 8's Black Box.

**Standing Orders rule cards.** Orders are stored as typed Rust enums, never as strings. The player builds each one as a rule card by filling trigger, action and priority slots from lists, and the terminal renders the card in command syntax:

```text
IF grid.power < 25% THEN shutdown smelter_01 AND divert heat TO hab_01
IF hab_01.temp < -5.0C THEN activate heater_02
```

```rust
pub enum Trigger { PowerBelowPercent(u8), TempBelow { module: ModuleId, centi_c: i32 }, TempAbove { module: ModuleId, centi_c: i32 } }
pub enum Action  { Activate(ModuleId), Deactivate(ModuleId), DivertHeat { to: ModuleId }, ShareHeat { line_of: ModuleId } }
pub struct StandingOrder { pub triggers: Vec<Trigger>, pub actions: Vec<Action>, pub priority: u8 }
```

Only triggers and actions the colony can act on today are in the enums. `WaterBelowLiters` and `EngageEmergencyCandles` join them when the water and life-support systems exist. Each action maps to one colony `Command` and goes through `Colony::apply`, so a card can do nothing a player couldn't. Temperatures render in C with one decimal place, rounded half away from zero from integer centi-degrees.

How the Caretaker runs them (`orders::Orders`, stored on `World` so cards and their firing state are in saves):

- **When.** Once per tick, before the hour's power and heat step, against the colony as it stands. Actions take effect in that same hour.
- **Order.** By priority, 1 first; equal priorities run in list (slot) order. All of a card's triggers must hold for it to fire.
- **Conflicts.** Once a card switches a module, or diverts or shares a heat line, lower-priority cards can't touch that module or line for the rest of the hour. Their blocked actions are reported as `OrderActionSkipped { reason: Overridden }`, so the higher-priority card wins instead of whichever ran last.
- **Firing rule: on the edge, with a re-arm margin.** A card fires once when its triggers become true, then is disarmed. It re-arms only when one of its triggers has moved back past its threshold by a margin (2.0 C for temperatures, 5 percentage points for power; both in `content/standing_orders.ron`). A new card starts armed, so it fires on the next tick if its triggers already hold.
  - *Why not fire every tick while true:* a level-triggered card would re-issue its commands every hour, overriding the player's manual changes and fighting any other card that touches the same module. It would also flood the base log.
  - *Why a margin:* with no orders, the Ash Plain habs flicker between FREEZE and NOMINAL every hour or two from Sol 3, swinging about 2 C either side of the 5.0 C band edge. A `hab_01.temp < 5.0C` card with no margin fires 18 times by Sol 6; with the 2.0 C margin it fires once (`the_margin_stops_a_card_firing_on_every_flutter`).
- **Memory.** The terminal holds a fixed number of cards (`memory`, 8 for now). Adding past it fails with `OrderError::MemoryFull`, and so does loading a save that holds more. Cards naming unknown modules, or steering heat on a module off any heat line, are rejected when added and when a save loads.
- **Events.** A fired card emits `SimEvent::OrderFired { slot, order }`, followed by the events its commands cause, so the base log can show what the Caretaker did.

Preset card sets (postures, each with a name and summary from milestone 6) live in `content/standing_orders.ron`. `cargo run -p nexa-tools -- 10 --orders caretaker` runs one headless. The `caretaker` preset is three cards: at power below 25%, shut down the smelter, start heater_02 and divert line heat to hab_01, then swing the heat to whichever hab hasn't passed 16.0 C. It keeps both habs nominal with no added wear through Sol 45. The cost is the smelter, which freezes and goes Offline on Sol 5. Keeping the smelter alive as well needs near-perfect hour-by-hour control with the battery near empty, which a small card set can't do on the current content.

**The editor** (`nexa-game/src/orders_view.rs`, `[O]` from the colony screen) lists the cards with slot, priority and command text, and builds new ones from lists: a trigger kind, then its module and threshold (temperatures start at the module's current reading, in 0.5 C steps; power in 5% steps, then 1% steps below 5% so any value from 1 to 100% can be set), then an action and its module, then priority 1 to 9. The steps, ranges and highest priority are the `editor` block of `content/standing_orders.ron`; they aren't saved, since stored cards already hold their values. Heat actions only offer modules on a heat line. Backspace undoes the last part added, and deleting a card takes a second `D` to confirm. Cards are stored and deleted through `World::add_order` and `World::remove_order`, so a refused card stays open and the terminal shows the sim's `OrderError`. The colony is held while the terminal is open.

A typed-entry parser that produces the same enums can be added later; the slice doesn't need one.

**Crew and the Split.** The six crew come from `content/crew.ron`. Each has a `CrewId` (`cal`, `maya`, ...), a display name, an `adult` flag, skill ratings 0 to 5 per `Skill` (Piloting, Engineering, Medicine, Negotiation, Astrogation, Sensors, Rigging), one `Trait` (their psychological dynamic) and a bunk hab. A hab is a module with `kind: Hab` in the colony content, and a hab must have a machine. Content is checked at load: exactly six crew, unique IDs, no unknown fields, every bunk a hab in the colony, `pilot_min` from 1 to 5, at least one qualified pilot, stress thresholds 0 < calm < strained < breaking <= 10000 and rates <= 10000. Like the colony, `World.crew` (a `Crew`: the roster plus one `Assignment` per member) keeps its content definitions in the save. A save is refused if its crew fails those content checks, doesn't give exactly one side to each member, or has anyone flying under a Split that breaks the rules below. Saves from before crew get the shipped roster, all on the Ground, checked against that save's colony (`Roster::shipped`). Those saves also predate `ModuleKind`, so their `hab_*` modules are marked as habs on that path (`saves_from_before_crew_get_the_shipped_crew_and_their_habs`).

- **Specializations apply when their systems exist.** Three flight specializations apply from milestone 5 as skill-check bonuses, listed by `CrewId` per hazard option in `content/hazards.ron`: Cal on the countermeasure (Master Aviator), Jonathan on the Overclock Burn (Reactor Overclocking) and Dax on the bluff (The Face). A bonus counts only while that member is on the Flight side. The others (Jonathan's wider tolerances, Cal's shift discipline, Maya's duct runs, Oron-3's transit-day shaving, ...) are shown, not applied, until the systems they touch exist.
- **The Split.** Each crew member's `Assignment` is Ground or Flight; everyone starts on Ground. The player opens the crew screen (`[C]` from the colony screen) at any time and confirms through `World::confirm_split`, which enforces the two rules and refuses with a `SplitError`:
  - `NoQualifiedPilot`: no Flight crew member has Piloting at or above `pilot_min` (Cal only, on current content). Non-pilots flying by manual is out of slice scope.
  - `MayaWithoutAdult { child }`: Maya's side (Ground or Flight) has no other crew member with `adult: true`. Oron-3 counts as an adult. The rule applies to every crew member with `adult: false`; `child` names the one left uncovered.

  An assignment naming someone not in the crew is refused with `UnknownCrew`; crew left out of the assignments go to the Ground, so confirming again replaces the whole Split. A refused Split leaves the current one in place.

  Those are the only rules: everyone flying is allowed, and so is Maya flying. A confirmed Split emits `SimEvent::SplitConfirmed { flight }`, listing who flies in roster order; the base log shows how many. Launching *The Mullein* needs a confirmed Split with at least one Flight member (see *The Slip* below); while she is away the Split can't change (`SplitError::ShipAway`).
- **Stress and morale.** Both are integers in basis points (0 to 10000), kept per crew member in the save (`Vitals`), and stepped once per tick after the colony's hour (`Crew::step`). The crew land with no stress and full morale.
  - *Bunks:* a Ground crew member's stress rises while their bunk hab is cold (in Freeze or failed), recovers while it's nominal, and holds while it overheats. Flight crew aren't affected by their bunks.
  - *Father's Vigilance:* Jonathan gains extra stress while Maya is on the Ground and her bunk is cold, wherever he is. A flying Maya isn't in her bunk, so its cold doesn't count. In code that's any crew member with the trait, while any Ground child's (`adult: false`) bunk is cold.
  - *Morale* falls while stress is at or above `strained` and recovers while it's below `calm`: the same edges at which the levels are reached.
  - *Levels:* Calm, Strained and Breaking. A level is reached at its threshold but only left below the threshold under it: Strained eases to Calm below `calm`, Breaking to Strained below `strained`. Without that margin a hab flickering in and out of Freeze would flood the log with level changes (`crossing_a_stress_level_is_announced_once`). Each change emits `SimEvent::CrewStressChanged { crew, level }`.
  - *Saves* are refused if a member's stress or morale is missing, above 10000, or at a level their stress can't be at.

  Rates and thresholds live in `content/crew.ron`. On the Ash Plain with nobody tending the habs, Jonathan strains on Sol 4 and breaks on Sol 5, and everyone has broken by Sol 8 (`unattended_ash_plain_breaks_the_crew`). The `caretaker` preset keeps everyone calm through Sol 45 (`caretaker_orders_keep_the_crew_calm`). For tuning, `nexa-tools` logs each level change and ends with a table of each crew member's side, stress, level, morale, peak stress and hours spent Strained and Breaking; `--split cal,oron` confirms a Split before the run. The levels have no consequences yet; desertion, refusals and mistakes come with the systems that need them.

**The crew screen** (`nexa-game/src/crew_view.rs`, `[C]` from the colony screen) lists the six crew with their side, stress (percent and level) and morale, and below the list the selected member's age, bunk, skills and trait. Space (or Left/Right) flips the selected member between Ground and Flight on a draft; changed sides are starred until confirmed. Enter confirms through `World::confirm_split`: a confirmed Split goes to the base log, and a refused one keeps the draft and shows the sim's `SplitError`. Esc drops an unconfirmed draft and returns to the colony. The colony is held while the screen is open.

**Skill checks** use one function, `check(crew, &CheckDef, &CheckTuning, &mut rng) -> Roll`, with outcomes of Success, Partial and Fail. Every hazard and haggle resolution goes through it, which makes balancing and testing one place. A `CheckDef` names a `Skill`, a difficulty and specialist bonuses by `CrewId`; `CheckTuning` (`content/hazards.ron`) holds the formula's numbers. The Flight-side member with the best `skill * skill_weight + bonus` rolls (ties go to roster order): score = that + a die roll from 1 to `die_sides`. Score at or above the difficulty is a Success, within `partial_band` below it a Partial, lower a Fail. The `Roll` records who rolled, the die, score and outcome for the transit log.

**The Slip (milestone 5).** *The Mullein* (`content/mullein.ron`, kept in the save as `World.ship` like the colony's module definitions) has a 1,000 L tank, burns 350 L per leg and 150 L per Overclock Burn, all in whole liters; she carries one ECM pod and an outbound manifest of raw-silicon ore crates, each with a mass (kg) and an integrity percent. For now the manifest is fixed in the file; from milestone 6 it is whatever the smelter has produced by launch. `World.credits` is an integer balance that starts from the same file; a ransom spends it, and milestone 7's haggle builds on it.

- *Launch.* `World::launch(universe)` needs the ship on the pad, a confirmed Split with at least one Flight member, and fuel for the leg. It burns the leg's fuel, draws a seed from `World.rng`, asks the universe for a `LaunchBriefing` (`Universe::on_launch`) and starts a `Voyage`, which keeps a copy of the hazard content it launched with so a save resolves the same.
- *Transit.* `World::advance_voyage` moves one Sol at a time. The briefing schedules Slip events on Sols of the leg; when one comes up its hazard is pending and the voyage won't advance until `World::resolve_hazard(option)` settles it.
- *The interdiction* (`content/hazards.ron`) has five options: the countermeasure (Piloting check, spends an ECM pod, unavailable with none left), the Overclock Burn (Engineering check, burns 150 L; outbound it is unavailable if it would leave less than the return leg's 350 L, and on the return leg, whose 350 L is burned at launch, it only needs 150 L in the tank, even if that runs her dry; failure is engine drift, a severe delay), and the concession split three ways: dump cargo (certain, costs crates, unavailable with fewer aboard), pay the ransom (certain, costs credits, unavailable with too few) and bluff (Negotiation check). Each option has a cost paid up front and a `Toll` for Partial (escape with a minor cost: cargo integrity or a minor delay) and Fail.
- *Delays.* Minor hazard delays draw +5 to +7 Sols and severe drift +7 to +10. Random delays are capped at +12 Sols per round trip: the voyage keeps a running total so milestone 8's return leg respects the same cap. Player-caused delays (audits, from milestone 7) are uncapped.
- *The colony while away.* While the voyage is in transit `World::tick` holds the colony, so nothing runs twice. On arrival the colony catches up: it ticks through every Sol the voyage used, Standing Orders and Ground-crew stress included, and `advance_voyage` returns those events apart from the voyage's own so the base log can summarize them.
- *Holding.* Milestone 5 ends outbound. On arrival the ship holds over Scrap Barge 77 with the flight crew aboard, the colony runs normally with the Ground crew, and there is no relaunch until milestones 7 and 8 add docking, the haggle and the return leg.

Events: `Launched`, `SlipSolPassed`, `HazardEncountered`, `HazardResolved` (the option, the `Roll` if any, and what it cost after the cap) and `Arrived`.

*Tuning.* `cargo run -p nexa-tools -- --launch --seed 7 --resolve overclock` flies one voyage headless (after SOLS on the ground, default 0) and prints the transit log, each roll, what it cost and where the ship ended up; caught-up colony events are stamped with the Sol they fell in. `--sweep N` flies seeds 0..N per option and prints Success/Partial/Fail rates and mean fuel, cargo integrity, crates, credits and delay at arrival. Both fly Cal, Jonathan and Dax unless `--split` says otherwise. First-pass numbers (`content/hazards.ron`): `skill_weight` 3, a d20, `partial_band` 6 and difficulty 29 on all three checks, with a +3 specialist bonus. A rated-5 specialist aboard scores 18 + d20: 50% Success, 30% Partial, 20% Fail, so every check option is a real gamble and none beats the certain concessions outright. Without the specialist the next-best member is much weaker (Dax's Piloting 3 gives 9 + d20: 5% Success, 30% Partial, 65% Fail), which is the Split's price. The Mullein carries one ECM pod, so using it outbound leaves none for the return leg (milestone 8); the ransom (300 of the 500 starting credits) eats into the haggle budget; dumping costs 3 of 10 crates.

## Presentation layering plan

The game starts as a terminal inside a macroquad window and gains visuals layer by layer on the same front end, with no rewrite between layers.

| Layer | Adds | Made by | Trigger to start |
| --- | --- | --- | --- |
| 0 | Full loop playable as text: character-grid terminal, ASCII colony map, text events, keyboard input | Code | Now |
| 1 | CRT post-process shader (scanlines, curvature, phosphor glow, flicker), synthesized sound effects, ambient drones | Claude, in code | Loop is fun as text |
| 2 | Vector cockpit / station / haggle panels (SVG rendered to PNG), code-drawn isometric tiles and machine sprites | Claude, in code | Layer 1 done |
| 3 | Character portraits, painted scenes, music | Asset packs, AI image tools, later commissioned art | Time and budget allow |

**Terminal renderer.** A fixed grid of cells (glyph, ink color) drawn with a monospace font. Layer 1 renders it to an offscreen texture and runs the CRT shader over that texture. All screens (Standing Orders console, hazard terminal, haggle, Black Box log) are views drawn into this grid at first; Layer 2 replaces regions of it with sprite panels.

**Screens as views.** Each screen (`orders_view`, `crew_view`, `voyage_view`, plus the colony screen) is a struct with `draw(&self, t: &mut Terminal, world: &World, help_row)` and its own key handling. Views read state and change the world only through `World`'s public methods (`apply`, `add_order`, `confirm_split`, `launch`, `resolve_hazard`, ...), never by touching its fields, and show the sim's typed errors as they come back. There is no shared `View` trait yet; add one if the screens' shapes converge.

**UI toolkit.** No egui or other third-party UI library. A small bespoke immediate-mode UI draws on the terminal cell grid: boxed frames, list selectors, slot pickers, meters and scrolling logs in a monospace font. That keeps the amber CRT look authentic and the code fully under our control.

**Asset IDs with fallbacks.** An `assets/manifest.ron` maps IDs to the best available representation:

```ron
"machine.smelter": [Sprite("tiles/smelter.png"), Glyph('S', Amber)],
"portrait.dax":    [Image("portraits/dax.png"), CommFeed("DAX THORNE")],
```

The loader takes the first entry that exists, so a missing file silently falls back to the layer below.

**Audio cues.** The sim emits `SimEvent`s; the front end maps them to named cues (`alarm.overheat`, `dock.clamp`, `slip.ambient`) through `audio/cues.ron`. An unmapped cue is silent, a Layer 1 cue is a synthesized WAV, a Layer 3 cue is a recording.

## Platform targets

Desktop is the only shipped target until the game is done; the web build is kept compiling from the start so the later port is a build step, not a rewrite.

| Target | Build | When |
| --- | --- | --- |
| Desktop (native) | `cargo run -p nexa-game` | From day one |
| Web (WASM) | `cargo build -p nexa-game --target wasm32-unknown-unknown` + macroquad's JS loader | Port after the game is complete; both sim and game checked for WASM in CI from day one |
| Server (later) | `cargo run -p nexa-server` | Only when the shared universe is built |

**Portability rules (enforced in review and CI):**

- No file system access outside the `platform` module. Saves and settings go through a `Storage` trait: files on desktop, browser storage on web.
- Asset loading is async (`load_texture(...).await`), never blocking reads from disk.
- Single-threaded game loop: `loop { update(); draw(); next_frame().await }`. No `std::thread` in game or sim code.
- No `std::time::Instant` or `SystemTime` in sim code (it panics on WASM). Presentation uses macroquad's `get_time()`.
- No native-only crates in `nexa-sim` or `nexa-game` (no `crossterm`, no direct OS APIs).
- Audio starts only after the first key press ("PRESS ANY KEY TO BOOT TERMINAL"), which browsers require.

## Shared universe architecture

Each player owns a private colony that runs locally; stations, markets, reputation, distress calls and Echoes live in a shared universe that syncs only at launch and docking. It is designed now and built later; until then an offline implementation stands in.

**What is local vs. shared**

| Local (player's client, authoritative) | Shared (server, authoritative) |
| --- | --- |
| Colony grid, machines, parts, atmosphere | Station markets: prices, stock, tariffs |
| Crew, stress, morale, assignments | Station reputation: Integrity and Heat per player, plus station-wide mood |
| Standing Orders, Black Box log | Distress calls and wrecks posted by other players |
| Epoch progress | Echo ledger: favors owed between players |
| Voyage state while in The Slip | Constrained player messages left at stations |
| | Universe clock |

**The interface.** All game code reaches the universe through one trait. Milestone 5 builds only `on_launch` (in `nexa-sim/src/universe.rs`; `LaunchContext` and `LaunchBriefing` are `nexa-types` structs). The other methods are added when the systems behind them exist: `on_dock` and `submit_trade` with Scrap Barge 77 (milestone 7), `post` with distress calls and Echoes.

```rust
pub trait Universe {
    fn on_launch(&mut self, ctx: &LaunchContext) -> LaunchBriefing;      // pending echoes, slip events (built)
    // planned:
    fn on_dock(&mut self, station: StationId, report: &VoyageReport) -> DockState; // prices, reputation, posts
    fn submit_trade(&mut self, trade: &Trade) -> Result<TradeReceipt, TradeError>;
    fn post(&mut self, post: &UniversePost) -> Result<PostId, PostError>; // distress, message, echo seed
}
```

- `OfflineUniverse` (built now): seeded and deterministic from the `LaunchContext` seed, which the world draws from its own `SimRng`. For now its briefing schedules one Slip hazard (the pirate interdiction) on a Sol of the outbound leg; simulated NPC traffic, prices and Echoes follow. This is the single-player game.
- `OnlineUniverse` (later): same trait, talks to `nexa-server` over HTTPS. Falls back to cached state if offline, which The Slip's radio silence explains in the fiction.

**Sync points.** Launch fetches a briefing (pending Echoes, posted distress calls on the route). Docking uploads the voyage report and trades, and downloads current station state. Nothing syncs mid-flight. No real-time networking is needed.

**Server (later).** `nexa-server`: Rust, axum, Postgres, sharing `nexa-types` with the client so message formats cannot drift. Accounts via a simple token or a hosted auth provider. Expected cost at indie scale is one small VM plus a managed database.

**Time model.** Each colony keeps its own Sol clock, which pauses with the player. A Master Universe Day advances every 24 real hours. A docking stamps the voyage with the current universe day, and station prices and posts are evaluated at that time.

**Keeping a small universe alive.** NPC traffic from `OfflineUniverse` also runs on the server, so stations feel busy with 1 player or 1,000. Real players add to that traffic; they are never required for it.

**Anti-cheat.** The client controls its own colony, so cheating there only affects that player. Anything entering the shared world is validated:

- Trades are checked against plausible cargo mass, credits and the voyage's recorded timeline.
- Because the sim is deterministic, the server can replay a voyage from its seed and command log when a report looks suspicious, using the same `nexa-sim` crate.
- Economic values are integers end to end (principle 4), so replays match exactly.

**Moderation.** Player messages use a fixed vocabulary assembled from parts, rendered as terminal codes (`DISTRESS: FUEL // WARNING: PIRATES AT SLIP-2 // THANKS: CALLAHAN`). No free text, so there is almost nothing to moderate.

**Habits required now (cheap, also good for single-player):**

- [x] Every off-colony interaction goes through `Universe`
- [x] Universe events and posts are serializable `nexa-types` structs
- [x] Deterministic sim: seeded `SimRng`, fixed ticks, no wall clock
- [x] Integer or fixed-point math for credits, mass, fuel, prices
- [x] Stable IDs for content (`cal`, `hab_01`, `barge_77`: short text slugs). Players, ships and events that cross the server get UUIDs or ULIDs when `nexa-server` exists
- [ ] Command log recorded per voyage (also gives replays and bug reports)

## Content, data formats and saves

All content is RON (Rusty Object Notation) loaded through `serde` and validated when parsed. Today each file is compiled into `nexa-sim` with `include_str!`, so an edit needs a rebuild; hot reload on desktop can come later through the `platform` module.

| File | Holds |
| --- | --- |
| `content/mullein.ron` | *The Mullein*: tank, per-leg and Overclock burns, ECM pods, route and transit Sols, outbound ore-crate manifest, starting credits |
| `content/crew.ron` | The six crew: skills, traits, adult flag, bunk hab; pilot minimum, stress and morale rates and thresholds |
| `content/hazards.ron` | Slip hazard events, options, checks, outcomes, delay ranges |
| `content/stations/scrap_barge_77.ron` (*planned*, milestone 7) | Vendors, stock, patience, dialogue, the prospector dilemma |
| `content/epochs.ron` (*planned*, milestone 8) | Atmosphere targets and epoch requirements |
| `content/landing_ash_plain.ron` | Hand-built 32 x 32 active map; each module's machine (tolerance band, wear rate, starting wear), power and heat; heat lines; the starting parts stock |
| `content/standing_orders.ron` | Standing Orders tuning (terminal memory, re-arm margins) and preset card sets |
| `assets/manifest.ron` (*planned*, Layer 1) | Asset ID → fallback chain |
| `audio/cues.ron` (*planned*, Layer 1) | Event → sound cue |

**Saves.** A save is the serialized `World`, `SimRng` state included. The sim side is built and tested (a `World` round-trips through RON and old saves load); the game can't write or load one yet. That arrives with a `Storage` trait in the `platform` module: files on desktop, browser storage on web. Every save carries a `format_version`; a `migrate()` step upgrades old saves so playtesters don't lose progress across updates. The same format later serves as a server-side replay input.

A save also keeps the content it was made with: each module's `ModuleDef` and the Standing Orders tuning (terminal memory, re-arm margins). So a save replays the same after a balance change, and edits to `content/` reach only new games. The ship's definition and a voyage's hazard content are kept the same way. Saves from before milestone 5 get a fixed default RNG seed, the shipped *Mullein* on the pad and the shipped starting credits. `format_version` and `migrate()` aren't built yet; when they are, `migrate()` is where a deliberate rebalance reaches old saves.

**Validation.** Every content file is checked when it loads (crew, hazards, ship, colony layout, orders tuning), cross-references included (every bunk is a hab, every specialist bonus names a crew member). Tests load the shipped files, so broken content fails `cargo test`, not the playtest.

## Testing strategy and AI-coding workflow

Because the sim is pure and deterministic, most of the game can be tested without a window, which is what makes AI-driven iteration safe.

**Test layers**

- **Unit tests** per sim module: tolerance bands, flow routing, Standing Orders evaluation and rendering, skill checks.
- **Scenario tests**: load a content file, apply a scripted command list, assert on the resulting `World` (e.g. "heater off for 3 Sols freezes the seal").
- **Snapshot tests** (`insta`, *planned* with the Black Box in milestone 8): the Black Box log for a fixed seed must match a stored snapshot. Any balance change shows up as a readable diff.
- **Headless balance runs**: `nexa-tools` runs the colony for N Sols (optionally under an orders preset and a Split) and `--sweep N` flies N seeded voyages per hazard option. Survival rates and margin against the 45-Sol deadline follow once the full loop exists.
- **Playthrough scripts** (`scripts/`, run by `nexa_sim::script`): line-based player actions and `expect` checks. `nexa-tools --script` plays one headless; `nexa-game --script` plays it through the real screens with key presses and `--screenshot-at MOMENT,... --out DIR` saves PNGs; `--headless` runs it without a window. `--seed N` fixes the game's RNG. `crates/nexa-sim/tests/gdd_invariants.rs` checks the GDD's numbers; rules for unbuilt systems are ignored tests that record the expected values.
- **Property tests** (`proptest`, *planned*) on orders and the economy: no command sequence produces negative cargo or fuel, or creates credits from nothing.

**Working with AI**

- Keep `CLAUDE.md` at the repo root with these principles, the crate rules and the commands to build, test and lint.
- Work in small, single-purpose tasks ("add Freeze state with seal-crack failure + tests"), each ending green on `cargo test`, `cargo clippy -D warnings` and the WASM check.
- Ask for tests first on sim changes; review the test before the implementation.
- Treat compiler errors as the AI's feedback loop: let it iterate until it compiles and passes, then review the diff.
- Commit after every green step so any bad direction is one `git revert` away.

## Milestone roadmap

Milestones are ordered, not dated; each ends with something playable or testable. Milestones 1 through 8 are "Slice 0": the whole vertical-slice loop in text.

1. **Skeleton.** *(Done.)* Cargo workspace, crates, CI (fmt, test, clippy, WASM check), `CLAUDE.md`, a macroquad window showing an amber character grid with a boot screen and a real-time-with-pause clock.
2. **Grid and machines.** *(Done.)* Ash Plain map from data, power and heat routing, tolerance states, wear parts. Headless tests first, then an ASCII view.
3. **Standing Orders.** *(Done.)* Rule-card model, evaluator, terminal editor screen, Caretaker simulation running Sols unattended.
4. **Crew and the Split.** *(Done.)* Six crew from data, assignments, stress and morale, the Split screen with its two rules.
5. **The Slip.** *(Done.)* Launch, the outbound leg, fuel ledger, cargo integrity, credits, one pirate interdiction resolved by skill checks or a concession (the `check` function and the sim's seeded RNG land here, moved from milestone 4), arrival over Scrap Barge 77 as a holding state, and the colony catching up on the Sols the voyage used. `OfflineUniverse` introduced here with `on_launch` only. Followed by the testing harness: playthrough scripts (`scripts/`), `--seed`, `--script`, `--headless` and `--screenshot-at` in the game, and `gdd_invariants` checking the GDD's numbers.
6. **Colony gaps.** Smelter ore fills the manifest; scrubber saturation counts down to Sol 45; the Base Hypoxia stages (degraded, unconscious, lost) run in the sim. The Caretaker gets approachable: a battery guard on the smelter, named postures in plain words and a forecast before launch. Retune the Ash Plain, the guard and the postures so *Balanced* loads 10 crates by Sol 10 with no card built. Work order: `docs/milestone-6-plan.md`.
7. **Scrap Barge 77.** Docking (`Universe::on_dock`), haggle session with Silas Vane through `check`, Integrity and Heat, orbital audits (uncapped player-caused delay), the prospector dilemma, Echo seed.
8. **Homecoming.** Return leg (respecting the round trip's random-delay cap, Overclock on 150 L), landing, Black Box reconciliation, Carbon Filters installed (scrubber reset), first-rain milestone, the autopsy log on the cockpit terminal. **Slice 0 complete: playtest the loop as text.**
9. **Layer 1.** CRT shader, synthesized sound effects and ambience.
10. **Layer 2.** Vector panels for cockpit, haggle and Black Box; code-drawn isometric tiles.
11. **Web build.** WASM port, browser storage, playable link for testers.
12. **Shared universe.** `nexa-server`, `OnlineUniverse`, accounts, constrained messages, server-side NPC traffic.
13. **Layer 3.** Portraits, painted scenes, music as time and budget allow.

## Design decisions

All eight spec questions are decided, plus the post-milestone-5 review below the table. Three were adjusted to fit this architecture (marked below). The game design doc (`docs/game-design.md`) reflects all of them.

| Topic | Decision | Architecture note |
| --- | --- | --- |
| Time model | Real-time with pause for the colony (1x / 2x / 3x); discrete event steps for The Slip and stations | Sim unchanged: fixed ticks, front end sets speed. 1 tick = 1 hour, 26 ticks per Sol; at 1x, 1 hour = 5 s (about 2 min 10 s per Sol). On landing the sim fast-forwards the colony through the elapsed Sols |
| Colony view | Isometric, fixed angle | **Adjusted:** no 4-way rotation for now, since it multiplies sprite work by four. Layer 0 ASCII stays top-down; the sim grid doesn't depend on projection, so isometric is a Layer 2 rendering choice |
| Crew split | Player decides fully, with two rules: at least one qualified pilot on *The Mullein*; at least one adult stays with Maya | Rules validated in `crew` when the Split is confirmed. Oron-3 counts as an adult. Non-pilots flying via a flight manual is out of slice scope |
| Barge 77 merchant | Silas "Grip" Vane, scrap broker running the reclamation dock. Whoever goes negotiates; Dax is best | Haggle uses the negotiator's skill and that character's own dialogue set |
| Epoch 01 exit | Carbon filters only. Bore valves move to the start of Epoch 02 | Done in the GDD |
| Day budget | Nominal run 23 Sols (10 ground, 5 + 5 flight, 3 station). Random hazard delays capped at +12 Sols, so the worst random case lands on Sol 35 | **Adjusted:** the cap covers random hazards only. Player-caused audits and penalties stack uncapped, so one 14-Sol audit after bad luck lands on Sol 49 and loses the colony |
| Fail state | Hypoxia Sols 45 to 47, crew coma Sol 48, hard failure from Sol 49 with an autopsy report in the Black Box log | Effective deadline is Sol 48. **Adjusted:** the discovery is a cockpit terminal readout over the base umbilical link: scrubber saturation, O2 candles exhausted, biometric flatlines, then zero carrier on the Caretaker frequency |
| Prospector dilemma | The prospector always asks for 200 L. The player can give at most what still leaves the 350 L return burn (150 L after an outbound Overclock), or refuse | The station never lets a gift strand *The Mullein*; the terminal shows the most she can spare |
| Return-leg Overclock | Available with 150 L in the tank; may run her dry | The return leg's 350 L is burned at launch, so no reserve is held back |
| Silicon | The smelter produces the ore crates; 10 by Sol 10 in a well-run colony | Manifest becomes colony output, tying Standing Orders to trade value |
| Scrubber deadline | Saturation meter on `scrubber_01`, full on Sol 45; filters reset it | No oxygen network on the grid |
| Hypoxia slowdown | "Movement slows 50%" becomes half production and slower repairs | Nothing walks in the sim; movement can return with the isometric layer |
| ECM pods | One pod | Using it outbound leaves none for the return |
| Caretaker difficulty | Slice goals must be reachable without building a rule card: a battery guard (two dials), named postures in plain words, and a forecast before launch. Cards stay as the advanced option | Decided after the milestone-6 interview; replaces a planned `PowerAbovePercent` trigger. Guard in `colony`, forecast in `forecast`, postures in `content/standing_orders.ron` |
| Universe clock (shared universe) | One Master Universe Day per real 24 hours. Each colony's Sols stay local and pause with the player; the shared world is touched only at Slip entry and docking, stamped with the current universe day | Matches the sync-at-docking design. Later idea: colonies that reach Epoch 04 become permanent trade nodes on the shared star chart |
