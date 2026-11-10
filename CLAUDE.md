# Nexa Terra: rules for AI-assisted work

Nexa Terra is a neo-noir colony sim / tramp-freighter game written in Rust on
macroquad (a library, not an engine). This file is the short version for
day-to-day coding. The full design lives in `docs/`:

- `docs/game-design.md`: what the game is (crew, systems, epochs, the vertical
  slice, day budget, fuel ledger, scope, definition of done).
- `docs/architecture.md`: how it's built (crates, sim modules, presentation layers,
  platform rules, shared universe, testing, milestone roadmap).

Read the relevant doc before starting a milestone. If code and docs disagree, ask
before changing either; update the docs in the same commit when a decision changes.

## Commands

```sh
cargo run                                   # play (nexa-game is the default member)
cargo run -p nexa-tools -- 45               # run the sim headless for 45 Sols
cargo run -p nexa-tools -- 10 --orders caretaker   # ... under a Standing Orders preset
cargo run -p nexa-tools -- 10 --split cal,oron     # ... with a confirmed Split
cargo run -p nexa-tools -- --launch --seed 7 --resolve overclock   # fly The Slip, print a voyage report
cargo run -p nexa-tools -- --sweep 1000            # outcome rates per hazard option (after 10 Sols under caretaker)
cargo run -p nexa-tools -- --script scripts/playthrough/a_nominal.nts   # play a script headless (scripts/README.md)
cargo run -- --seed 7                               # play with a fixed RNG seed
cargo run -- --script FILE --screenshot-at hazard,sol:12 --out shots    # scripted run through the real screens + PNGs
cargo test --workspace                      # all tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo build -p nexa-sim --target wasm32-unknown-unknown   # web-safety checks (CI runs both)
cargo build -p nexa-game --target wasm32-unknown-unknown
```

Every change must end with test, clippy (no warnings), fmt, and both WASM checks green.

## Crates and dependency rules

| Crate | Role | May depend on |
| --- | --- | --- |
| `nexa-types` | Shared serde data: IDs, time units, colony commands, Universe messages | serde only |
| `nexa-sim` | Pure deterministic rules and `SimEvent`s: grid, machines, crew, orders, flight, scripts; later station, echoes | `nexa-types`, small pure crates |
| `nexa-game` | Window, input, audio, terminal renderer, screens (views) | `nexa-sim`, `nexa-types`, macroquad |
| `nexa-tools` | Headless dev binaries: balance runs, content checks, generators | `nexa-sim`, `nexa-types` |
| `nexa-server` | Shared universe service (later, not yet created) | `nexa-sim`, `nexa-types` |

Dependencies point down only. Only `nexa-game` knows macroquad exists.

## Guiding principles (non-negotiable)

1. **The sim knows nothing about presentation.** `nexa-sim` never draws, plays sound,
   reads input, touches files, spawns threads or reads the clock. The front end sends
   commands and renders state and `SimEvent`s; it never mutates the world directly.
2. **Content is data.** Machines, parts, crew, events and tuning numbers go in RON files
   under `content/`, not in code.
3. **Determinism.** Same seed + same commands = same result on every platform. All
   randomness comes from the sim's seeded RNG. One tick = one planetary hour; 26 per Sol.
4. **Integers for anything economic or shared.** Credits, cargo mass, fuel (whole liters),
   prices, wear and temperatures (centi-degrees) are integers. Floats only in presentation.
5. **Everything is text.** Code, data, shaders, SVGs and generator scripts live in the repo.
6. **Assets and sounds by ID with fallbacks** (`machine.smelter`, `alarm.overheat`), never
   raw file paths in game logic.
7. **The universe sits behind one interface.** Anything off-colony (stations, markets,
   other ships, Echoes) goes through the `Universe` trait. Offline first, online later.

## Web portability rules

- No file I/O outside a `platform` module in `nexa-game`; saves go through a `Storage` trait.
- Assets load asynchronously. No blocking reads.
- Single-threaded loop: `loop { update; draw; next_frame().await }`.
- No `std::time::Instant` / `SystemTime` in sim code. The game uses macroquad's `get_time()`.
- No native-only crates (e.g. `crossterm`) in `nexa-sim` or `nexa-game`.

## Presentation

- Layer 0 is a fixed character-cell terminal (`crates/nexa-game/src/terminal.rs`) in the
  amber palette. All screens write into it. No egui or third-party UI library: we build a
  small immediate-mode UI (frames, list pickers, meters, logs) on the cell grid.
- The default font has no box-drawing glyphs; use ASCII (`+ - |`) for frames.
- Later layers add a CRT shader over the terminal and sprite panels into regions of it.

## How to work

- Small, single-purpose tasks. Each ends green on the checks above, then commit.
- Sim changes: write or update the test first, then the implementation.
- Prefer enums with exhaustive `match` for states (machine states, hazard types, orders).
- Standing Orders are typed enums (rule cards), rendered as `IF ... THEN ...` text for display.
- Keep comments short and about *why*; don't narrate the code.
