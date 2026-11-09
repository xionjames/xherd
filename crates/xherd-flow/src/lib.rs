//! Flow packs (design §6): the `xherd-flow.yaml` manifest, its stage graph and
//! `next` rules, command templates, and document frontmatter. The daemon never
//! interprets a pack's prose; it only needs what is here.

pub mod approve;
pub mod frontmatter;
pub mod install;
pub mod manifest;
pub mod memory;
pub mod progress;
pub mod review;
pub mod tasks;
pub mod template;
pub mod xap;

pub use manifest::{
    Condition, Facts, Flow, GateKind, GateSpec, NextRule, Pack, PackError, StageDef,
};

/// XAP major version this daemon speaks (xap.md §9).
pub const XAP_PROTOCOL: u32 = 1;

/// Name of the built-in pack.
pub const BUILTIN: &str = "xherd";

/// The built-in xherd pack (spec §7).
pub fn builtin() -> Pack {
    Pack::parse(include_str!("../../../packs/xherd/xherd-flow.yaml"))
        .expect("built-in xherd manifest is valid (tested)")
}
