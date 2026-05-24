//! Console navigation state for the v0.9 full-screen shell.
//!
//! This module is intentionally renderer-independent. It owns tab switching,
//! selection, detail state, filtering/search, and responsive summaries while
//! consuming the view models built from `CoreRoomSnapshot`.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::console_layout::{compute_console_layout, ConsoleBreakpoint};
use crate::console_snapshot::{CoreRoomSnapshot, StatusState};
use crate::console_views::{
    build_crep_logs_view, build_evidence_view, build_gates_view, build_roles_view,
    build_sources_view, build_workorder_xray_view, build_workorders_view, CrepLogFilter,
};
use crate::crep::CrepEvent;

/// Console tabs in navigation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsoleView {
    /// Project overview and public conversation.
    Overview,
    /// Role lane table.
    Roles,
    /// WorkOrder table.
    WorkOrders,
    /// Gate progression table.
    Gates,
    /// Evidence closure table.
    Evidence,
    /// Source health table.
    Sources,
    /// CREP/event log table.
    Logs,
    /// WorkOrder Xray chain.
    Xray,
}

impl ConsoleView {
    /// Stable lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Roles => "roles",
            Self::WorkOrders => "workorders",
            Self::Gates => "gates",
            Self::Evidence => "evidence",
            Self::Sources => "sources",
            Self::Logs => "logs",
            Self::Xray => "xray",
        }
    }
}

/// All console views in tab order.
pub const CONSOLE_VIEWS: [ConsoleView; 8] = [
    ConsoleView::Overview,
    ConsoleView::Roles,
    ConsoleView::WorkOrders,
    ConsoleView::Gates,
    ConsoleView::Evidence,
    ConsoleView::Sources,
    ConsoleView::Logs,
    ConsoleView::Xray,
];

/// One visible row in the active view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleVisibleRow {
    /// Stable row id.
    pub id: String,
    /// Primary label.
    pub primary: String,
    /// Secondary detail.
    pub secondary: String,
    /// Compact status for styling.
    pub status: StatusState,
    /// Source/citation/freshness label.
    pub source: Option<String>,
}

/// Responsive navigation summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleResponsiveState {
    /// Input terminal width.
    pub width: u16,
    /// Breakpoint selected by layout model.
    pub breakpoint: ConsoleBreakpoint,
    /// Whether the right rail is visible.
    pub right_rail_visible: bool,
    /// Width reserved for the public conversation.
    pub conversation_columns: u16,
}

/// Mutable navigation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleNavigator {
    /// Active view.
    pub active_view: ConsoleView,
    /// Selected row in the active view.
    pub selected: usize,
    /// Whether the detail pane is open.
    pub detail_open: bool,
    /// Simple text filter applied to visible rows.
    pub filter: Option<String>,
    /// Search text applied to visible rows/logs.
    pub search: Option<String>,
}

impl Default for ConsoleNavigator {
    fn default() -> Self {
        Self {
            active_view: ConsoleView::Overview,
            selected: 0,
            detail_open: false,
            filter: None,
            search: None,
        }
    }
}

impl ConsoleNavigator {
    /// Advance to the next view.
    pub fn next_view(&mut self) {
        let current = view_index(self.active_view);
        self.active_view = CONSOLE_VIEWS[(current + 1) % CONSOLE_VIEWS.len()];
        self.selected = 0;
        self.detail_open = false;
    }

    /// Move to the previous view.
    pub fn previous_view(&mut self) {
        let current = view_index(self.active_view);
        self.active_view = CONSOLE_VIEWS[(current + CONSOLE_VIEWS.len() - 1) % CONSOLE_VIEWS.len()];
        self.selected = 0;
        self.detail_open = false;
    }

    /// Move selection down, saturating at the last row.
    pub fn move_down(&mut self, row_count: usize) {
        if row_count > 0 {
            self.selected = (self.selected + 1).min(row_count - 1);
        }
    }

    /// Move selection up, saturating at the first row.
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Open the detail pane.
    pub fn open_detail(&mut self) {
        self.detail_open = true;
    }

    /// Close the detail pane.
    pub fn close_detail(&mut self) {
        self.detail_open = false;
    }

