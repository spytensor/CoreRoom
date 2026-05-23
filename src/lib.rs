//! CoreRoom — Engineering Control Room for AI Agents.
//!
//! Public API surface is intentionally small at v0.x; the binary `cr` is the
//! primary consumer. See `docs/architecture.md` for the v0.1 constitution and
//! `docs/v0.2-trust-and-interrupt.md` for the v0.2 amendment.

#![doc(html_root_url = "https://docs.rs/coreroom/0.7.0")]
#![warn(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod adapter;
pub mod bus;
pub mod config;
pub mod config_cmd;
pub mod config_layered;
pub mod console_snapshot;
pub mod context_pack;
pub mod cost;
pub mod crep;
pub mod detect;
pub mod doctor;
pub mod engines;
pub mod evidence_packet;
pub mod gate;
pub mod github_status;
pub mod host_action;
pub mod image_paths;
pub mod init;
pub mod liveness;
pub mod lock;
pub mod manifest;
pub mod output;
pub(crate) mod peer_quote;
pub mod permissions;
pub mod pointers;
pub mod priors;
pub mod project_status;
pub mod prompt_cmd;
pub mod rename;
pub mod repl;
pub mod role;
pub mod source_graph;
pub mod source_registry;
pub mod tracker;
pub mod turn;
pub mod update;
mod work;
pub mod work_order;
