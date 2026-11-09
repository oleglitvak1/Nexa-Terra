# Nexa Terra

A gritty neo-noir colony sim and tramp-freighter survival game. Keep a terraforming
colony alive while half your irreplaceable crew flies dangerous trade runs for the
parts the base needs to survive.

Written in Rust on [macroquad](https://github.com/not-fl3/macroquad). Desktop first,
browser (WASM) later. The game currently runs as an amber CRT-style terminal and will
gain visuals layer by layer.

## Run

```sh
cargo run
```

On Linux, macroquad needs `libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev`.
macOS and Windows need nothing extra.

Controls: any key boots the terminal; `Space` pauses, `1` / `2` / `3` set speed,
`O` opens the Standing Orders terminal, `C` the crew screen (the Split), `L` launches
*The Mullein*, `Esc` backs out of a screen or quits.

## Layout

```text
crates/
  nexa-types/   shared data types
  nexa-sim/     pure, deterministic simulation core
  nexa-game/    macroquad client: terminal renderer and screens
  nexa-tools/   headless dev tools
content/        game data (RON)
scripts/        playthrough scripts (see scripts/README.md)
docs/           game design and architecture
assets/         fonts, sprites, shaders, asset manifest (from Layer 1; empty now)
audio/          sound cues and generated sounds (from Layer 1; empty now)
```

See `CLAUDE.md` for engineering rules and checks.
