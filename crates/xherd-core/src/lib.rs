//! xherd domain: agent statuses and lifecycle, status reports, flow types,
//! paths and config, and the external command runner.

pub mod agent;
pub mod config;
pub mod domain;
pub mod exec;
pub mod hash;
pub mod paths;
pub mod queue;
pub mod report;
pub mod status;
pub mod xap;

pub use agent::{Agent, Millis, new_id, now_ms};
pub use status::{Status, Trigger};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
