//! One round of PR comments (R-304, R-306): the files that carry it and the
//! decisions the user takes before the agent applies them.
//!
//! - `.xherd/pr-comments-raw.json`: the new comments as the host gave them
//!   (for the UI; the agent's copy in the spec folder is wrapped as untrusted).
//! - `.xherd/pr-overrides.json`: what the user decided in the UI, and which
//!   replies were already posted. Owned by xherd, replaced every round.
//! - `.xherd/spec/pr-decisions.json`: the final decisions for `pr-apply`,
//!   written when the user approves the `pr-review` gate.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;
use xherd_core::paths::write_atomic;
use xherd_core::pr::{Comment, Verdict};

use crate::{ActionError, Daemon, UserAction};

const RAW: &str = ".xherd/pr-comments-raw.json";
const OVERRIDES: &str = ".xherd/pr-overrides.json";
/// For the agent, in the spec folder.
pub const DECISIONS: &str = ".xherd/spec/pr-decisions.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overrides {
    /// By comment id.
    #[serde(default)]
    pub overrides: BTreeMap<String, Verdict>,
    /// Ids of the comments whose reply was posted.
    #[serde(default)]
    pub posted: Vec<String>,
}

fn io(e: impl ToString) -> String {
    e.to_string()
}

/// The round's comments as the host gave them; none before the first.
pub fn load_comments(wd: &Path) -> Vec<Comment> {
    #[derive(Deserialize)]
    struct File {
        comments: Vec<Comment>,
    }
    std::fs::read(wd.join(RAW))
        .ok()
        .and_then(|b| serde_json::from_slice::<File>(&b).ok())
        .map(|f| f.comments)
        .unwrap_or_default()
}

pub fn load_overrides(wd: &Path) -> Overrides {
    std::fs::read(wd.join(OVERRIDES))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_overrides(wd: &Path, o: &Overrides) -> Result<(), String> {
    let text = serde_json::to_string_pretty(o).map_err(io)?;
    write_atomic(&wd.join(OVERRIDES), text.as_bytes(), None).map_err(io)
}

/// A new round: the comments for the UI, and no decisions yet.
pub fn start_round(wd: &Path, comments: &[Comment]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&json!({"comments": comments})).map_err(io)?;
    write_atomic(&wd.join(RAW), text.as_bytes(), None).map_err(io)?;
    save_overrides(wd, &Overrides::default())
}

/// The agent's proposals of `pr-review.md` (none while it is missing).
pub fn load_proposals(wd: &Path) -> Result<Vec<Verdict>, String> {
    match std::fs::read_to_string(wd.join(xherd_flow::xap::SPEC_DIR).join("pr-review.md")) {
        Ok(text) => xherd_flow::pr_review::parse(&text),
        Err(_) => Ok(Vec::new()),
    }
}

/// The final decisions for the agent, after the replies went out.
pub fn write_decisions(wd: &Path, verdicts: &[Verdict], posted: &[String]) -> Result<(), String> {
    let rows: Vec<_> = verdicts
        .iter()
        .map(|v| {
            let mut r = serde_json::to_value(v).unwrap_or_default();
            r["posted"] = posted.contains(&v.id).into();
            r
        })
        .collect();
    let text = serde_json::to_string_pretty(&json!({"decisions": rows})).map_err(io)?;
    write_atomic(&wd.join(DECISIONS), text.as_bytes(), None).map_err(io)
}

impl Daemon {
    /// The round for the UI: the comments, the agent's proposals, what the
    /// user decided so far and which replies are posted. A malformed
    /// `pr-review.md` is reported as `error` (the gate refuses it too).
    pub async fn pr_review(&self, who: &str) -> Result<serde_json::Value, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let wd = crate::workdir_of(&a)?;
        let o = load_overrides(wd);
        let (proposals, error) = match load_proposals(wd) {
            Ok(p) => (p, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        Ok(json!({
            "comments": load_comments(wd),
            "proposals": proposals,
            "overrides": o.overrides,
            "posted": o.posted,
            "error": error,
        }))
    }

    /// Replace the user's decisions (`PUT /agents/:id/pr-decisions`). Ids must
    /// be comments of this round; completeness and reply texts are checked
    /// when the gate is approved.
    pub async fn set_pr_decisions(
        &self,
        who: &str,
        decisions: BTreeMap<String, Verdict>,
    ) -> Result<(), ActionError> {
        let bad = |message: String| ActionError {
            code: "bad_decisions",
            message,
        };
        let a = self.action(who, UserAction::Get).await?;
        let wd = crate::workdir_of(&a)?;
        let comments = load_comments(wd);
        for (id, v) in &decisions {
            if &v.id != id {
                return Err(bad(format!("decision of {id} names {}", v.id)));
            }
            if !comments.iter().any(|c| &c.id == id) {
                return Err(bad(format!("{id} is not a comment of this round")));
            }
        }
        let mut o = load_overrides(wd);
        o.overrides = decisions;
        save_overrides(wd, &o).map_err(|e| ActionError {
            code: "io",
            message: e,
        })
    }
}
