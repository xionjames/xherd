//! Clock, change hub and tokens.

use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use xherd_core::{Agent, Millis};
use xherd_store::Store;

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Millis;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Millis {
        xherd_core::now_ms()
    }
}

/// Test clock moved by hand.
#[derive(Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(at: Millis) -> ManualClock {
        ManualClock(AtomicI64::new(at))
    }
    pub fn advance(&self, ms: Millis) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Millis {
        self.0.load(Ordering::SeqCst)
    }
}

/// Published on every agent change; the websocket hub (M2) forwards these.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Change {
    Agent { agent: Box<Agent> },
    Notify { agent_id: String, text: String },
}

pub type Hub = broadcast::Sender<Change>;

pub fn hub() -> Hub {
    broadcast::channel(1024).0
}

pub const TOKEN_USER: &str = "user";
pub const TOKEN_AGENT: &str = "agent";
pub const SCOPE_HOOKS: &str = "hooks";
pub const SCOPE_XAP: &str = "xap";
/// `xherd credential` from the agent's session (R-684).
pub const SCOPE_CREDENTIAL: &str = "credential";

/// A new random token (`xh_` + 64 hex chars).
pub fn generate_token() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("OS randomness");
    format!("xh_{}", hex::encode(b))
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Create and store (hashed) a token; returns the clear value, shown once.
pub async fn mint_token(
    store: &Store,
    kind: &str,
    subject: Option<&str>,
    scopes: &[&str],
    now: Millis,
) -> xherd_store::Result<String> {
    let token = generate_token();
    store
        .insert_token(
            &xherd_core::new_id(),
            kind,
            subject,
            scopes,
            &hash_token(&token),
            now,
        )
        .await?;
    Ok(token)
}
