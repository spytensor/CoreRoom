//! Chat-stream design demo — throwaway visual prototype.
//!
//! This is a **design demo**, not production code. It implements the chat
//! stream visual described in ADR `docs/v0.10-chat-stream-vs-dashboard.md`
//! (issue #378) so reviewers can step through the four-state sub-agent
//! lifecycle (`Spawning → Working → Done → Reported`) at terminal fidelity
//! before the real widget (#381) lands.
//!
//! Scope:
//! - Renders a single scripted scenario with hard-coded ticks.
//! - Depends only on `ratatui`, `crossterm`, and `coreroom::tui_style` for
//!   identity colors. No kernel, no snapshot, no runtime plumbing.
//! - Pre-recorded scene table — `Space` advances one tick, `r` resets,
//!   `q` quits.
//!
//! Safe to delete once the production `WorkingCard` widget (#381) and the
//! footer narration line (#382) land on the live room. This file is owned
//! by the v0.10 redesign cycle (umbrella #377) and has no callers outside
//! the `cargo run --example chat-stream-demo` entry point.
//!
//! Run:
//! ```text
//! cargo run --example chat-stream-demo
//! ```

use std::io::{self, Write};
use std::time::Duration;

use coreroom::tui_style::{role_color, role_label_spans};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor::Show, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

const HOST_ROLE: &str = "host";

/// One scripted scene of the chat stream.
///
/// Each scene is a complete world-state — the renderer reads it directly,
/// so reviewers can scrub forward by hitting `Space` and the rendering
/// stays a pure function of `scene_index`.
#[derive(Debug, Clone)]
struct Scene {
    /// Chat rows in display order, top to bottom.
    rows: Vec<Row>,
    /// Footer narration line shown above the composer. `None` hides the
    /// line entirely (no empty row reserved — see ADR §"Locked vocabulary").
    footer: Option<String>,
    /// Top-bar status badge: `idle` or `working · N`.
    status_badge: StatusBadge,
}

#[derive(Debug, Clone)]
enum StatusBadge {
    Idle,
    Working(usize),
}

/// One row in the chat stream.
///
/// `Message` and `SpawningHint` are single-line; `WorkingCard` and
/// `DoneCollapsed` expand into multiple lines when rendered.
#[derive(Debug, Clone)]
enum Row {
    /// Plain chat message `HH:MM @role  body`.
    Message {
        timestamp: &'static str,
        role: &'static str,
        body: &'static str,
    },
    /// Continuation line for a multi-line message — indents under the
    /// message body without re-printing timestamp/role.
    MessageCont { body: &'static str },
    /// Inline `Spawning` hint emitted by the spawner before the working
    /// card materializes. Rendered as a dim continuation of the prior
    /// `@host` line.
    SpawningHint { role: &'static str },
    /// Live working card with title row, accumulated tool calls, and
    /// footer hotkey hint. Matches Frame B / Frame C in the ADR.
    WorkingCard {
        role: &'static str,
        title: &'static str,
        elapsed: &'static str,
        steps: Vec<Step>,
    },
    /// Collapsed `Done` marker: `@role ✓ done · {elapsed} · {N} steps · [e]xpand log`.
    DoneCollapsed {
        role: &'static str,
        elapsed: &'static str,
        steps: usize,
    },
}

#[derive(Debug, Clone, Copy)]
enum StepKind {
    /// Completed step — rendered with `✓`.
    Done,
    /// In-progress step — rendered with `∴`.
    InProgress,
}

#[derive(Debug, Clone)]
struct Step {
    kind: StepKind,
    text: &'static str,
}

fn main() -> io::Result<()> {
    let scenes = build_scenes();
    let mut idx: usize = 0;

    let mut guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| draw(f, &scenes[idx], idx, scenes.len()))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('r') => idx = 0,
                    KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Right
                        if idx + 1 < scenes.len() =>
                    {
                        idx += 1;
                    }
                    KeyCode::Left | KeyCode::Backspace => idx = idx.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }

