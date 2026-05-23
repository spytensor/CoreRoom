//! CoreRoom rename migration compatibility tests.

use std::fs;

use coderoom::rename::{
    current_env_name, legacy_env_name, resolve_env_alias_from_values, resolve_state_dir,
    EnvAliasSource, StateDirKind, CURRENT_STATE_DIR, LEGACY_STATE_DIR, PRIMARY_COMMAND,
    PRODUCT_DESCRIPTOR, PRODUCT_NAME,
};

#[test]
fn accepted_product_name_and_descriptor_are_core_room() {
    assert_eq!(PRODUCT_NAME, "CoreRoom");
    assert_eq!(PRODUCT_DESCRIPTOR, "Engineering Control Room for AI Agents");
    assert_eq!(PRIMARY_COMMAND, "cr");
}

#[test]
fn active_readme_uses_core_room_descriptor_and_stable_command() {
    let readme = include_str!("../README.md");

    assert!(readme.contains("# CoreRoom"));
    assert!(readme.contains("Engineering Control Room for AI Agents"));
    assert!(readme.contains("cr --version"));
}

#[test]
fn state_dir_uses_current_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join(CURRENT_STATE_DIR)).unwrap();

    let resolved = resolve_state_dir(tmp.path());

    assert_eq!(resolved.kind, StateDirKind::Current);
    assert_eq!(
        resolved.selected.as_ref().unwrap(),
        &tmp.path().join(CURRENT_STATE_DIR)
    );
    assert!(resolved.is_usable());
    assert!(!resolved.uses_legacy());
}

#[test]
fn state_dir_accepts_legacy_with_migration_note() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join(LEGACY_STATE_DIR)).unwrap();

    let resolved = resolve_state_dir(tmp.path());

    assert_eq!(resolved.kind, StateDirKind::Legacy);
    assert_eq!(
        resolved.selected.as_ref().unwrap(),
        &tmp.path().join(LEGACY_STATE_DIR)
    );
    assert!(resolved.is_usable());
    assert!(resolved.uses_legacy());
    assert!(resolved.note.contains("explicit confirmation"));
}

#[test]
fn state_dir_fails_loudly_when_both_exist() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join(CURRENT_STATE_DIR)).unwrap();
    fs::create_dir(tmp.path().join(LEGACY_STATE_DIR)).unwrap();

    let resolved = resolve_state_dir(tmp.path());

    assert_eq!(resolved.kind, StateDirKind::Conflict);
    assert!(resolved.selected.is_none());
    assert!(!resolved.is_usable());
    assert!(resolved.note.contains("user must resolve"));
}

#[test]
fn state_dir_defaults_new_projects_to_current_name() {
    let tmp = tempfile::tempdir().unwrap();

    let resolved = resolve_state_dir(tmp.path());

    assert_eq!(resolved.kind, StateDirKind::MissingUseCurrent);
    assert_eq!(
        resolved.selected.as_ref().unwrap(),
        &tmp.path().join(CURRENT_STATE_DIR)
    );
}

#[test]
fn env_alias_prefers_coreroom_over_coderoom() {
    let current = current_env_name("NO_UPDATE_CHECK");
    let legacy = legacy_env_name("NO_UPDATE_CHECK");

    let resolved = resolve_env_alias_from_values(&current, Some("1"), &legacy, Some("legacy-1"));

    assert_eq!(resolved.value, Some("1"));
    assert_eq!(resolved.source, EnvAliasSource::CurrentPreferredOverLegacy);
    assert!(resolved.legacy_used);
}

#[test]
fn env_alias_accepts_legacy_when_current_is_missing() {
    let current = current_env_name("NO_UPDATE_CHECK");
    let legacy = legacy_env_name("NO_UPDATE_CHECK");

    let resolved = resolve_env_alias_from_values(&current, None, &legacy, Some("1"));

    assert_eq!(resolved.value, Some("1"));
    assert_eq!(resolved.source, EnvAliasSource::Legacy);
    assert!(resolved.legacy_used);
}

#[test]
fn env_alias_reports_missing_when_no_spelling_is_set() {
    let current = current_env_name("NO_UPDATE_CHECK");
    let legacy = legacy_env_name("NO_UPDATE_CHECK");

    let resolved = resolve_env_alias_from_values(&current, None, &legacy, None);

    assert_eq!(resolved.value, None);
    assert_eq!(resolved.source, EnvAliasSource::Missing);
    assert!(!resolved.legacy_used);
}
