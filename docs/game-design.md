# NEXA TERRA — Game Design Document & Vertical Slice Prototype Plan

---

## 1. Executive Summary & Core Pillars

* **Project Title:** *Nexa Terra*
* **Genre:** Gritty Neo-Noir Colony Sim / Tramp-Freighter Survival RPG
* **Target Platforms:** PC desktop (Steam / GOG) first; browser (WASM) build later
* **Technology:** Rust on macroquad, no game engine. Stylized 2D: amber CRT terminal UI, fixed-angle isometric colony view, CRT post-processing shader, systemic grid math. Built in layers from a text-only terminal up to illustrated panels (see *Nexa Terra — Technical Architecture*).
* **Multiplayer (planned):** Asynchronous shared universe. Each player owns a private colony; stations, markets, reputation and Echoes are shared and sync at launch and docking.
* **High Concept:** A character-driven survival sim where surface terraforming stalls without interplanetary trade, forcing the player to manage an irreplaceable specialist crew divided between an automated industrial base on the ground and high-stakes freight runs through hostile space.

### Core Design Pillars

1. **The Divided Crew Dilemma (Zero-Sum Personnel):** There are no nameless workers. Every crew member assigned to a flight leaves the ground base vulnerable; every specialist left on the surface impairs ship operations, navigation, or trading leverage.
2. **Tramp-Freighter Realism (Worn Industrial Scifi):** Space travel is an unglamorous, hazardous chore. Fuel margins are razor-thin, hulls degrade under stress, and cargo mass imposes severe flight penalties.
3. **Tactile Hybrid Perspectives:** Macro-level colony operations run via isometric strategy, while high-leverage moments (docking, cockpit transit hazards, duct repairs, station haggling, milestone events) switch to close-up diegetic screens: cockpit panels, the hazard terminal, duct cross-sections and the haggle counter.
4. **Hard-Earned Humanity (Costly Altruism & Commercial Trust):** Survival requires trade, and trade requires reputation. Honesty and charity carry heavy, life-threatening upfront costs, but pay delayed systemic dividends across the frontier.

---

## 2. The Ensemble Player Architecture

The player does not embody a single static avatar. The view gives strategic isometric oversight of the colony, and during hands-on moments it switches to the diegetic panel of the specialist on duty (their console, HUD overlay or crawlspace view).

```
                  ┌──────────────────────────────────────────────┐
                  │          ISOMETRIC STRATEGY LAYER            │
                  │  Colony routing, power grids, task queues    │
                  └──────────────────────┬───────────────────────┘
                                         │
                   [Trigger Event / Hands-On Task Initiated]
                                         │
                                         ▼
                  ┌──────────────────────────────────────────────┐
                  │        DIEGETIC SPECIALIST PANELS            │
                  └──────────────────────┬───────────────────────┘
                                         │
         ┌───────────────────────────────┼───────────────────────────────┐
         ▼                               ▼                               ▼
   [CAL CALLAHAN]                [JONATHAN WINTER]                 [MAYA WINTER]
Cockpit flight controls,       Analog circuit tuning,           Crawlspace duct repairs,
manual docking thrusters       engine overclocks, repairs       acoustic sensor listening
         │                               │                               │
         ▼                               ▼                               ▼
    [DAX THORNE]                 [DR. ELENA WINTER]                  [ORON-3]
Station bartering,             Biochemical synthesis,           Spectroscopic surveying,
sleight-of-hand, black market  surgical triage, spore care      geological core sampling

```

---

## 3. Roster Specification