    guard.leave()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scenario script
// ---------------------------------------------------------------------------

/// The scripted scenario from issue #379, expanded into discrete scenes.
///
/// The scenes were chosen to make every lifecycle transition observable:
/// scene 0 — idle baseline; scenes 1-2 — `@you` and `@host` exchange;
/// scene 3 — `Spawning` hints; scenes 4-7 — `Working` cards accumulate
/// tool-call lines; scene 8 — `@qa` reaches `Done` (collapsed line);
/// scene 9 — `@qa` posts its report (`Reported`); scene 10 — the footer
/// narration line updates to "2 roles still working · @security @backend".
///
/// This is intentionally one big literal table so the script reads top-to-
/// bottom in scene order; splitting it into per-scene helpers would hide
/// the only thing that matters in a demo binary — the sequence.
#[allow(clippy::too_many_lines)]
fn build_scenes() -> Vec<Scene> {
    let mut scenes = Vec::new();

    let you_task = Row::Message {
        timestamp: "17:02",
        role: "you",
        body: "ship the v0.10 audit",
    };
    let host_delegate = Row::Message {
        timestamp: "17:02",
        role: "host",
        body: "on it. delegating @security @backend @qa …",
    };

    // Scene 0 — idle baseline.
    scenes.push(Scene {
        rows: vec![],
        footer: None,
        status_badge: StatusBadge::Idle,
    });

    // Scene 1 — @you posts the task.
    scenes.push(Scene {
        rows: vec![you_task.clone()],
        footer: None,
        status_badge: StatusBadge::Idle,
    });

    // Scene 2 — @host posts the delegation message.
    scenes.push(Scene {
        rows: vec![you_task.clone(), host_delegate.clone()],
        footer: None,
        status_badge: StatusBadge::Idle,
    });

    // Scene 3 — @security spawns first (hint only), then becomes working.
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::SpawningHint { role: "security" },
        ],
        footer: Some("@security spawning".to_owned()),
        status_badge: StatusBadge::Working(0),
    });

    // Scene 4 — @security card appears; @backend hint added.
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::WorkingCard {
                role: "security",
                title: "audit README claims",
                elapsed: "8s",
                steps: vec![Step {
                    kind: StepKind::InProgress,
                    text: "reading README.md §2.4 security model",
                }],
            },
            Row::SpawningHint { role: "backend" },
        ],
        footer: Some("1 role still working · @security · @backend spawning".to_owned()),
        status_badge: StatusBadge::Working(1),
    });

    // Scene 5 — @security accumulates a tool call; @backend card appears;
    // @qa spawns.
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::WorkingCard {
                role: "security",
                title: "audit README claims",
                elapsed: "14s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "read README.md §2.4 security model",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "cross-checking claims against src/permissions/",
                    },
                ],
            },
            Row::WorkingCard {
                role: "backend",
                title: "verify technical claims",
                elapsed: "6s",
                steps: vec![Step {
                    kind: StepKind::InProgress,
                    text: "running cargo test --workspace",
                }],
            },
            Row::SpawningHint { role: "qa" },
        ],
        footer: Some("2 roles still working · @security @backend · @qa spawning".to_owned()),
        status_badge: StatusBadge::Working(2),
    });

    // Scene 6 — all three cards working; @qa has its first step.
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::WorkingCard {
                role: "security",
                title: "audit README claims",
                elapsed: "22s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "read README.md §2.4 security model",
                    },
                    Step {
                        kind: StepKind::Done,
                        text: "grep -r 'permission' src/permissions/",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "drafting findings note",
                    },
                ],
            },
            Row::WorkingCard {
                role: "backend",
                title: "verify technical claims",
                elapsed: "14s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "ran cargo test --workspace",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "counting passing-vs-failing tests in fixtures/",
                    },
                ],
            },
            Row::WorkingCard {
                role: "qa",
                title: "smoke-test the v0.9.16 build",
                elapsed: "4s",
                steps: vec![Step {
                    kind: StepKind::InProgress,
                    text: "launching cr in headless mode",
                }],
            },
        ],
        footer: Some("3 roles still working · @security @backend @qa".to_owned()),
        status_badge: StatusBadge::Working(3),
    });

    // Scene 7 — @qa adds a tool call and is about to complete.
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::WorkingCard {
                role: "security",
                title: "audit README claims",
                elapsed: "31s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "read README.md §2.4 security model",
                    },
                    Step {
                        kind: StepKind::Done,
                        text: "grep -r 'permission' src/permissions/",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "drafting findings note",
                    },
                ],
            },
            Row::WorkingCard {
                role: "backend",
                title: "verify technical claims",
                elapsed: "23s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "ran cargo test --workspace",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "counting passing-vs-failing tests in fixtures/",
                    },
                ],
            },
            Row::WorkingCard {
                role: "qa",
                title: "smoke-test the v0.9.16 build",
                elapsed: "12s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "launched cr in headless mode",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "running scripted dogfood scenario",
                    },
                ],
            },
        ],
        footer: Some("3 roles still working · @security @backend @qa".to_owned()),
        status_badge: StatusBadge::Working(3),
    });

    // Scene 8 — @qa reaches Done; its card collapses in place.
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::WorkingCard {
                role: "security",
                title: "audit README claims",
                elapsed: "38s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "read README.md §2.4 security model",
                    },
                    Step {
                        kind: StepKind::Done,
                        text: "grep -r 'permission' src/permissions/",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "drafting findings note",
                    },
                ],
            },
            Row::WorkingCard {
                role: "backend",
                title: "verify technical claims",
                elapsed: "30s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "ran cargo test --workspace",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "counting passing-vs-failing tests in fixtures/",
                    },
                ],
            },
            Row::DoneCollapsed {
                role: "qa",
                elapsed: "19s",
                steps: 2,
            },
        ],
        footer: Some("2 roles still working · @security @backend".to_owned()),
        status_badge: StatusBadge::Working(2),
    });

    // Scene 9 — @qa emits its report as a normal chat row; the collapsed
    // line stays as the header for the report (ADR §"Locked vocabulary").
    scenes.push(Scene {
        rows: vec![
            you_task.clone(),
            host_delegate.clone(),
            Row::WorkingCard {
                role: "security",
                title: "audit README claims",
                elapsed: "44s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "read README.md §2.4 security model",
                    },
                    Step {
                        kind: StepKind::Done,
                        text: "grep -r 'permission' src/permissions/",
                    },
                    Step {
                        kind: StepKind::Done,
                        text: "drafted findings note",
                    },
                ],
            },
            Row::WorkingCard {
                role: "backend",
                title: "verify technical claims",
                elapsed: "36s",
                steps: vec![
                    Step {
                        kind: StepKind::Done,
                        text: "ran cargo test --workspace",
                    },
                    Step {
                        kind: StepKind::InProgress,
                        text: "counting passing-vs-failing tests in fixtures/",
                    },
                ],
            },
            Row::DoneCollapsed {
                role: "qa",
                elapsed: "19s",
                steps: 2,
            },
            Row::Message {
                timestamp: "17:03",
                role: "qa",
                body: "headless smoke pass: 2/2 scenarios green; no panics, no zombies.",
            },
            Row::MessageCont {
                body: "log saved to /tmp/cr-smoke-2026-05-26.log.",
            },
        ],
        footer: Some("2 roles still working · @security @backend".to_owned()),
        status_badge: StatusBadge::Working(2),
    });

    scenes
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw(f: &mut Frame, scene: &Scene, idx: usize, total: usize) {
    let size = f.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top status bar
            Constraint::Min(8),    // room + rail
            Constraint::Length(1), // footer narration (always reserved, but
            // may render an empty line — the ADR says hidden, the demo
            // renders it conditionally inside the function)
            Constraint::Length(3), // composer
            Constraint::Length(1), // hint bar
        ])
        .split(size);

    draw_top_bar(f, layout[0], scene, idx, total);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(60), Constraint::Length(22)])
        .split(layout[1]);
    draw_room(f, body[0], scene);
    draw_rail(f, body[1]);

    draw_footer(f, layout[2], scene);
    draw_composer(f, layout[3]);
    draw_hint_bar(f, layout[4]);
}

