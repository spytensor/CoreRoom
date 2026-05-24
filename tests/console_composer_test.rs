//! Renderer-independent console composer fixtures.

use coreroom::console_composer::{
    ComposerAcceptOutcome, ComposerCommandSpec, ComposerState, ComposerSubmissionState,
    ComposerSubmitOutcome,
};

fn composer() -> ComposerState {
    ComposerState::new(
        vec![
            "host".to_owned(),
            "backend".to_owned(),
            "reviewer".to_owned(),
            "security".to_owned(),
        ],
        vec![
            ComposerCommandSpec::new("allow", "allow a tool for the session", true),
            ComposerCommandSpec::new("exit", "leave the room", false),
            ComposerCommandSpec::new("fresh", "restart roles cleanly", false),
            ComposerCommandSpec::new("halt", "interrupt current turn", false),
            ComposerCommandSpec::new("help", "show help", false),
            ComposerCommandSpec::new("refresh", "refresh a role", true),
        ],
        "type a task - @role - /help",
    )
}

#[test]
fn composer_completes_role_mentions_without_submitting_empty_task() {
    let mut composer = composer();
    composer.insert_char('@');
    composer.insert_char('h');

    let view = composer.view_model();
    assert_eq!(view.ghost_suffix.as_deref(), Some("ost"));

    assert_eq!(composer.submit(), ComposerSubmitOutcome::CompletionAccepted);
    assert_eq!(composer.input(), "@host ");
    assert_eq!(composer.submission_state(), ComposerSubmissionState::Idle);
}

#[test]
fn composer_submits_bare_text_for_host_routing() {
    let mut composer = composer();
    composer.paste_str("scope the unified room composer");

    assert_eq!(
        composer.submit(),
        ComposerSubmitOutcome::Submitted("scope the unified room composer".to_owned())
    );
    assert_eq!(composer.input(), "");
    assert_eq!(
        composer.submission_state(),
        ComposerSubmissionState::Submitting
    );
}

#[test]
fn composer_supports_explicit_role_task_submission() {
    let mut composer = composer();
    composer.paste_str("@reviewer check the architecture");

    assert_eq!(
        composer.submit(),
        ComposerSubmitOutcome::Submitted("@reviewer check the architecture".to_owned())
    );
}

#[test]
fn composer_paste_preserves_multiline_content_as_one_submission() {
    let mut composer = composer();
    composer.paste_str("line 1\r\nline 2\rcmd\targ\x07");

    let view = composer.view_model();
    assert!(view.multiline);
    assert_eq!(view.input, "line 1\nline 2\ncmd\targ");
    assert_eq!(
        composer.submit(),
        ComposerSubmitOutcome::Submitted("line 1\nline 2\ncmd\targ".to_owned())
    );
}

#[test]
fn composer_insert_newline_keeps_multiline_buffer_until_submit() {
    let mut composer = composer();
    composer.paste_str("first");
    composer.insert_newline();
    composer.paste_str("second");

    assert!(composer.view_model().multiline);
    assert_eq!(
        composer.submit(),
        ComposerSubmitOutcome::Submitted("first\nsecond".to_owned())
    );
}

#[test]
fn composer_completes_slash_commands_with_argument_space_when_needed() {
    let mut composer = composer();
    composer.paste_str("/ref");

    assert_eq!(
        composer.accept_completion(),
        ComposerAcceptOutcome::ExpectsMore
    );
    assert_eq!(composer.input(), "/refresh ");
    assert_eq!(composer.cursor(), "/refresh ".chars().count());
}

#[test]
fn composer_completes_argless_slash_commands_as_submittable() {
    let mut composer = composer();
    composer.paste_str("/hel");

    assert_eq!(
        composer.accept_completion(),
        ComposerAcceptOutcome::Complete
    );
    assert_eq!(composer.input(), "/help");
    assert_eq!(
        composer.submit(),
        ComposerSubmitOutcome::Submitted("/help".to_owned())
    );
}

#[test]
fn composer_cycles_completion_candidates_and_exposes_selected_row() {
    let mut composer = composer();
    composer.insert_char('@');

    let first = composer.view_model();
    assert!(first.candidates.iter().any(|candidate| candidate.selected));
    assert_eq!(first.candidates[0].label, "@backend");

    assert!(composer.cycle_completion());
    let second = composer.view_model();
    assert!(second.candidates[1].selected);

    assert!(composer.cycle_completion_back());
    let third = composer.view_model();
    assert!(third.candidates[0].selected);
}

#[test]
fn composer_dismisses_menu_without_erasing_ghost_hint() {
    let mut composer = composer();
    composer.insert_char('/');
    composer.insert_char('h');

    assert!(composer.view_model().candidates.len() >= 2);
    assert!(composer.dismiss_completion());

    let view = composer.view_model();
    assert!(view.candidates.is_empty());
    assert_eq!(view.ghost_suffix.as_deref(), Some("alt"));
}

#[test]
fn composer_blocks_submit_while_runtime_is_not_idle() {
    let mut composer = composer();
    composer.paste_str("hello");
    composer.set_submission_state(ComposerSubmissionState::Blocked);

    assert_eq!(composer.submit(), ComposerSubmitOutcome::Noop);
    assert_eq!(composer.input(), "hello");
}

#[test]
fn composer_cursor_editing_invalidates_completion_and_preserves_text() {
    let mut composer = composer();
    composer.paste_str("hello");
    assert!(composer.move_left());
    assert!(composer.backspace());
    composer.insert_char('p');

    assert_eq!(composer.input(), "helpo");
    assert_eq!(composer.cursor(), 4);
    assert!(composer.move_end());
    assert!(!composer.delete());
}
