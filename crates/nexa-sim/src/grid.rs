//! The colony map: tiles, conduits and the power and heat networks they form.
//!
//! The map is a character layout from content. Conduit tiles that touch form a
//! network; a module joins whichever power and heat network its tiles touch.

use serde::{Deserialize, Serialize};

/// One map cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tile {
    Ground,
    Fence,
    Pad,
    PowerConduit,
    HeatLine,
    /// A power conduit and heat line sharing one tile.
    Trunk,
    /// Part of the module at this index in the colony's module list.
    Module(usize),
}

impl Tile {
    /// Layout glyph for every non-module tile. Module glyphs come from content.
    pub const LEGEND: [(char, Tile); 6] = [
        ('.', Tile::Ground),
        ('#', Tile::Fence),
        ('_', Tile::Pad),
        ('=', Tile::PowerConduit),
        ('~', Tile::HeatLine),
        ('+', Tile::Trunk),
    ];

    pub fn carries_power(self) -> bool {
        matches!(self, Tile::PowerConduit | Tile::Trunk)
    }

    pub fn carries_heat(self) -> bool {
        matches!(self, Tile::HeatLine | Tile::Trunk)
    }
}

/// Why a layout couldn't become a grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    Empty,
    RaggedRow {
        row: usize,
        expected: usize,
        found: usize,
    },
    UnknownGlyph {
        glyph: char,
        row: usize,
        col: usize,
    },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::Empty => write!(f, "layout has no rows"),
            LayoutError::RaggedRow {
                row,
                expected,
                found,
            } => write!(
                f,
                "layout row {row} is {found} tiles wide, expected {expected}"
            ),
            LayoutError::UnknownGlyph { glyph, row, col } => {
                write!(f, "unknown glyph {glyph:?} at row {row}, col {col}")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

/// Which network, if any, each module joins. Indexed like the module list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Networks {
    pub power_count: usize,
    pub heat_count: usize,
    pub module_power: Vec<Vec<usize>>,
    pub module_heat: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid {
    width: usize,
    height: usize,
    tiles: Vec<Tile>,
}

impl Grid {
    /// Build a grid from layout rows. `module_glyphs[i]` marks module `i`'s tiles.
    pub fn parse<S: AsRef<str>>(rows: &[S], module_glyphs: &[char]) -> Result<Self, LayoutError> {
        let width = rows
            .first()
            .ok_or(LayoutError::Empty)?
            .as_ref()
            .chars()
            .count();
        let mut tiles = Vec::with_capacity(width * rows.len());
        for (row, line) in rows.iter().enumerate() {
            let line = line.as_ref();
            let found = line.chars().count();
            if found != width {
                return Err(LayoutError::RaggedRow {
                    row,
                    expected: width,
                    found,
                });
            }
            for (col, glyph) in line.chars().enumerate() {
                let tile = Tile::LEGEND
                    .iter()
                    .find(|(g, _)| *g == glyph)
                    .map(|(_, t)| *t)
                    .or_else(|| {
                        module_glyphs
                            .iter()
                            .position(|g| *g == glyph)
                            .map(Tile::Module)
                    })
                    .ok_or(LayoutError::UnknownGlyph { glyph, row, col })?;
                tiles.push(tile);
            }
        }
        Ok(Self {
            width,
            height: rows.len(),
            tiles,
        })
    }

    /// Whether the tiles fill the grid and every module tile names one of
    /// `module_count` modules; a saved grid isn't built by [`Self::parse`].
    pub fn is_consistent(&self, module_count: usize) -> bool {
        self.width.checked_mul(self.height) == Some(self.tiles.len())
            && self
                .tiles
                .iter()
                .all(|t| !matches!(t, Tile::Module(m) if *m >= module_count))
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn tile(&self, col: usize, row: usize) -> Option<Tile> {
        (col < self.width && row < self.height).then(|| self.tiles[row * self.width + col])
    }

    fn neighbors(&self, i: usize) -> impl Iterator<Item = usize> {
        let (col, row) = (i % self.width, i / self.width);
        let (w, h) = (self.width, self.height);
        [
            (col > 0).then(|| i - 1),
            (col + 1 < w).then(|| i + 1),
            (row > 0).then(|| i - w),
            (row + 1 < h).then(|| i + w),
        ]
        .into_iter()
        .flatten()
    }

    /// Label touching conduit tiles into networks; returns (label per tile, count).
    fn label(&self, carries: fn(Tile) -> bool) -> (Vec<Option<usize>>, usize) {
        let mut labels = vec![None; self.tiles.len()];
        let mut count = 0;
        for start in 0..self.tiles.len() {
            if !carries(self.tiles[start]) || labels[start].is_some() {
                continue;
            }
            labels[start] = Some(count);
            let mut stack = vec![start];
            while let Some(i) = stack.pop() {
                for n in self.neighbors(i) {
                    if carries(self.tiles[n]) && labels[n].is_none() {
                        labels[n] = Some(count);
                        stack.push(n);
                    }
                }
            }
            count += 1;
        }
        (labels, count)
    }

    /// Networks each of `module_count` modules touches, sorted and deduplicated.
    pub fn networks(&self, module_count: usize) -> Networks {
        let (power_labels, power_count) = self.label(Tile::carries_power);
        let (heat_labels, heat_count) = self.label(Tile::carries_heat);
        let mut module_power = vec![Vec::new(); module_count];
        let mut module_heat = vec![Vec::new(); module_count];
        for (i, tile) in self.tiles.iter().enumerate() {
            let Tile::Module(m) = *tile else { continue };
            for n in self.neighbors(i) {
                if let Some(net) = power_labels[n] {
                    module_power[m].push(net);
                }
                if let Some(net) = heat_labels[n] {
                    module_heat[m].push(net);
                }
            }
        }
        for nets in module_power.iter_mut().chain(module_heat.iter_mut()) {
            nets.sort_unstable();
            nets.dedup();
        }
        Networks {
            power_count,
            heat_count,
            module_power,
            module_heat,
        }
    }

    /// Number of tiles belonging to each of `module_count` modules.
    pub fn module_tile_counts(&self, module_count: usize) -> Vec<usize> {
        let mut counts = vec![0; module_count];
        for tile in &self.tiles {
            if let Tile::Module(m) = *tile {
                counts[m] += 1;
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legend_and_module_glyphs() {
        let g = Grid::parse(&["#.A", "=~+"], &['A']).unwrap();
        assert_eq!((g.width(), g.height()), (3, 2));
        assert_eq!(g.tile(0, 0), Some(Tile::Fence));
        assert_eq!(g.tile(2, 0), Some(Tile::Module(0)));
        assert_eq!(g.tile(2, 1), Some(Tile::Trunk));
        assert_eq!(g.tile(3, 0), None);
    }

    #[test]
    fn rejects_ragged_rows() {
        assert_eq!(
            Grid::parse(&["...", ".."], &[]),
            Err(LayoutError::RaggedRow {
                row: 1,
                expected: 3,
                found: 2
            })
        );
    }

    #[test]
    fn rejects_unknown_glyphs() {
        assert_eq!(
            Grid::parse(&["..?"], &[]),
            Err(LayoutError::UnknownGlyph {
                glyph: '?',
                row: 0,
                col: 2
            })
        );
    }

    #[test]
    fn touching_conduits_form_one_network_and_gaps_split_them() {
        // A and B share a power line; C sits on a separate one.
        let g = Grid::parse(&["A==B.C=", "......."], &['A', 'B', 'C']).unwrap();
        let n = g.networks(3);
        assert_eq!(n.power_count, 2);
        assert_eq!(n.module_power, vec![vec![0], vec![0], vec![1]]);
        assert_eq!(n.module_heat, vec![Vec::<usize>::new(); 3]);
    }

    #[test]
    fn trunk_tiles_carry_both_power_and_heat() {
        let g = Grid::parse(&["A+B", "..."], &['A', 'B']).unwrap();
        let n = g.networks(2);
        assert_eq!(n.module_power, vec![vec![0], vec![0]]);
        assert_eq!(n.module_heat, vec![vec![0], vec![0]]);
    }

    #[test]
    fn power_and_heat_lines_side_by_side_stay_separate() {
        let g = Grid::parse(&["A=", "~."], &['A']).unwrap();
        let n = g.networks(1);
        assert_eq!((n.power_count, n.heat_count), (1, 1));
    }
}
