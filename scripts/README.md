# Playthrough scripts

Line-based scripts that play the game: one step per line, `#` comments. The
verbs and `expect` keys are documented in `crates/nexa-sim/src/script.rs`.

```sh
cargo run -p nexa-tools -- --script scripts/playthrough/a_nominal.nts          # headless, sim log
cargo run -p nexa-game -- --headless --script scripts/playthrough/a_nominal.nts  # headless, base-log words
cargo run -p nexa-game -- --script scripts/playthrough/a_nominal.nts \
    --screenshot-at hazard,resolved --out shots/a                                # real screens + PNGs
```

Exit codes: 0 ran to the end, 1 a step failed, 2 reached a system that isn't
built yet. Until milestone 6 every playthrough stops at its `dock` line with
exit 2; `crates/nexa-sim/tests/playthroughs.rs` checks that everything before
it passes.