fn draw_top_bar(f: &mut Frame, area: Rect, scene: &Scene, idx: usize, total: usize) {
    let badge = match scene.status_badge {
        StatusBadge::Idle => Span::styled(" idle", Style::default().fg(Color::DarkGray)),
        StatusBadge::Working(n) => Span::styled(
            format!(" working · {n}"),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ),
    };
    let left = format!(
        "CoreRoom v0.10.0-demo  ·  chat-stream-demo  ·  scene {} / {}",
        idx + 1,
        total
    );
    let mut spans = vec![Span::raw(left), Span::raw("  ")];
    spans.push(Span::raw("●"));
    spans.push(badge);
    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_room(f: &mut Frame, area: Rect, scene: &Scene) {
    let block = Block::default().borders(Borders::ALL).title("Room");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = render_rows(&scene.rows, inner.width);
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_rows(rows: &[Row], inner_width: u16) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for row in rows {
        match row {
            Row::Message {
                timestamp,
                role,
                body,
            } => {
                out.push(render_message(timestamp, role, body));
            }
            Row::MessageCont { body } => {
                // 17 chars = "HH:MM @role  " padded to align under message
                // body. We use a fixed 13-char indent because timestamps
                // are always HH:MM and roles fit within 7 chars in this
                // demo's vocabulary.
                out.push(Line::from(vec![
                    Span::raw("             "),
                    Span::raw((*body).to_owned()),
                ]));
            }
            Row::SpawningHint { role } => {
                out.push(render_spawning_hint(role));
            }
            Row::WorkingCard {
                role,
                title,
                elapsed,
                steps,
            } => {
                render_working_card(&mut out, role, title, elapsed, steps, inner_width);
            }
            Row::DoneCollapsed {
                role,
                elapsed,
                steps,
            } => {
                out.push(render_done_collapsed(role, elapsed, *steps));
            }
        }
    }
    out
}

fn render_message(timestamp: &str, role: &str, body: &str) -> Line<'static> {
    let color = role_color(role, HOST_ROLE);
    let mut spans = vec![
        Span::styled(timestamp.to_owned(), Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(
            format!("@{role}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(body.to_owned()),
    ];
    // Pad the role token to 7 chars so message bodies align across rows
    // (`@you`, `@host`, `@qa`, …).
    let role_token_width = format!("@{role}").chars().count();
    if role_token_width < 7 {
        let pad = " ".repeat(7 - role_token_width);
        // Insert before the body separator.
        spans.insert(4, Span::raw(pad));
    }
    Line::from(spans)
}

fn render_spawning_hint(role: &str) -> Line<'static> {
    let color = role_color(role, HOST_ROLE);
    Line::from(vec![
        Span::raw("             "),
        Span::styled(
            format!("(@{role} spawning)"),
            Style::default().fg(color).add_modifier(Modifier::DIM),
        ),
    ])
}

fn render_working_card(
    out: &mut Vec<Line<'static>>,
    role: &str,
    title: &str,
    elapsed: &str,
    steps: &[Step],
    inner_width: u16,
) {
    let color = role_color(role, HOST_ROLE);
    let indent = "             "; // align with message body column.
    let card_width = (inner_width as usize).saturating_sub(indent.len()).max(40);

    // Top border with inline title:
    //   ┌─ @role · title ── working · elapsed ──...─┐
    let title_spans = vec![Span::styled("┌─ ", Style::default().fg(Color::DarkGray))];
    let mut header = Vec::new();
    header.push(Span::raw(indent));
    header.extend(title_spans);
    header.extend(role_label_spans(role, HOST_ROLE));
    header.push(Span::styled(
        format!(" · {title} "),
        Style::default().fg(color),
    ));
    header.push(Span::styled("── ", Style::default().fg(Color::DarkGray)));
    header.push(Span::styled(
        "working",
        Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
    ));
    header.push(Span::styled(
        format!(" · {elapsed} "),
        Style::default().fg(Color::DarkGray),
    ));
    let visible_so_far: usize = header
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>()
        .saturating_sub(indent.len());
    let pad = card_width.saturating_sub(visible_so_far + 1);
    if pad > 0 {
        header.push(Span::styled(
            "─".repeat(pad),
            Style::default().fg(Color::DarkGray),
        ));
    }
    header.push(Span::styled("┐", Style::default().fg(Color::DarkGray)));
    out.push(Line::from(header));

    // Step rows.
    let mut done_count = 0_usize;
    for step in steps {
        let (marker, marker_style) = match step.kind {
            StepKind::Done => {
                done_count += 1;
                ("✓", Style::default().fg(Color::LightGreen))
            }
            StepKind::InProgress => ("∴", Style::default().fg(Color::LightYellow)),
        };
        let body_width = card_width.saturating_sub(4 + step.text.chars().count() + 1);
        let pad_str = " ".repeat(body_width);
        out.push(Line::from(vec![
            Span::raw(indent),
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            Span::styled(marker, marker_style),
            Span::raw(" "),
            Span::raw(step.text.to_owned()),
            Span::raw(pad_str),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
        ]));
    }

    // Bottom border:
    //   └─ N step(s) done · [e]xpand [i]nterrupt [f]ocus ─...─┘
    let footer_text = format!(
        " {done_count} step{} done · [e]xpand [i]nterrupt [f]ocus ",
        if done_count == 1 { "" } else { "s" }
    );
    let mut footer = vec![
        Span::raw(indent),
        Span::styled("└─", Style::default().fg(Color::DarkGray)),
        Span::styled(footer_text.clone(), Style::default().fg(Color::DarkGray)),
    ];
    let used: usize = footer
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>()
        .saturating_sub(indent.len());
    let pad = card_width.saturating_sub(used + 1);
    if pad > 0 {
        footer.push(Span::styled(
            "─".repeat(pad),
            Style::default().fg(Color::DarkGray),
        ));
    }
    footer.push(Span::styled("┘", Style::default().fg(Color::DarkGray)));
    out.push(Line::from(footer));
}

fn render_done_collapsed(role: &str, elapsed: &str, steps: usize) -> Line<'static> {
    let color = role_color(role, HOST_ROLE);
    Line::from(vec![
        Span::raw("             "),
        Span::styled(
            format!("@{role}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("✓", Style::default().fg(Color::LightGreen)),
        Span::raw(" "),
        Span::styled("done", Style::default().fg(Color::LightGreen)),
        Span::styled(
            format!(" · {elapsed} · {steps} steps · [e]xpand log"),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

fn draw_rail(f: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Status");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Slim Status strip per ADR Q5: only fields that outlive a single turn.
    let lines = vec![
        Line::from(Span::styled(
            "Work",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(" active: 0"),
        Line::from(" cards: 0"),
        Line::from(""),
        Line::from(Span::styled(
            "Blockers",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(" state: none"),
        Line::from(""),
        Line::from(Span::styled(
            "Evidence",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(" validation: clean"),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, area: Rect, scene: &Scene) {
    // ADR §"Locked vocabulary": when nothing is in flight, the line is
    // hidden — no empty row reserved. We still render into the reserved
    // row, but draw a blank line so the layout stays stable across scenes.
    let line = if let Some(text) = &scene.footer {
        Line::from(Span::styled(
            text.clone(),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::ITALIC),
        ))
    } else {
        Line::from("")
    };
    f.render_widget(Paragraph::new(line), area);
}

fn draw_composer(f: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Ask @host");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let line = Line::from(vec![
        Span::styled("cr > ", Style::default().fg(Color::DarkGray)),
        Span::raw(""),
    ]);
    f.render_widget(Paragraph::new(line), inner);
    // Place the caret right after `cr > ` so the demo shows the cursor in
    // the expected position. This is the v0.9.16 #374 pattern — we do not
    // standalone-Hide the cursor; ratatui handles visibility per frame
    // based on whether `set_cursor_position` is called.
    let cursor_x = inner.x + 5;
    let cursor_y = inner.y;
    f.set_cursor_position((cursor_x, cursor_y));
}

fn draw_hint_bar(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" space ", Style::default().fg(Color::DarkGray)),
        Span::raw("advance · "),
        Span::styled("r ", Style::default().fg(Color::DarkGray)),
        Span::raw("reset · "),
        Span::styled("q ", Style::default().fg(Color::DarkGray)),
        Span::raw("quit"),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------
// Terminal lifecycle (mirrors `RoomTerminalGuard` in console_room_runtime).
// ---------------------------------------------------------------------------

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut out = io::stdout();
        // We deliberately do not emit `Hide` here — see the v0.9.16 #374
        // notes in `src/console_room_runtime.rs`: ratatui's per-frame
        // `set_cursor_position` already manages cursor visibility, and a
        // standalone `Hide` would race that and leave the composer caret
        // missing on alt-screen entry.
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;
        out.flush()?;
        Ok(Self { active: true })
    }

    fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let mut out = io::stdout();
        execute!(
            out,
            DisableMouseCapture,
            DisableBracketedPaste,
            Show,
            LeaveAlternateScreen,
        )?;
        terminal::disable_raw_mode()?;
        out.flush()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}
