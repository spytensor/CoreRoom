use std::io::{IsTerminal, Write as _};

use crossterm::{style::Stylize, terminal};

use crate::output;

/// Frames of the standard braille spinner. ~10 frames at 100 ms gives
/// a familiar one-second rotation that matches `cargo`, `npm`, etc.
pub(super) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Tick interval for the spinner, in milliseconds. Below ~80 ms users
/// notice the redraws as flicker; above ~120 ms it looks frozen.
pub(super) const SPINNER_TICK_MS: u64 = 100;

/// Status line region that lives below the user's last input while we wait
/// for role activity. Today it owns one role slot; the type boundary is the
/// future contract for concurrent multi-role rendering.
///
/// Skips all output when stdout is not a TTY (`cr ... | tee log.txt`)
/// to keep redirected output free of ANSI escapes.
pub(super) struct StatusRegion {
    pub(super) slots: Vec<StatusSlot>,
    pub(super) is_painted: bool,
    pub(super) is_tty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatusSlot {
    pub(super) role: String,
    pub(super) frame: usize,
}

impl StatusRegion {
    pub(super) fn start(role: &str) -> Self {
        let mut region = Self {
            slots: vec![StatusSlot {
                role: role.to_owned(),
                frame: 0,
            }],
            is_painted: false,
            is_tty: std::io::stdout().is_terminal(),
        };
        region.repaint();
        region
    }

    fn paint(&mut self) {
        if !self.is_tty {
            return;
        }
        // \r returns cursor to col 0; \x1b[2K clears the whole line.
        // The role color is dropped on intentionally so the line is
        // unambiguously "status" and not confused with a RoleSpoke.
        let columns = terminal::size().map_or(80, |(cols, _)| usize::from(cols));
        print!("\r\x1b[2K{}", self.render_line_at_width(columns));
        let _ = std::io::stdout().flush();
        self.is_painted = true;
    }

    pub(super) fn advance(&mut self) {
        for slot in &mut self.slots {
            slot.frame = (slot.frame + 1) % SPINNER_FRAMES.len();
        }
        if self.is_painted {
            self.paint();
        }
    }

    pub(super) fn repaint(&mut self) {
        self.paint();
    }

    pub(super) fn clear(&mut self) {
        if !self.is_tty || !self.is_painted {
            self.is_painted = false;
            return;
        }
        print!("\r\x1b[2K");
        let _ = std::io::stdout().flush();
        self.is_painted = false;
    }

    pub(super) fn render_line_at_width(&self, width: usize) -> String {
        let slots = self
            .slots
            .iter()
            .map(|slot| {
                let frame = SPINNER_FRAMES[slot.frame % SPINNER_FRAMES.len()];
                format!("@{} {frame}", slot.role)
            })
            .collect::<Vec<_>>()
            .join("  ");
        let count = self.slots.len();
        let noun = if count == 1 { "role" } else { "roles" };
        let line =
            format!("│ {count} {noun} working · chat stream paused until they report · {slots}");
        output::truncate_visible(&line, width)
            .with(output::DIM)
            .to_string()
    }
}

impl Drop for StatusRegion {
    fn drop(&mut self) {
        // Defensive: never leave status text painted on the screen if a
        // panic or early return ate the explicit clear() call.
        self.clear();
    }
}
