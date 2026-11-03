//! A fixed character-cell terminal: the game's only renderer at Layer 0.
//!
//! Screens write glyphs into the grid; `draw` paints the grid to the window.
//! Later layers add a CRT shader over this output and replace regions of the
//! grid with sprite panels, without changing how screens write to it.

use macroquad::prelude::*;

/// Ink colors of the amber phosphor palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Bright,
    Amber,
    Dim,
}

impl Ink {
    fn color(self) -> Color {
        match self {
            Ink::Bright => Color::from_rgba(255, 214, 128, 255),
            Ink::Amber => Color::from_rgba(255, 176, 0, 255),
            Ink::Dim => Color::from_rgba(150, 96, 16, 255),
        }
    }
}

/// Screen background: a warm near-black.
pub const BACKGROUND: Color = Color::new(0.06, 0.04, 0.02, 1.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub ink: Ink,
}

impl Cell {
    const BLANK: Cell = Cell {
        ch: ' ',
        ink: Ink::Amber,
    };
}

pub struct Terminal {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![Cell::BLANK; cols * rows],
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::BLANK);
    }

    #[cfg_attr(not(test), allow(dead_code))] // used by tests now, screens later
    pub fn get(&self, col: usize, row: usize) -> Option<Cell> {
        (col < self.cols && row < self.rows).then(|| self.cells[row * self.cols + col])
    }

    /// Write text starting at (col, row). Text past the right edge is clipped.
    pub fn put_str(&mut self, col: usize, row: usize, text: &str, ink: Ink) {
        if row >= self.rows {
            return;
        }
        for (i, ch) in text.chars().enumerate() {
            let c = col + i;
            if c >= self.cols {
                break;
            }
            self.cells[row * self.cols + c] = Cell { ch, ink };
        }
    }

    /// Write text centered horizontally on a row.
    pub fn put_centered(&mut self, row: usize, text: &str, ink: Ink) {
        let len = text.chars().count();
        let col = self.cols.saturating_sub(len) / 2;
        self.put_str(col, row, text, ink);
    }

    /// Draw an ASCII frame. Uses plain ASCII because the default font has no
    /// box-drawing glyphs.
    pub fn frame(&mut self, col: usize, row: usize, width: usize, height: usize, ink: Ink) {
        if width < 2 || height < 2 {
            return;
        }
        let right = col + width - 1;
        let bottom = row + height - 1;
        let horizontal = format!("+{}+", "-".repeat(width - 2));
        self.put_str(col, row, &horizontal, ink);
        self.put_str(col, bottom, &horizontal, ink);
        for r in row + 1..bottom {
            self.put_str(col, r, "|", ink);
            self.put_str(right, r, "|", ink);
        }
    }

    /// Paint the grid to the window, scaled to fill it.
    pub fn draw(&self) {
        clear_background(BACKGROUND);
        let cell_w = screen_width() / self.cols as f32;
        let cell_h = screen_height() / self.rows as f32;
        // Size the font so a glyph fills most of a cell's width, without
        // growing taller than the cell.
        let probe = measure_text("M", None, 100, 1.0).width.max(1.0);
        let font_size = (100.0 * cell_w * 0.92 / probe).min(cell_h * 1.05) as u16;
        let mut buf = [0u8; 4];
        for row in 0..self.rows {
            for col in 0..self.cols {
                let cell = self.cells[row * self.cols + col];
                if cell.ch == ' ' {
                    continue;
                }
                let x = col as f32 * cell_w;
                let y = row as f32 * cell_h + cell_h * 0.8;
                draw_text_ex(
                    cell.ch.encode_utf8(&mut buf),
                    x,
                    y,
                    TextParams {
                        font_size,
                        color: cell.ink.color(),
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_str_clips_at_right_edge() {
        let mut t = Terminal::new(5, 2);
        t.put_str(3, 0, "ABCD", Ink::Amber);
        assert_eq!(t.get(3, 0).unwrap().ch, 'A');
        assert_eq!(t.get(4, 0).unwrap().ch, 'B');
        assert!(t.get(5, 0).is_none());
    }

    #[test]
    fn put_str_ignores_rows_off_screen() {
        let mut t = Terminal::new(5, 2);
        t.put_str(0, 9, "X", Ink::Amber);
        assert!((0..5).all(|c| t.get(c, 0).unwrap().ch == ' '));
    }

    #[test]
    fn frame_draws_corners() {
        let mut t = Terminal::new(10, 5);
        t.frame(1, 1, 4, 3, Ink::Dim);
        assert_eq!(t.get(1, 1).unwrap().ch, '+');
        assert_eq!(t.get(4, 3).unwrap().ch, '+');
        assert_eq!(t.get(1, 2).unwrap().ch, '|');
    }
}
