# Playthrough scripts

Line-based scripts that play the game: one step per line, `#` comments. The
verbs and `expect` keys are documented in `crates/nexa-sim/src/script.rs`.
`orders NAME` loads a posture from `content/standing_orders.ron`, replacing any
cards already loaded.
`expect ore KG` checks the smelter's ground stockpile, in kg: ore not yet
packed into a crate, or waiting while the hold is full or the ship is away.
Played through the real screens, `launch` presses `L` and then `Enter` on the
launch confirmation.

```sh
cargo run -p nexa-tools -- --script scripts/playthrough/a_nominal.nts          # headless, sim log
cargo run -p nexa-game -- --headless --script scripts/playthrough/a_nominal.nts  # headless, base-log words
cargo run -p nexa-game -- --script scripts/playthrough/a_nominal.nts \
    --screenshot-at hazard,resolved --out shots/a                                # real screens + PNGs
```

Exit codes: 0 ran to the end, 1 a step failed, 2 reached a system that isn't
built yet. Until docking exists (milestone 7) every playthrough stops at its `dock` line
with exit 2; `crates/nexa-sim/tests/playthroughs.rs` checks that everything before
it passes.
