use std::path::Path;

use anyhow::Result;

use crate::bus::MessageBus;
use crate::config::{Config, CODEROOM_DIR};
use crate::crep::CrepEvent;
use crate::output;
use crate::wal;
use crate::work;

use super::render::render_event;

/// Filters applied by `cr show` while replaying `.coderoom/messages.jsonl`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShowOptions {
    /// Role name to render. Stored without a leading `@`.
    pub role: Option<String>,
    /// Skip the log entirely if its filesystem mtime is older than this date.
    ///
    /// CREP v0.1 events do not carry per-event timestamps, so this mirrors
    /// `cr cost --since` until timestamped events land.
    pub since: Option<chrono::NaiveDate>,
    /// Render only the last N matching events.
    pub tail: Option<usize>,
    /// Print turns whose `TurnIntent` has no matching `TurnCommit`
    /// (amendment A-012). When set, all other rendering is suppressed
    /// and the output is a compact list of orphans.
    pub orphans: bool,
}

/// Replay events in `.coderoom/messages.jsonl` through the same renderer
/// the live REPL uses. Used by `cr show`.
pub async fn show_log(project_root: &Path, options: &ShowOptions) -> Result<()> {
    let coderoom_dir = project_root.join(CODEROOM_DIR);
    let log_path = coderoom_dir.join("messages.jsonl");
    if !log_path.is_file() {
        println!("(no messages — has `cr start` ever run in this project?)");
        return Ok(());
    }
    if let Some(since) = options.since {
        let modified = tokio::fs::metadata(&log_path).await?.modified()?;
        let modified: chrono::DateTime<chrono::Local> = modified.into();
        if modified.date_naive() < since {
            println!("(message log is older than {since})");
            return Ok(());
        }
    }
    // Loading config gives us the host role for stable lavender rendering.
    // If the config can't load (e.g. malformed), fall back to the default
    // host name — the replay still renders, lavender just won't pin.
    let host_role =
        Config::load(project_root).map_or_else(|_| "host".to_owned(), |cfg| cfg.host_role);
    let replay = MessageBus::replay(&log_path).await?;
    if replay.skipped_malformed > 0 {
        output::warn(format!(
            "{} corrupted line(s) skipped while replaying{}",
            replay.skipped_malformed,
            replay
                .first_malformed_line
                .map_or_else(String::new, |line| format!(" (first at line {line})"))
        ));
    }
    if replay.events.is_empty() {
        println!("(message log is empty)");
        return Ok(());
    }
    if options.orphans {
        render_orphans(&replay.events, options.role.as_deref());
        return Ok(());
    }
    let events = filter_show_events(&replay.events, options);
    if events.is_empty() {
        println!("(no matching events)");
        return Ok(());
    }
    for event in events {
        render_show_event(event, &host_role);
    }
    Ok(())
}

/// Surface the result of [`wal::scan_orphans`] in the same compact line
/// format used by other inspection commands.
fn render_orphans(events: &[CrepEvent], role_filter: Option<&str>) {
    let orphans = wal::scan_orphans(events);
    let filtered: Vec<&wal::OrphanTurn> = orphans
        .iter()
        .filter(|orphan| match role_filter {
            Some(role) => orphan.role == role,
            None => true,
        })
        .collect();
    if filtered.is_empty() {
        println!("(no orphan turns — every TurnIntent has a matching TurnCommit)");
        return;
    }
    println!(
        "{} orphan turn(s) — TurnIntent without TurnCommit. CodeRoom does not auto-reissue.",
        filtered.len()
    );
    for orphan in filtered {
        let parent = orphan
            .parent_hash
            .as_deref()
            .map_or_else(|| "<root>".to_owned(), str::to_owned);
        println!(
            "  @{role} · turn {turn_id} · thread {thread_id} · intent {intent_sha} · parent {parent}",
            role = orphan.role,
            turn_id = orphan.turn_id,
            thread_id = orphan.thread_id,
            intent_sha = orphan.intent_sha,
        );
    }
}

pub(super) fn filter_show_events<'a>(
    events: &'a [CrepEvent],
    options: &ShowOptions,
) -> Vec<&'a CrepEvent> {
    let mut filtered = events
        .iter()
        .filter(|event| match options.role.as_deref() {
            Some(role) => event_role(event) == role,
            None => true,
        })
        .collect::<Vec<_>>();

    if let Some(tail) = options.tail {
        let keep_from = filtered.len().saturating_sub(tail);
        filtered = filtered.split_off(keep_from);
    }

    filtered
}

fn event_role(event: &CrepEvent) -> &str {
    match event {
        CrepEvent::RoleStarted { role, .. }
        | CrepEvent::RoleSessionUpdated { role, .. }
        | CrepEvent::TurnDispatched { role, .. }
        | CrepEvent::WorkTitle { role, .. }
        | CrepEvent::RoleSpoke { role, .. }
        | CrepEvent::RoleOutputDelta { role, .. }
        | CrepEvent::TurnInterrupted { role, .. }
        | CrepEvent::ToolCallProposed { role, .. }
        | CrepEvent::ToolCallExecuted { role, .. }
        | CrepEvent::PermissionDenied { role, .. }
        | CrepEvent::RoleStopped { role, .. }
        | CrepEvent::TurnIntent { role, .. }
        | CrepEvent::TurnCommit { role, .. } => role,
    }
}

fn render_show_event(event: &CrepEvent, host_role: &str) {
    for event in normalize_show_event(event) {
        render_event(&event, host_role);
    }
}

pub(super) fn normalize_show_event(event: &CrepEvent) -> Vec<CrepEvent> {
    let CrepEvent::RoleSpoke {
        role,
        text,
        mentions,
        cost_usd,
        cache_read,
        turn_id,
        thread_id,
        priors_hash,
    } = event
    else {
        return vec![event.clone()];
    };

    let extracted = work::extract_cr_task(text);
    let mut events = Vec::new();
    if let Some(title) = extracted.title {
        events.push(CrepEvent::WorkTitle {
            role: role.clone(),
            title,
            turn_id: turn_id.clone(),
            thread_id: thread_id.clone(),
        });
    }
    let body = extracted.body.trim().to_owned();
    if !body.is_empty() {
        events.push(CrepEvent::RoleSpoke {
            role: role.clone(),
            text: body,
            mentions: mentions.clone(),
            cost_usd: *cost_usd,
            cache_read: *cache_read,
            turn_id: turn_id.clone(),
            thread_id: thread_id.clone(),
            priors_hash: priors_hash.clone(),
        });
    }
    events
}