| Character | Background | Surface Specialization | Flight / Expedition Specialization | Psychological Dynamic |
| --- | --- | --- | --- | --- |
| **Dr. Jonathan Winter** | Dissident propulsion engineer fleeing an imperial regime. | **Master Engineering:** Calibrates power plants, widens machine stress tolerances by 30%. | **Reactor Overclocking:** Maximizes jump efficiency in The Slip; resolves engine hazard checks. | *Father's Vigilance:* Suffers extreme stress if Maya is exposed to danger, cold, or malnutrition. |
| **Dr. Elena Winter** | Senior trauma surgeon and biological pathologist. | **Biosphere & Triage:** Treats acute radiation, dust-lung, and manages hydroponic chemistry. | **Bio-Preservation:** Prevents imported terraforming cultures from spoiling in transit. | *Hippocratic Anchor:* Suffers severe morale penalties if crew or refugees are neglected or harmed. |
| **Maya Winter (Age 8)** | Precocious, agile daughter; learns rapidly from the crew. | **Duct Runner:** Crawls into sub-floor conduits to flip emergency bypasses during overloads. | **Acoustic Sensor Tuning:** Spots radar locks and anomaly pings seconds before automated systems. | *Living Compass:* Serves as the moral barometer. Empathy or cynicism shapes her development. |
| **Capt. Cal Callahan** | Burned-out ex-fleet combat pilot seeking quiet frontier runs. | **Rigging & Discipline:** Enforces rigid shifts, reducing human-error accidents during Caretaker Cycles. | **Master Aviator:** Precision manual docking; unlocks advanced evasive maneuvers at hazard terminals. | *Combat Fatigue:* Shuts down in bureaucratic corporate stations; thrives in the quiet void of The Slip. |
| **Oron-3** | Silicon-carbon Xenogeologist studying abiotic-biotic transitions. | **Barometric Foresight:** Accurately forecasts weather, seismic tremors, and toxic fronts 24h early. | **Astrogation:** Calculates gravitational slingshots, shaving transit days off trade voyages. | *Clinical Detachment:* Immune to human claustrophobia; evaluates all events through planetary data. |
| **Dax Thorne** | Charismatic runaway smuggler dodging syndicate bounties. | **Improvised Rigging:** Patches structural breaches using raw scrap instead of manufactured parts. | **The Face:** Slashes trade tariffs; uncovers black-market catalogs; bluffs pirate raiders. | *Flight Risk:* Susceptible to deserting at major freeports if colony rations and morale collapse. |

---

## 4. Systems Architecture

### A. The Stress-and-Thermal Colony Grid

Base modules do not degrade on arbitrary timers. Breakdowns are the direct consequence of operating machines outside their rated tolerances.

* **Operating Tolerances:** Running a machine within its **Nominal Band** produces 0% wear.
* **Thermal Stress:** Freezing temperatures lock lubricants and crack metal housings; overheating causes insulation meltdowns and electrical fires.
* **Component-Based Repairs:** Machines require specific manufactured wear parts:
* *Tungsten Heating Filaments* (freeze prevention)
* *Hermetic Viton Seals* (toxic gas containment)
* *Silicate Baffle Mesh* (coarse mechanical dust filtration for mining machinery; not interchangeable with the life-support Carbon Filters, which need microgravity fabrication off-world)


* **The Standing Orders Protocol:** Before departing on a trade run, the player sets systemic fail-safes via an amber CRT terminal (e.g., `IF Grid Power < 25% THEN Shut Down Smelter AND Divert Heat to Hab 01`). Orders are built as rule cards: the player fills trigger, action and priority slots from lists, and the terminal displays the result in this command syntax. Typed entry may be added later. Most players never need to build one: they pick a named posture (*Protect the habs*, *Balanced*, *Max ore*) described in plain words, set the smelter's battery guard with two dials, and read a forecast of what the base will do while they're away. Building cards is the advanced option for players who want finer control.

```
[Power / Thermal Distribution] ──► [Nominal Operation: 0% Wear]
              │
              ├──(Overclock / High Heat)──► [Burnout: Filament Destroys] ──► Requires New Filament
              │
              └──(Freeze / Power Cut)─────► [Frozen Block: Seal Cracks]  ──► Requires New Viton Seal

```

### B. The Black Box Run & Hazard Terminal

Space flight is represented through a tactile cockpit interface and an emergency communications terminal rather than real-time arcade dogfighting.

* **The Slip:** The radio-silent transit corridor between Nexa Terra and target orbital stations.
* **The Terminal Event Matrix:** Interdictions, debris clouds, or distress signals trigger a tactical decision screen:
* *Option 1: Tactical Countermeasure (Hardware-Dependent):* Deploy Chaff or ECM pods (Pilot check).
* *Option 2: Engineering Overclock (Resource-Dependent):* Burn 15% of tank capacity (150 L) in reaction mass to outrun the vector (Engineer check; failure causes engine drift delay).
* *Option 3: Economic Concession / Bluff (Social-Dependent):* Dump raw cargo, pay a credit ransom, or bluff using false transponder codes (Negotiator check).


