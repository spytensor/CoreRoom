//! CoreRoom rename identity tests.

use std::fs;

use coreroom::rename::{
    env_name, resolve_state_dir, LONG_COMMAND, NPM_PACKAGE, PRIMARY_COMMAND, PRODUCT_DESCRIPTOR,
    PRODUCT_NAME, STATE_DIR,
};

#[test]
fn accepted_product_name_and_descriptor_are_core_room() {
    assert_eq!(PRODUCT_NAME, "CoreRoom");
    assert_eq!(PRODUCT_DESCRIPTOR, "Engineering Control Room for AI Agents");
    assert_eq!(PRIMARY_COMMAND, "cr");
    assert_eq!(LONG_COMMAND, "coreroom");
    assert_eq!(NPM_PACKAGE, "@spytensor/coreroom");
}

#[test]
fn active_readme_uses_core_room_descriptor_and_stable_command() {
    let readme = include_str!("../README.md");

    assert!(readme.contains("# CoreRoom"));
    assert!(readme.contains("Engineering Control Room for AI Agents"));
    assert!(readme.contains("cr --version"));
}

#[test]
fn state_dir_uses_coreroom_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join(STATE_DIR)).unwrap();

    let resolved = resolve_state_dir(tmp.path());

    assert_eq!(resolved.selected, tmp.path().join(STATE_DIR));
    assert!(resolved.exists);
}

#[test]
fn state_dir_defaults_new_projects_to_coreroom() {
    let tmp = tempfile::tempdir().unwrap();

    let resolved = resolve_state_dir(tmp.path());

    assert_eq!(resolved.selected, tmp.path().join(STATE_DIR));
    assert!(!resolved.exists);
}

#[test]
fn env_name_uses_coreroom_prefix() {
    assert_eq!(env_name("NO_UPDATE_CHECK"), "COREROOM_NO_UPDATE_CHECK");
}
