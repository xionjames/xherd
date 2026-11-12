//! xherd domain: agent statuses and lifecycle, status reports, flow types,
//! paths and config, and the external command runner.

pub mod agent;
pub mod autonomy;
pub mod config;
pub mod container;
pub mod domain;
pub mod exec;
pub mod git_auth;
pub mod guard;
pub mod hash;
pub mod new;
pub mod paths;
pub mod permission;
pub mod pr;
pub mod profile;
pub mod queue;
pub mod report;
pub mod runner;
pub mod status;
pub mod ticket;
pub mod todo;
pub mod tripwire;
pub mod untrusted;
pub mod usage;
pub mod xap;

pub use agent::{Agent, Millis, new_id, now_ms};
pub use status::{Status, Trigger};

/// The Cargo version: what compatibility checks and the store compare.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The version shown to people (see `build.rs`): the git tag of a release,
/// `<tag>+<commits>.<hash>` in development.
pub const BUILD_VERSION: &str = env!("XHERD_BUILD_VERSION");