* *Specialists and supplies:* the best qualified member of the flight crew makes each check, and a flight specialist aboard adds a bonus: Cal on the countermeasure, Jonathan on the Overclock Burn, Dax on the bluff. ECM pods are limited: *The Mullein* carries one, and the countermeasure spends it, so using it outbound leaves none for the return leg. A ransom needs the credits to pay it.
* *Outcomes:* a check succeeds, partly succeeds or fails. A partial escape still costs something small (some cargo integrity or a minor delay); a failure costs more.
* **Fuel Ledger (Vertical Slice):** *The Mullein* carries a 1,000 L reaction mass tank. A clean round trip burns 700 L (350 L each way), leaving a 300 L reserve. One Overclock Burn costs 150 L. Outbound, an Overclock must leave the 350 L for the return leg; on the return leg (whose 350 L is burned at launch) it only needs 150 L in the tank and may run her dry. All fuel is tracked in whole liters.
* **Systemic Failure Impacts:** Encounters do not deal generic hull hit points; they cause cargo seal blowouts, fuel leaks, or transit delays that eat directly into the colony's ground survival buffers. Random hazard delays run +5 to +7 Sols (minor) or +7 to +10 Sols (severe drift), and are capped at **+12 Sols** per round trip so bad luck alone never kills the colony.

### C. Station Commerce & Morale Matrix

Stations are compact, atmospheric hubs consisting of the **Gantry**, the **Exchange**, and the **Spigot**.

* **The Haggle Matrix:** Bartering balances **Vendor Patience** against **Crew Leverage**. Pushing for razor-thin margins risks irritating the merchant and locking inventory.
* **Commercial Integrity vs. Station Heat:**
* *Integrity (Honesty):* Accurate assays, paying tariffs, and honoring contracts unlock merchant credit lines, rapid docking clearance, and certified high-grade bio-cultures.
* *Heat (Deception):* Salting ore crates, bribing inspectors, and forging naval codes yield quick profits, but trigger mandatory orbital cargo audits (costing 7–14 Sols of delay) and attract pirate interdictions. Player-caused delays are **not** covered by the random-hazard cap, so deception can still cost the colony.


* **Altruism & The Echo System:** Sacrificing resources to help stranded ships or sick station workers produces no immediate reward. Instead, it triggers delayed "Echoes" in later epochs (e.g., an unexpected escort during an ambush, or waived docking impounds during a crisis).

---

## 5. Terraforming Epochs & Chemical Cascades

```
[EPOCH 01: ASH & SLAG]        [EPOCH 02: CONDENSATION]     [EPOCH 03: THE MOSS LINE]     [EPOCH 04: OPEN CANOPY]
• Patm: 0.12 bar              • Patm: 0.45 bar             • Patm: 0.78 bar              • Patm: 0.98 bar (Breathable)
• Tsurf: -48°C                • Tsurf: -10°C               • Tsurf: +8°C                 • Tsurf: +18°C
• Toxicity: 82%               • Toxicity: 55%              • Toxicity: 18%               • Toxicity: < 0.5%
• Liquid Water: 0%            • Liquid Water: 12% (Acid)   • Liquid Water: 45% (Runoff)  • Liquid Water: 70% (Living Cycle)

```

