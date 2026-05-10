use std::io::Write as _;

use anyhow::{Context, Result};
use crossterm::cursor::{MoveDown, MoveToColumn, MoveUp};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::terminal::{self, Clear, ClearType};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::output;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InputLine {
    Line(String),
    Eof,
    Interrupted,
}

pub(super) async fn read_tty_line() -> Result<InputLine> {
    tokio::task::spawn_blocking(read_tty_line_blocking)
        .await
        .context("joining tty input reader")?
}

fn read_tty_line_blocking() -> Result<InputLine> {
    let _raw_mode = RawModeGuard::enter()?;
    let columns = terminal::size().map_or(80, |(cols, _)| usize::from(cols));
    let mut editor = LineEditor::new(columns);
    let mut stdout = std::io::stdout();
    writeln!(stdout)?;
    editor.redraw(&mut stdout)?;

    loop {
        let Event::Key(key) = event::read().context("reading terminal input")? else {
            continue;
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            continue;
        }
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.finish(&mut stdout)?;
                writeln!(stdout, "^C")?;
                stdout.flush()?;
                return Ok(InputLine::Interrupted);
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) && editor.is_empty() =>
            {
                editor.finish(&mut stdout)?;
                return Ok(InputLine::Eof);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.clear();
                editor.redraw(&mut stdout)?;
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                editor.insert(ch);
                editor.redraw(&mut stdout)?;
            }
            KeyCode::Backspace => {
                if editor.backspace() {
                    editor.redraw(&mut stdout)?;
                }
            }
            KeyCode::Delete => {
                if editor.delete() {
                    editor.redraw(&mut stdout)?;
                }
            }
            KeyCode::Left => {
                if editor.move_left() {
                    editor.redraw(&mut stdout)?;
                }
            }
            KeyCode::Right => {
                if editor.move_right() {
                    editor.redraw(&mut stdout)?;
                }
            }
            KeyCode::Home => {
                if editor.move_home() {
                    editor.redraw(&mut stdout)?;
                }
            }
            KeyCode::End => {
                if editor.move_end() {
                    editor.redraw(&mut stdout)?;
                }
            }
            KeyCode::Enter => {
                let line = editor.input();
                editor.finish(&mut stdout)?;
                writeln!(stdout)?;
                stdout.flush()?;
                return Ok(InputLine::Line(line));
            }
            _ => {}
        }
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("enabling raw terminal input")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Debug)]
struct LineEditor {
    prompt: String,
    prompt_width: usize,
    buffer: Vec<char>,
    cursor: usize,
    painted_cursor_width: usize,
    columns: usize,
}

impl LineEditor {
    fn new(columns: usize) -> Self {
        Self {
            prompt: output::prompt_inline(),
            prompt_width: UnicodeWidthStr::width(output::prompt_plain()),
            buffer: Vec::new(),
            cursor: 0,
            painted_cursor_width: 0,
            columns: columns.max(1),
        }
    }

    fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    fn input(&self) -> String {
        self.buffer.iter().collect()
    }

    fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += 1;
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.buffer.remove(self.cursor);
        true
    }

    fn delete(&mut self) -> bool {
        if self.cursor >= self.buffer.len() {
            return false;
        }
        self.buffer.remove(self.cursor);
        true
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.buffer.len() {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        if self.cursor == self.buffer.len() {
            return false;
        }
        self.cursor = self.buffer.len();
        true
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    fn redraw(&mut self, stdout: &mut std::io::Stdout) -> Result<()> {
        self.move_from_width_to_prompt_start(stdout, self.painted_cursor_width)?;
        queue!(stdout, Clear(ClearType::FromCursorDown))?;
        write!(stdout, "{}{}", self.prompt, self.input())?;
        let cursor_width = self.cursor_width();
        self.move_from_line_end_to_width(stdout, cursor_width)?;
        self.painted_cursor_width = cursor_width;
        stdout.flush()?;
        Ok(())
    }

    fn finish(&mut self, stdout: &mut std::io::Stdout) -> Result<()> {
        self.move_from_width_to_prompt_start(stdout, self.painted_cursor_width)?;
        write!(stdout, "{}{}", self.prompt, self.input())?;
        let total_width = self.total_width();
        self.move_from_line_end_to_width(stdout, total_width)?;
        self.painted_cursor_width = total_width;
        stdout.flush()?;
        Ok(())
    }

    fn move_from_width_to_prompt_start(
        &self,
        stdout: &mut std::io::Stdout,
        width: usize,
    ) -> Result<()> {
        let (row, _) = self.visual_position(width);
        queue!(stdout, MoveToColumn(0))?;
        if row > 0 {
            queue!(stdout, MoveUp(saturating_u16(row)))?;
        }
        Ok(())
    }

    fn move_from_line_end_to_width(
        &self,
        stdout: &mut std::io::Stdout,
        width: usize,
    ) -> Result<()> {
        let (end_row, _) = self.visual_position(self.total_width());
        let (target_row, target_col) = self.visual_position(width);
        queue!(stdout, MoveToColumn(0))?;
        if end_row > 0 {
            queue!(stdout, MoveUp(saturating_u16(end_row)))?;
        }
        if target_row > 0 {
            queue!(stdout, MoveDown(saturating_u16(target_row)))?;
        }
        queue!(stdout, MoveToColumn(saturating_u16(target_col)))?;
        Ok(())
    }

    fn cursor_width(&self) -> usize {
        self.prompt_width + self.buffer_width_until(self.cursor)
    }

    fn total_width(&self) -> usize {
        self.prompt_width + self.buffer_width_until(self.buffer.len())
    }

    fn buffer_width_until(&self, end: usize) -> usize {
        self.buffer
            .iter()
            .take(end)
            .map(|ch| UnicodeWidthChar::width(*ch).unwrap_or(0))
            .sum()
    }

    fn visual_position(&self, width: usize) -> (usize, usize) {
        (width / self.columns, width % self.columns)
    }
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_counts_cjk_cells_for_backspace() {
        let mut editor = LineEditor::new(80);
        editor.insert('物');
        editor.insert('物');
        editor.insert('品');

        assert_eq!(editor.input(), "物物品");
        assert_eq!(editor.buffer_width_until(editor.cursor), 6);

        assert!(editor.backspace());
        assert_eq!(editor.input(), "物物");
        assert_eq!(editor.buffer_width_until(editor.cursor), 4);
    }

    #[test]
    fn editor_tracks_cursor_width_separately_from_buffer_end() {
        let mut editor = LineEditor::new(80);
        editor.insert('物');
        editor.insert('a');
        editor.insert('品');
        assert!(editor.move_left());

        assert_eq!(editor.input(), "物a品");
        assert_eq!(
            editor.cursor_width(),
            UnicodeWidthStr::width(output::prompt_plain()) + 3
        );
        assert_eq!(
            editor.total_width(),
            UnicodeWidthStr::width(output::prompt_plain()) + 5
        );
    }

    #[test]
    fn editor_maps_wrapped_columns_by_display_width() {
        let mut editor = LineEditor::new(10);
        editor.insert('a');
        editor.insert('物');

        assert_eq!(editor.visual_position(editor.cursor_width()), (1, 1));
        editor.insert('品');
        assert_eq!(editor.visual_position(editor.cursor_width()), (1, 3));
    }
}
