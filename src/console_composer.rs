//! Renderer-independent composer state for the future unified room.
//!
//! This module intentionally contains no terminal I/O, no ratatui types, and no
//! project-state facts. It models the input surface that a full-screen room can
//! render while preserving the current REPL's role/slash completion semantics.

/// Static slash-command metadata used by the composer completion model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerCommandSpec {
    /// Command name without the leading slash.
    pub name: String,
    /// One-line description for menus.
    pub description: String,
    /// Whether accepting the command should append a trailing argument space.
    pub takes_args: bool,
}

impl ComposerCommandSpec {
    /// Create a command spec.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>, takes_args: bool) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            takes_args,
        }
    }
}

/// Composer submission lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerSubmissionState {
    /// The composer is editable.
    Idle,
    /// The composer has handed a prompt to the room runtime.
    Submitting,
    /// The room is waiting on permission or another blocking prompt.
    Blocked,
}

/// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerCandidate {
    /// Replacement text, including `@` or `/`.
    pub label: String,
    /// Secondary menu text.
    pub description: String,
    /// Whether this candidate is selected by the completion cursor.
    pub selected: bool,
}

/// Renderer-facing composer view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerViewModel {
    /// Current input buffer.
    pub input: String,
    /// Cursor position in char indices.
    pub cursor: usize,
    /// True when the buffer contains at least one newline.
    pub multiline: bool,
    /// Ghost suffix that may be rendered after the cursor.
    pub ghost_suffix: Option<String>,
    /// Dropdown/menu candidates.
    pub candidates: Vec<ComposerCandidate>,
    /// Current submission lifecycle.
    pub submission_state: ComposerSubmissionState,
    /// Short prompt hint such as `type a task - @role - /help`.
    pub prompt_hint: String,
}

/// Result of accepting the active completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerAcceptOutcome {
    /// Nothing was active.
    None,
    /// Completion produced a complete command, so Enter may submit.
    Complete,
    /// Completion inserted a role/command prefix that expects more text.
    ExpectsMore,
}

/// Result of pressing Enter in submit mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerSubmitOutcome {
    /// Submitted prompt text.
    Submitted(String),
    /// A completion was accepted, but the composer still expects task text.
    CompletionAccepted,
    /// The composer is empty or blocked.
    Noop,
}

/// Pure composer state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerState {
    buffer: Vec<char>,
    cursor: usize,
    roles: Vec<String>,
    commands: Vec<ComposerCommandSpec>,
    completion_index: usize,
    completion_anchor: Option<String>,
    menu_dismissed: bool,
    submission_state: ComposerSubmissionState,
    prompt_hint: String,
}

impl ComposerState {
    /// Create a composer state with stable role and command completion order.
    #[must_use]
    pub fn new(
        mut roles: Vec<String>,
        mut commands: Vec<ComposerCommandSpec>,
        prompt_hint: impl Into<String>,
    ) -> Self {
        roles.sort_by_key(|name| name.to_ascii_lowercase());
        commands.sort_by_key(|cmd| cmd.name.to_ascii_lowercase());
        Self {
            buffer: Vec::new(),
            cursor: 0,
            roles,
            commands,
            completion_index: 0,
            completion_anchor: None,
            menu_dismissed: false,
            submission_state: ComposerSubmissionState::Idle,
            prompt_hint: prompt_hint.into(),
        }
    }

    /// Current input string.
    #[must_use]
    pub fn input(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Cursor position in char indices.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Current submission state.
    #[must_use]
    pub const fn submission_state(&self) -> ComposerSubmissionState {
        self.submission_state
    }

    /// Set submission state from the room runtime.
    pub fn set_submission_state(&mut self, state: ComposerSubmissionState) {
        self.submission_state = state;
    }

    /// Renderer-facing view model.
    #[must_use]
    pub fn view_model(&self) -> ComposerViewModel {
        ComposerViewModel {
            input: self.input(),
            cursor: self.cursor,
            multiline: self.buffer.contains(&'\n'),
            ghost_suffix: self.ghost_suffix(),
            candidates: self.candidates(),
            submission_state: self.submission_state,
            prompt_hint: self.prompt_hint.clone(),
        }
    }

    /// Insert one character at the cursor.
    pub fn insert_char(&mut self, ch: char) {
        if is_allowed_input_char(ch) {
            self.buffer.insert(self.cursor, ch);
            self.cursor += 1;
            self.invalidate_completion();
        }
    }

    /// Insert a literal newline without submitting.
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Insert pasted text as one logical buffer update.
    pub fn paste_str(&mut self, text: &str) {
        let chars = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .filter(|ch| is_allowed_input_char(*ch))
            .collect::<Vec<_>>();
        if chars.is_empty() {
            return;
        }
        let cursor = self.cursor;
        self.buffer.splice(cursor..cursor, chars.iter().copied());
        self.cursor = cursor + chars.len();
        self.invalidate_completion();
    }

    /// Delete one character left of the cursor.
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.buffer.remove(self.cursor);
        self.invalidate_completion();
        true
    }

