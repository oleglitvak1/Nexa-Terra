//! Everything that differs between desktop and web: command-line options,
//! reading files and saving screenshots. The only module that touches the
//! file system. On the web there are no arguments, so every option is off.

use macroquad::prelude::Image;
#[cfg(test)]
use macroquad::prelude::BLACK;
use std::path::{Path, PathBuf};

/// When to save a screenshot during a scripted run, besides `screenshot` steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShotAt {
    /// A sim moment by name: `split`, `launched`, `hazard`, `resolved`, `arrived`.
    Event(String),
    /// The first frame on or after this Sol.
    Sol(u64),
}

pub const SHOT_EVENTS: &[&str] = &["split", "launched", "hazard", "resolved", "arrived"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub seed: Option<u64>,
    pub headless: bool,
    pub script: Option<String>,
    pub shots_at: Vec<ShotAt>,
    pub out: Option<PathBuf>,
}

/// Read the options from arguments (without the program name).
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--seed" => {
                let v = value()?;
                opts.seed = Some(v.parse().map_err(|_| format!("bad seed {v:?}"))?);
            }
            "--headless" => opts.headless = true,
            "--script" => opts.script = Some(value()?),
            "--out" => opts.out = Some(PathBuf::from(value()?)),
            "--screenshot-at" => {
                for part in value()?.split(',').filter(|p| !p.is_empty()) {
                    opts.shots_at.push(shot_at(part)?);
                }
            }
            _ => return Err(format!("unexpected argument {arg:?}")),
        }
    }
    if opts.headless && opts.script.is_none() {
        return Err("--headless needs --script".into());
    }
    if opts.headless && (opts.out.is_some() || !opts.shots_at.is_empty()) {
        return Err("--headless has no screen to capture".into());
    }
    if !opts.shots_at.is_empty() && opts.out.is_none() {
        return Err("--screenshot-at needs --out DIR".into());
    }
    if (!opts.shots_at.is_empty() || opts.out.is_some()) && opts.script.is_none() {
        return Err("screenshots need --script".into());
    }
    Ok(opts)
}

fn shot_at(text: &str) -> Result<ShotAt, String> {
    if let Some(sol) = text.strip_prefix("sol:") {
        return sol
            .parse()
            .map(ShotAt::Sol)
            .map_err(|_| format!("bad Sol in {text:?}"));
    }
    if SHOT_EVENTS.contains(&text) {
        return Ok(ShotAt::Event(text.to_string()));
    }
    Err(format!(
        "unknown screenshot moment {text:?}; use sol:N or one of {}",
        SHOT_EVENTS.join(", ")
    ))
}

/// The options this process was started with; on the web, none.
pub fn options() -> Result<Options, String> {
    if cfg!(target_arch = "wasm32") {
        return Ok(Options::default());
    }
    parse(std::env::args().skip(1))
}

/// End the process with `code`. The web has no process to end: the page
/// stays as it is.
pub fn exit(code: i32) {
    #[cfg(not(target_arch = "wasm32"))]
    std::process::exit(code);
    #[cfg(target_arch = "wasm32")]
    let _ = code;
}

pub fn read_text(path: &str) -> Result<String, String> {
    if cfg!(target_arch = "wasm32") {
        return Err(format!("can't read {path}: no files on the web"));
    }
    std::fs::read_to_string(path).map_err(|e| format!("can't read {path}: {e}"))
}

/// Save `image` as `dir/name.png`, creating `dir`; returns the path.
pub fn save_png(image: &Image, dir: &Path, name: &str) -> Result<PathBuf, String> {
    if cfg!(target_arch = "wasm32") {
        return Err("no files on the web".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let path = dir.join(format!("{name}.png"));
    // `export_png` panics on a failed write, so open the file first to turn
    // the usual failures (permissions, a directory in the way) into errors.
    std::fs::File::create(&path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    image.export_png(&path.to_string_lossy());
    match std::fs::metadata(&path) {
        Ok(meta) if meta.len() > 0 => Ok(path),
        _ => Err(format!("nothing written to {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Result<Options, String> {
        parse(text.split_whitespace().map(String::from))
    }

    #[test]
    fn no_arguments_is_a_normal_game() {
        assert_eq!(args(""), Ok(Options::default()));
    }

    #[test]
    fn reads_seed_script_and_screenshots() {
        let o = args("--seed 7 --script a.nts --screenshot-at hazard,sol:12 --out shots").unwrap();
        assert_eq!(o.seed, Some(7));
        assert_eq!(o.script.as_deref(), Some("a.nts"));
        assert_eq!(
            o.shots_at,
            vec![ShotAt::Event("hazard".into()), ShotAt::Sol(12)]
        );
        assert_eq!(o.out, Some(PathBuf::from("shots")));
    }

    #[test]
    fn refuses_what_it_cant_do() {
        assert!(args("--seed x").is_err());
        assert!(args("--seed").is_err());
        assert!(args("--headless").is_err());
        assert!(args("--headless --script a --out d").is_err());
        assert!(args("--script a --screenshot-at hazard").is_err());
        assert!(args("--script a --screenshot-at docked --out d").is_err());
        assert!(args("--out d").is_err());
        assert!(args("--fast").is_err());
    }

    #[test]
    fn a_screenshot_that_cant_be_written_is_an_error() {
        let dir = std::env::temp_dir().join(format!("nexa-shot-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("taken.png")).unwrap();
        let image = Image::gen_image_color(2, 2, BLACK);
        assert!(save_png(&image, &dir, "taken").is_err());
        assert!(save_png(&image, &dir, "fresh").is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