    /// Apply a keyboard event to this navigation state.
    pub fn apply_key(&mut self, code: KeyCode, modifiers: KeyModifiers, row_count: usize) {
        match (code, modifiers) {
            (KeyCode::Tab, _) => self.next_view(),
            (KeyCode::BackTab, _) => self.previous_view(),
            (KeyCode::Char('j') | KeyCode::Down, _) => self.move_down(row_count),
            (KeyCode::Char('k') | KeyCode::Up, _) => self.move_up(),
            (KeyCode::Enter | KeyCode::Right, _) => self.open_detail(),
            (KeyCode::Esc | KeyCode::Left, _) => self.close_detail(),
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => self.filter = None,
            (KeyCode::Char('/'), _) => self.search = Some(String::new()),
            _ => {}
        }
        self.clamp_selection(row_count);
    }

    /// Set or clear a text filter.
    pub fn set_filter(&mut self, filter: Option<String>, row_count: usize) {
        self.filter = normalize_query(filter);
        self.clamp_selection(row_count);
    }

    /// Set or clear search text.
    pub fn set_search(&mut self, search: Option<String>, row_count: usize) {
        self.search = normalize_query(search);
        self.clamp_selection(row_count);
    }

    fn clamp_selection(&mut self, row_count: usize) {
        if row_count == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(row_count - 1);
        }
    }
}

/// Build visible rows for the active view.
#[must_use]
pub fn visible_rows(
    snapshot: &CoreRoomSnapshot,
    crep_events: &[CrepEvent],
    navigator: &ConsoleNavigator,
) -> Vec<ConsoleVisibleRow> {
    let rows = raw_rows(snapshot, crep_events, navigator);
    rows.into_iter()
        .filter(|row| matches_query(row, navigator.filter.as_deref()))
        .filter(|row| matches_query(row, navigator.search.as_deref()))
        .collect()
}

/// Compute responsive navigation state for a terminal width.
#[must_use]
pub fn responsive_state(snapshot: &CoreRoomSnapshot, width: u16) -> ConsoleResponsiveState {
    let layout = compute_console_layout(snapshot, width);
    ConsoleResponsiveState {
        width,
        breakpoint: layout.breakpoint,
        right_rail_visible: layout.right_rail.is_some(),
        conversation_columns: layout.conversation_columns,
    }
}

fn raw_rows(
    snapshot: &CoreRoomSnapshot,
    crep_events: &[CrepEvent],
    navigator: &ConsoleNavigator,
) -> Vec<ConsoleVisibleRow> {
    match navigator.active_view {
        ConsoleView::Overview => overview_rows(snapshot),
        ConsoleView::Roles => role_rows(snapshot),
        ConsoleView::WorkOrders => work_rows(snapshot),
        ConsoleView::Gates => gate_rows(snapshot),
        ConsoleView::Evidence => evidence_rows(snapshot),
        ConsoleView::Sources => source_rows(snapshot),
        ConsoleView::Logs => log_rows(crep_events),
        ConsoleView::Xray => xray_rows(snapshot, navigator.selected),
    }
}

fn overview_rows(snapshot: &CoreRoomSnapshot) -> Vec<ConsoleVisibleRow> {
    vec![
        row(
            "project",
            snapshot.project.project.clone(),
            snapshot.project.repository.clone(),
            StatusState::Ok,
            Some(format!("tracker:#{}", snapshot.project.tracker_issue)),
        ),
        row(
            "phase",
            snapshot.project.active_phase.clone(),
            snapshot.project.branch.clone(),
            StatusState::Ok,
            snapshot.project.head_sha.clone(),
        ),
    ]
}

fn role_rows(snapshot: &CoreRoomSnapshot) -> Vec<ConsoleVisibleRow> {
    build_roles_view(snapshot)
        .into_iter()
        .map(|role| {
            row(
                format!("role:{}", role.role),
                format!("@{}", role.role),
                format!(
                    "{} / {:?} / {}",
                    role.engine,
                    role.state,
                    role.current_work_order.unwrap_or_else(|| "idle".to_owned())
                ),
                role.status,
                role.next_action,
            )
        })
        .collect()
}