    /// Delete one character at the cursor.
    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.buffer.len() {
            return false;
        }
        self.buffer.remove(self.cursor);
        self.invalidate_completion();
        true
    }

    /// Move cursor one char left.
    pub fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.invalidate_completion();
        true
    }

    /// Move cursor one char right.
    pub fn move_right(&mut self) -> bool {
        if self.cursor >= self.buffer.len() {
            return false;
        }
        self.cursor += 1;
        self.invalidate_completion();
        true
    }

    /// Move cursor to start.
    pub fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        self.invalidate_completion();
        true
    }

    /// Move cursor to end.
    pub fn move_end(&mut self) -> bool {
        if self.cursor == self.buffer.len() {
            return false;
        }
        self.cursor = self.buffer.len();
        self.invalidate_completion();
        true
    }

    /// Clear the input buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.invalidate_completion();
    }

    /// Hide the menu for the current completion anchor.
    pub fn dismiss_completion(&mut self) -> bool {
        let was_active = self.completion_anchor.is_some()
            || self.ghost_suffix().is_some()
            || !self.candidates().is_empty();
        self.completion_anchor = None;
        self.completion_index = 0;
        self.menu_dismissed = true;
        was_active
    }

    /// Cycle completion forward.
    pub fn cycle_completion(&mut self) -> bool {
        self.cycle_completion_step(1)
    }

    /// Cycle completion backward.
    pub fn cycle_completion_back(&mut self) -> bool {
        self.cycle_completion_step(-1)
    }

    /// Accept active role or slash completion.
    pub fn accept_completion(&mut self) -> ComposerAcceptOutcome {
        if let Some((token_start, prefix)) = self.at_token() {
            let matches = self.matching_roles(&prefix);
            if matches.is_empty() {
                return ComposerAcceptOutcome::None;
            }
            let pick = matches[self.completion_index % matches.len()].to_owned();
            self.buffer.drain(token_start..self.cursor);
            let mut insert_at = token_start;
            for ch in std::iter::once('@')
                .chain(pick.chars())
                .chain(std::iter::once(' '))
            {
                self.buffer.insert(insert_at, ch);
                insert_at += 1;
            }
            self.cursor = insert_at;
            self.invalidate_completion();
            return ComposerAcceptOutcome::ExpectsMore;
        }
        if let Some(prefix) = self.slash_token() {
            let matches = self.matching_commands(&prefix);
            if matches.is_empty() {
                return ComposerAcceptOutcome::None;
            }
            let pick = matches[self.completion_index % matches.len()].clone();
            self.buffer.drain(0..self.cursor);
            let mut insert_at = 0usize;
            for ch in std::iter::once('/').chain(pick.name.chars()) {
                self.buffer.insert(insert_at, ch);
                insert_at += 1;
            }
            if pick.takes_args {
                self.buffer.insert(insert_at, ' ');
                insert_at += 1;
            }
            self.cursor = insert_at;
            self.invalidate_completion();
            return if pick.takes_args {
                ComposerAcceptOutcome::ExpectsMore
            } else {
                ComposerAcceptOutcome::Complete
            };
        }
        ComposerAcceptOutcome::None
    }

    /// Submit the current buffer, accepting a visible completion first.
    pub fn submit(&mut self) -> ComposerSubmitOutcome {
        if self.submission_state != ComposerSubmissionState::Idle {
            return ComposerSubmitOutcome::Noop;
        }
        if self.ghost_suffix().is_some()
            && self.accept_completion() == ComposerAcceptOutcome::ExpectsMore
        {
            return ComposerSubmitOutcome::CompletionAccepted;
        }
        let input = self.input();
        if input.trim().is_empty() {
            return ComposerSubmitOutcome::Noop;
        }
        self.clear();
        self.submission_state = ComposerSubmissionState::Submitting;
        ComposerSubmitOutcome::Submitted(input)
    }

    fn invalidate_completion(&mut self) {
        self.completion_anchor = None;
        self.completion_index = 0;
        self.menu_dismissed = false;
    }

    fn at_token(&self) -> Option<(usize, String)> {
        if self.cursor != self.buffer.len() {
            return None;
        }
        let mut idx = self.cursor;
        while idx > 0 {
            let ch = self.buffer[idx - 1];
            if ch == '@' {
                let before_ok = idx == 1 || self.buffer[idx - 2].is_whitespace();
                if !before_ok {
                    return None;
                }
                let prefix = self.buffer[idx..self.cursor].iter().collect();
                return Some((idx - 1, prefix));
            }
            if ch.is_whitespace() {
                return None;
            }
            idx -= 1;
        }
        None
    }

    fn slash_token(&self) -> Option<String> {
        if self.cursor != self.buffer.len() || self.buffer.first() != Some(&'/') {
            return None;
        }
        let after_slash = self.buffer.iter().skip(1).collect::<String>();
        (!after_slash.chars().any(char::is_whitespace)).then_some(after_slash)
    }

    fn matching_roles(&self, prefix: &str) -> Vec<&str> {
        let needle = prefix.to_ascii_lowercase();
        self.roles
            .iter()
            .filter(|name| name.to_ascii_lowercase().starts_with(&needle))
            .map(String::as_str)
            .collect()
    }

    fn matching_commands(&self, prefix: &str) -> Vec<&ComposerCommandSpec> {
        let needle = prefix.to_ascii_lowercase();
        self.commands
            .iter()
            .filter(|cmd| cmd.name.to_ascii_lowercase().starts_with(&needle))
            .collect()
    }

    fn ghost_suffix(&self) -> Option<String> {
        if let Some((_, prefix)) = self.at_token() {
            let matches = self.matching_roles(&prefix);
            if matches.is_empty() {
                return None;
            }
            let pick = matches[self.completion_index % matches.len()];
            if pick.eq_ignore_ascii_case(&prefix) {
                return None;
            }
            return pick.get(prefix.len()..).map(ToOwned::to_owned);
        }
        if let Some(prefix) = self.slash_token() {
            let matches = self.matching_commands(&prefix);
            if matches.is_empty() {
                return None;
            }
            let pick = matches[self.completion_index % matches.len()];
            if pick.name.eq_ignore_ascii_case(&prefix) {
                return None;
            }
            return pick.name.get(prefix.len()..).map(ToOwned::to_owned);
        }
        None
    }

    fn candidates(&self) -> Vec<ComposerCandidate> {
        if self.menu_dismissed {
            return Vec::new();
        }
        if let Some((_, prefix)) = self.at_token() {
            let matches = self.matching_roles(&prefix);
            if matches.len() < 2 {
                return Vec::new();
            }
            return matches
                .iter()
                .enumerate()
                .map(|(index, role)| ComposerCandidate {
                    label: format!("@{role}"),
                    description: String::new(),
                    selected: index == self.completion_index % matches.len(),
                })
                .collect();
        }
        if let Some(prefix) = self.slash_token() {
            let matches = self.matching_commands(&prefix);
            if matches.len() < 2 {
                return Vec::new();
            }
            return matches
                .iter()
                .enumerate()
                .map(|(index, cmd)| ComposerCandidate {
                    label: format!("/{}", cmd.name),
                    description: cmd.description.clone(),
                    selected: index == self.completion_index % matches.len(),
                })
                .collect();
        }
        Vec::new()
    }

    fn cycle_completion_step(&mut self, delta: i32) -> bool {
        let (prefix_key, match_count) = if let Some((_, prefix)) = self.at_token() {
            (
                format!("@{}", prefix.to_ascii_lowercase()),
                self.matching_roles(&prefix).len(),
            )
        } else if let Some(prefix) = self.slash_token() {
            (
                format!("/{}", prefix.to_ascii_lowercase()),
                self.matching_commands(&prefix).len(),
            )
        } else {
            return false;
        };
        if match_count == 0 {
            return false;
        }
        let count_i32 = i32::try_from(match_count).unwrap_or(i32::MAX);
        if self.completion_anchor.as_deref() == Some(prefix_key.as_str()) {
            let next =
                (i32::try_from(self.completion_index).unwrap_or(0) + delta).rem_euclid(count_i32);
            self.completion_index = usize::try_from(next).unwrap_or(0);
        } else {
            self.completion_anchor = Some(prefix_key);
            self.completion_index = usize::try_from(delta.rem_euclid(count_i32)).unwrap_or(0);
        }
        true
    }
}

fn is_allowed_input_char(ch: char) -> bool {
    !ch.is_control() || matches!(ch, '\n' | '\t')
}