1. **Epoch 01 (Ash & Slag):** Closed-circuit life support whose scrubbers saturate on Sol 45; smelting local silicates into raw-silicon ore crates for trade; gathering frozen regolith core ice. Advancing requires off-world **Micro-Porous Carbon Filters**. Installing them stabilizes the scrubbers and triggers the first sulfur-acid drizzle (the vertical slice's ending milestone).
2. **Epoch 02 (Cold Condensation):** Industrial venting of greenhouse gases heats the crust, releasing corrosive acid slush. The epoch opens with importing **High-Pressure Bore Valves**, which unlock deep-aquifer heating. Advancing requires importing bulk **Liquid Nitrogen Canisters** and **Catalytic Crackers** to stabilize the atmosphere.
3. **Epoch 03 (The Moss Line):** Rain patterns wash sulfur out of the sky; soil remains chemically hostile. Advancing requires importing **Genetically Stabilized Lichen Vats** and **Nitrifying Micro-Fauna** transported in powered bio-holds.
4. **Epoch 04 (Open Canopy — Victory Condition):** Surface air reaches human-survivable pressure and non-toxic thresholds. Decommission base airlocks, open civilian landing beacons, and step onto the surface without helmets.

---

## 6. The Vertical Slice Prototype Plan

The goal of the Vertical Slice is a fully playable 45-to-60 minute slice demonstrating the entire core loop: **Landfall → Ground Triage → The Split → The Black Box Run → Station Barter → Touchdown & Reconciliation.**

```
                     VERTICAL SLICE FLOW (NOMINAL RUN: 23 SOLS, SCRUBBER DEADLINE: SOL 45)

[Sols 00 - 10]                    [Sol 10]                          [Sols 10 - 23]                    [Sol 23+]
PHASE 1: LANDFALL                 PHASE 2: THE SPLIT                PHASE 3: THE BLACK BOX RUN        PHASE 4: RECONCILIATION
• Hard touchdown in dust flat     • Configure Standing Orders       • Cockpit panel launch            • Re-entry and landing
• Connect hab power conduits      • Assign Caretakers (1+ adult     • Hazard terminal pirate event    • Black Box terminal tally
• Stabilize oxygen scrubbers        stays with Maya)                • Dock at Scrap Barge 77          • Install new carbon filters
• Identify filter failure clock   • Assign Flight Crew (1+ pilot)   • Haggle with Silas Vane          • First sulfur rain on the
                                  • Load manifest: raw silicon      • Prospector dilemma                viewport
```

### Time, Day Budget & Fail State

* **Time model:** The colony runs real-time with pause (1x / 2x / 3x). One Sol is 26 hours; at 1x, one hour passes in 5 seconds (about 2 min 10 s per Sol), so the ~10 Sols of ground prep take roughly 20–22 minutes. The Slip and station phases advance as discrete events, and the colony clock jumps forward by the Sols the voyage used. The jump is a real catch-up: the colony runs every hour of those Sols, so the Standing Orders and the Ground crew's stress play out while the ship is away.
* **Day budget:**

| Phase | Duration | Cumulative | Buffer to Sol 45 |
| --- | --- | --- | --- |
| Ground prep & mining | 10 Sols | 10 | 35 |
| Flight (5 out + 5 back) | 10 Sols | 20 | 25 |
| Station docking & barter | 3 Sols | 23 | 22 |
| Worst-case random delays (cap) | +12 Sols | 35 | 10 |

  Player-caused audits and penalties (7–14 Sols) stack on top of the cap. One 14-Sol audit after maximum bad luck lands on Sol 49 and loses the colony.
* **The scrubber clock:** the life-support scrubber carries a saturation meter that fills each hour it runs and is full on Sol 45; hours without power make it worse. Installing the imported Carbon Filters resets it.
* **Ore:** the smelter turns out raw-silicon ore crates while it runs in its nominal band. A well-run colony fills *The Mullein*'s 10-crate (10-ton) manifest by the nominal Sol-10 launch while keeping the habs warm; launching with fewer is allowed and leaves less to trade.
* **Fail state (Base Hypoxia):** Sols 45–47 the scrubbers saturate, emergency O2 candles burn power, the smelter runs at half rate, repairs slow, crew stress climbs and Maya's vitals go critical. Sol 48 the ground crew fall unconscious and production stops. From Sol 49, *The Mullein* docks to silence and the cockpit terminal prints the base log over the umbilical link (scrubber saturation, O2 candles exhausted, biometric flatlines, then `ZERO CARRIER DETECTED ON CARETAKER FREQUENCY`), followed by the autopsy report and a reload prompt.

---

### Step-by-Step Implementation Roadmap

Detailed, ordered milestones live in *Nexa Terra — Technical Architecture* (`docs/architecture.md`); the phases below group the work by system, not by build order. Panel screens and overlays are Layer 2 work. The slice is first built entirely as an amber text terminal ("Slice 0"), then layered with a CRT shader, synthesized audio, vector panels and isometric tiles.

#### Phase 1: Mechanics & The Grid Engine

* Implement the colony grid with resource routing (Power, Thermal Fluid); oxygen is the scrubber's saturation meter, not a network. ASCII top-down first; fixed-angle isometric tiles at the visual layer.
* Code the machine tolerance logic: Nominal, Overheat, Freeze, and Burnout states. The smelter produces ore crates while nominal.
* Build the data schema for discrete manufactured wear parts (*Tungsten Filaments*, *Viton Seals*, *Carbon Filters*).
* Implement Standing Orders as typed rule cards (trigger, action, priority) and the evaluator for the automated Caretaker Shift.

#### Phase 2: The Tramp Freighter & The Flight Loop

* Build *The Mullein* as four diegetic panel screens (Cockpit, Hab Commons, Engineering Core, Ventral Cargo Hold).
* Implement character-specific panel overlays (Cal's flight HUD, Jonathan's voltage diagnostics, Maya's duct cross-section).
* Build the diegetic Comms Hazard Terminal: the 3-choice interdiction encounter (ECM, Overclock Burn, Cargo Concession), with the concession offered as dump cargo, pay ransom or bluff.
* Script the math for travel delays (with the +12 Sol random cap), reaction mass consumption, and cargo integrity loss.

#### Phase 3: The Frontier Outpost & Commerce Slice

* Build **Scrap Barge 77** as three connected panel scenes (The Docking Gantry, The Exchange Counter, The Spigot Back Corridor).
* Implement the Haggle Matrix against the merchant **Silas "Grip" Vane**, an abrasive, aging scrap broker with a cybernetic ocular rig who runs the reclamation dock. Whoever made the trip negotiates with their own skills and dialogue options; Dax is the strongest negotiator.
* Track moral standing (*Commercial Integrity* vs. *Station Heat*).
* Script the moral dilemma: a stranded prospector begs for 200 L of reaction mass. Keeping it leaves the 300 L reserve, enough for an Overclock Burn on the return leg. Giving it leaves 100 L: enough to get home on a clean run, but not enough to burn, so a return interdiction must be solved by dumping cargo, paying, bluffing or a risky countermeasure roll. The terminal never lets the gift strand the ship: the player can give at most what still leaves the 350 L return burn. If an outbound Overclock was already used (500 L at the barge), that is 150 L, and nothing is left to burn on the way home.
* Hook up the "Echo System" trigger to log the player's moral choice for downstream consequences.

#### Phase 4: Systems Integration & The Black Box Reconciliation

* Connect the departure sequence to the colony's autonomous background simulation.
* Build the CRT **Black Box Log** displayed upon landing: resource deltas, machine overloads, and personal character incident logs.
* Implement the Epoch 01 → Epoch 02 milestone: installing the imported carbon filters stabilizes the scrubbers, and the first drops of sulfur-acid drizzle streak the viewport.
* Implement the Base Hypoxia fail state and terminal autopsy log.
* Audio & Post-Processing Pass: CRT scanline shader, amber phosphor glow, synthesized hydraulic clatter, and analog synth soundscapes.

---

## 7. Vertical Slice Scope & Boundary Control

To keep the prototype achievable, strict development boundaries are enforced:

### In Scope for Prototype

* **Characters:** Full 6-person crew with skill matrices, traits and written terminal logs.
* **Environments:**
  * 1 Planetary Landing Site (Nexa Terra - Ash Plain): a 32 x 32 tile active playable footprint centered on *The Mullein*'s landing pad, inside a perimeter fence. The surrounding 100m x 100m area renders as non-navigable background terrain (sulfur flats, dust plumes).
  * 1 Ship (*The Mullein* - 4 compartments as diegetic panel screens).
  * 1 Trade Station Hub (Scrap Barge 77 - 3 connected panel scenes).
* **Systems:** Stress-and-Thermal power grid; Standing Orders Caretaker engine; Hazard Comms Terminal; Haggle Matrix; Black Box reconciliation log; Base Hypoxia fail state.
* **Terraforming:** Epoch 01 complete, ending on the transition milestone to Epoch 02 (The First Raindrop).

### Strictly Out of Scope for Prototype

* 3D environments, first-person movement, and camera rotation (fixed isometric angle only).
* Voice acting and painted character art (portraits are added later as time and budget allow).
* Real-time space flight or exterior ship dogfighting.
* Procedural planet generation (landing site is handcrafted).
* Epochs 03 and 04 (The Moss Line and Open Canopy are reserved for full production).
* Additional star systems or secondary trading freeports (locked to Scrap Barge 77).
* Online shared universe (planned; the slice uses an offline simulated universe).
* Complex multi-branching tech trees (limited to baseline life support and extraction upgrades).

---

## 8. Definition of Done (Playable Prototype Milestone)

The prototype is verified as complete when an external playtester, starting with no prior instruction, can:

1. Touch down on Nexa Terra, triage the damaged landing habs, and diagnose the 45-Sol life-support scrubber deadline.
2. Route power through the grid to keep the smelter running and fill the 10-ton raw-silicon manifest, while managing thermal heating lines to prevent frozen seals.
3. Split the crew (at least one pilot aboard, at least one adult with Maya), leave adequate Standing Orders for the Caretakers, and launch *The Mullein* into The Slip.
4. Resolve an in-flight pirate interdiction via the cockpit terminal using a skill check or cargo concession.
5. Dock at Scrap Barge 77, conduct a multi-turn negotiation with Silas "Grip" Vane (led by Dax or whoever made the trip) to secure replacement Carbon Filters, and resolve a high-stakes charity dilemma.
6. Fly the return leg, land safely on Nexa Terra, review the Black Box reconciliation report of automated base events, and install the new filters to trigger the first rain milestone.