fn work_rows(snapshot: &CoreRoomSnapshot) -> Vec<ConsoleVisibleRow> {
    build_workorders_view(snapshot)
        .into_iter()
        .map(|work| {
            row(
                work.id.clone(),
                work.id,
                format!(
                    "{} / issue {:?} / PR {:?}",
                    work.title, work.github_issue, work.pull_request
                ),
                work.detail
                    .blocker
                    .as_ref()
                    .map_or(work.ci_state, |_| StatusState::Blocking),
                work.citations.first().cloned(),
            )
        })
        .collect()
}

fn gate_rows(snapshot: &CoreRoomSnapshot) -> Vec<ConsoleVisibleRow> {
    build_gates_view(snapshot)
        .into_iter()
        .map(|gate| {
            row(
                format!("gate:{}", gate.work_order),
                gate.work_order,
                format!(
                    "{} / missing reviews: {} / signoff: {}",
                    gate.current_phase,
                    gate.missing_reviews.len(),
                    gate.signoff_ready
                ),
                gate.status,
                gate.detail.next_action,
            )
        })
        .collect()
}

fn evidence_rows(snapshot: &CoreRoomSnapshot) -> Vec<ConsoleVisibleRow> {
    build_evidence_view(snapshot)
        .into_iter()
        .map(|evidence| {
            row(
                format!("evidence:{}", evidence.work_order),
                evidence.work_order,
                format!(
                    "{:?} / missing: {} / tracker: {}",
                    evidence.status,
                    evidence.missing_fields.len(),
                    evidence.tracker_updated
                ),
                evidence.health,
                evidence.rollback,
            )
        })
        .collect()
}

fn source_rows(snapshot: &CoreRoomSnapshot) -> Vec<ConsoleVisibleRow> {
    build_sources_view(snapshot)
        .into_iter()
        .map(|source| {
            row(
                format!("source:{}", source.source_id),
                source.source_id,
                format!(
                    "{:?} / {} / roles: {}",
                    source.status,
                    source.trust_level,
                    source.visible_roles.join(",")
                ),
                source.health,
                source.pin,
            )
        })
        .collect()
}

fn log_rows(crep_events: &[CrepEvent]) -> Vec<ConsoleVisibleRow> {
    build_crep_logs_view(crep_events, &CrepLogFilter::default())
        .into_iter()
        .map(|log| {
            row(
                format!("log:{}", log.event_type),
                log.event_type,
                log.summary,
                log.status,
                log.thread_id,
            )
        })
        .collect()
}

fn xray_rows(snapshot: &CoreRoomSnapshot, selected: usize) -> Vec<ConsoleVisibleRow> {
    let Some(work) = snapshot
        .work
        .get(selected)
        .or_else(|| snapshot.work.first())
    else {
        return Vec::new();
    };
    let Some(xray) = build_workorder_xray_view(snapshot, &work.id) else {
        return Vec::new();
    };
    xray.steps
        .into_iter()
        .map(|step| {
            row(
                step.name,
                step.value,
                step.freshness,
                step.status,
                step.citations.first().cloned(),
            )
        })
        .collect()
}

fn row(
    id: impl Into<String>,
    primary: impl Into<String>,
    secondary: impl Into<String>,
    status: StatusState,
    source: Option<String>,
) -> ConsoleVisibleRow {
    ConsoleVisibleRow {
        id: id.into(),
        primary: primary.into(),
        secondary: secondary.into(),
        status,
        source,
    }
}

fn matches_query(row: &ConsoleVisibleRow, query: Option<&str>) -> bool {
    let Some(query) = query else {
        return true;
    };
    let query = query.to_ascii_lowercase();
    row.id.to_ascii_lowercase().contains(&query)
        || row.primary.to_ascii_lowercase().contains(&query)
        || row.secondary.to_ascii_lowercase().contains(&query)
        || row
            .source
            .as_ref()
            .is_some_and(|source| source.to_ascii_lowercase().contains(&query))
}

fn normalize_query(query: Option<String>) -> Option<String> {
    query
        .map(|query| query.trim().to_owned())
        .filter(|query| !query.is_empty())
}

fn view_index(view: ConsoleView) -> usize {
    CONSOLE_VIEWS
        .iter()
        .position(|candidate| *candidate == view)
        .expect("view exists")
}
