//! Small immediate-mode widgets drawn on the terminal grid.

use crate::terminal::{Ink, Terminal};
use macroquad::prelude::KeyCode;

/// What a key did to a screen: keep showing it, or go back to the colony.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
}

/// What a key did to a [`ListPicker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Moved, or ignored: keep showing the list.
    Pending,
    Chosen(usize),
    Back,
}

/// A cursor over a list the caller owns, so the list can change between frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListPicker {
    cursor: usize,
}

impl ListPicker {
    pub fn at(cursor: usize) -> Self {
        Self { cursor }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Keep the cursor on an item after the list shrinks.
    pub fn clamp(&mut self, len: usize) {
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }

    /// Up and Down move and wrap around, Enter picks, Esc or Backspace backs out.
    pub fn handle(&mut self, key: KeyCode, len: usize) -> Pick {
        match key {
            KeyCode::Up if len > 0 => self.cursor = (self.cursor + len - 1) % len,
            KeyCode::Down if len > 0 => self.cursor = (self.cursor + 1) % len,
            KeyCode::Enter | KeyCode::KpEnter if self.cursor < len => {
                return Pick::Chosen(self.cursor)
            }
            KeyCode::Escape | KeyCode::Backspace => return Pick::Back,
            _ => {}
        }
        Pick::Pending
    }

    /// One item per row from (col, row), scrolled so the cursor stays in view.
    pub fn draw(&self, t: &mut Terminal, col: usize, row: usize, height: usize, items: &[String]) {
        let start = (self.cursor + 1).saturating_sub(height);
        for (i, item) in items.iter().enumerate().skip(start).take(height) {
            let (mark, ink) = if i == self.cursor {
                ("> ", Ink::Bright)
            } else {
                ("  ", Ink::Amber)
            };
            t.put_str(col, row + i - start, &format!("{mark}{item}"), ink);
        }
    }
}

/// Break text into lines of at most `width` characters at spaces.
/// A single word longer than the width gets a line of its own and is clipped when drawn.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let len = line.chars().count();
        if len > 0 && len + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_wraps_around_and_picks() {
        let mut p = ListPicker::default();
        assert_eq!(p.handle(KeyCode::Up, 3), Pick::Pending);
        assert_eq!(p.cursor(), 2);
        p.handle(KeyCode::Down, 3);
        assert_eq!(p.handle(KeyCode::Enter, 3), Pick::Chosen(0));
        assert_eq!(p.handle(KeyCode::Escape, 3), Pick::Back);
    }

    #[test]
    fn an_empty_list_picks_nothing() {
        let mut p = ListPicker::default();
        assert_eq!(p.handle(KeyCode::Down, 0), Pick::Pending);
        assert_eq!(p.handle(KeyCode::Enter, 0), Pick::Pending);
    }

    #[test]
    fn picker_scrolls_to_keep_the_cursor_in_view() {
        let items: Vec<String> = (0..5).map(|i| format!("item {i}")).collect();
        let mut t = Terminal::new(20, 3);
        ListPicker::at(4).draw(&mut t, 0, 0, 2, &items);
        assert_eq!(t.get(7, 0).unwrap().ch, '3');
        let last = t.get(7, 1).unwrap();
        assert_eq!((last.ch, last.ink), ('4', Ink::Bright));
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        assert_eq!(
            wrap("IF grid.power < 25% THEN shutdown smelter_01", 20),
            vec!["IF grid.power < 25%", "THEN shutdown", "smelter_01"]
        );
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("toolongword x", 4), vec!["toolongword", "x"]);
    }
}
