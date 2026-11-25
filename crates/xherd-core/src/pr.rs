//! Pull-request follow-up (R-303, R-305, R-307, pure): what a poll of the host
//! found, and what it means for the agent. The host plugin answers with a
//! `Snapshot`; `Job::apply_pr` keeps the last state and says what is new.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Job;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

/// Whether the pull request merges into its base without conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mergeable {
    Yes,
    No,
    Unknown,
}

/// Rolled-up CI result of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ci {
    Pending,
    Failing,
    Passing,
}

/// One check run. `bucket` is the host's roll-up of its result: `pass`,
/// `fail`, `pending`, `skipping` or `cancel`. `workflow` groups checks of
/// the same name; `link` is the page of the run (host text: the UI shows it
/// only when it is an https URL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub name: String,
    pub bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// Where a reviewer stands on the pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewState {
    /// Asked to review, no review since.
    Pending,
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

/// A person or team asked to review, or who reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    pub name: String,
    pub state: ReviewState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub state: PrState,
    pub mergeable: Mergeable,
    pub checks: Vec<CheckRun>,
    pub reviewers: Vec<Reviewer>,
    /// The branch the pull request merges into, when the host says.
    pub base: Option<String>,
    /// The pull request's title and description on the host, when it says.
    /// Untrusted text.
    pub title: Option<String>,
    pub body: Option<String>,
}

/// The remote-tracking ref of a pull request's base branch: the host's base
/// when it gave one, else the agent's. `origin/<b>`, kept as is when it
/// already starts with `origin/`. `None` when there is no base or the name is
/// not plain (characters outside `[A-Za-z0-9._/-]`, a leading `-`, or `..`):
/// the ref is typed into a terminal and passed to git.
pub fn base_ref(host: Option<&str>, agent: Option<&str>) -> Option<String> {
    let b = host.or(agent)?;
    let plain = !b.is_empty()
        && !b.starts_with('-')
        && !b.contains("..")
        && b.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if !plain {
        return None;
    }
    Some(if b.starts_with("origin/") {
        b.to_string()
    } else {
        format!("origin/{b}")
    })
}

/// A pull request description as compared by `TextSync`: line endings as LF
/// and trailing whitespace dropped, nothing else.
pub fn norm_body(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}

/// A pull request title as compared by `TextSync`: trimmed at both ends.
pub fn norm_title(s: &str) -> String {
    s.trim().to_string()
}

/// What `TextSync::sync` found for one text field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sync {
    /// Nothing to do.
    Nothing,
    /// The local text and the host's agree; the baseline follows.
    Baseline,
    /// The host's text changed and the local one did not: write it locally.
    Write(String),
    /// Both changed (or the host's is empty): tell the user, once.
    Diverged,
}

/// Per-field memory of the PR text sync: hashes (`content_sha256`) of the
/// normalised text, never the text itself (host text is untrusted).
/// `baseline` is the value both sides last agreed on; `notified` the host
/// value a divergence was already reported for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSync {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified: Option<String>,
}

fn text_hash(s: &str) -> String {
    crate::hash::content_sha256(s.as_bytes())
}

impl TextSync {
    pub fn is_empty(&self) -> bool {
        self.baseline.is_none() && self.notified.is_none()
    }

    /// The local text is what the host has now (after a post): it becomes the
    /// baseline.
    pub fn seed(&mut self, local: &str) {
        self.baseline = Some(text_hash(local));
        self.notified = None;
    }

    /// Compare the host's text with the local one (both already normalised).
    /// `local: None` means the document has no such field, which counts as
    /// unchanged since the baseline. The table is in design.md.
    pub fn sync(&mut self, host: Option<&str>, local: Option<&str>) -> Sync {
        let Some(host) = host else {
            return Sync::Nothing;
        };
        let h = text_hash(host);
        let Some(b) = self.baseline.clone() else {
            self.baseline = Some(h);
            self.notified = None;
            return Sync::Nothing;
        };
        let l = local.map_or_else(|| b.clone(), text_hash);
        if h == l {
            self.baseline = Some(h);
            self.notified = None;
            return Sync::Baseline;
        }
        if l == b {
            if !host.is_empty() {
                self.baseline = Some(h);
                self.notified = None;
                return Sync::Write(host.to_string());
            }
        } else if h == b {
            return Sync::Nothing;
        }
        if self.notified.as_deref() == Some(h.as_str()) {
            return Sync::Nothing;
        }
        self.notified = Some(h);
        Sync::Diverged
    }
}

/// Hidden marker xherd puts in the comments it posts, so it never reads its
/// own replies as new comments (R-304).
pub const MARKER: &str = "<!-- xherd -->";

/// A comment on the pull request, as the host plugin reports it (spec §10.1
/// `pr-comments`). `body` is untrusted text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// Unique on the PR.
    pub id: String,
    /// `review` (a review's own text), `review-comment` (on a line of the
    /// diff) or `issue` (the conversation).
    pub kind: String,
    pub author: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
}

/// What to do about a comment (R-306).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// The code must change.
    Fix,
    /// An answer is enough.
    Reply,
    /// Nothing to do.
    Skip,
}

/// The decision about one comment: proposed by the agent in `pr-review.md`
/// or set by the user, with a note and, for answers, the reply text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub id: String,
    pub decision: Decision,
    /// Why (the user's note, or the agent's reason).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The answer to post for `reply`; a draft for `fix`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

/// One verdict per comment: the user's `overrides` win over the agent's
/// `proposals`. A comment with neither, an unknown id in either list, or a
/// `reply` decision without text is an error naming it (R-306).
pub fn decide(
    comments: &[Comment],
    proposals: &[Verdict],
    overrides: &BTreeMap<String, Verdict>,
) -> Result<Vec<Verdict>, String> {
    let known = |id: &str| comments.iter().any(|c| c.id == id);
    if let Some(v) = proposals.iter().find(|v| !known(&v.id)) {
        return Err(format!("{} is not one of the comments", v.id));
    }
    if let Some(id) = overrides.keys().find(|id| !known(id)) {
        return Err(format!("{id} is not one of the comments"));
    }
    let mut out = Vec::new();
    for c in comments {
        let v = overrides
            .get(&c.id)
            .or_else(|| proposals.iter().find(|v| v.id == c.id))
            .ok_or_else(|| format!("comment {} has no decision", c.id))?;
        if v.decision == Decision::Reply && v.reply.as_deref().is_none_or(|r| r.trim().is_empty()) {
            return Err(format!(
                "comment {} is decided `reply` but has no reply text",
                c.id
            ));
        }
        out.push(v.clone());
    }
    Ok(out)
}

/// Something new in a snapshot, to tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Merged or closed (R-307).
    Finished(PrState),
    /// Checks started failing: the names of the failing ones (R-305).
    CiFailing(Vec<String>),
    /// All checks pass after having failed (R-305).
    CiPassingAgain,
    /// The PR cannot merge as it is; said once until it can again (R-305).
    Conflict,
}

impl Snapshot {
    /// `None` when the PR has no checks.
    pub fn ci(&self) -> Option<Ci> {
        if self.checks.is_empty() {
            None
        } else if self.checks.iter().any(|c| c.bucket == "fail") {
            Some(Ci::Failing)
        } else if self.checks.iter().any(|c| c.bucket == "pending") {
            Some(Ci::Pending)
        } else {
            Some(Ci::Passing)
        }
    }
}

impl Job {
    /// The comments of `all` not seen before and not written by xherd
    /// (R-304); they count as seen from now on. Comments xherd wrote are
    /// remembered too, so they stay out later.
    pub fn new_comments(&mut self, all: &[Comment]) -> Vec<Comment> {
        let mut fresh = Vec::new();
        for c in all {
            if self.pr_comments_seen.contains(&c.id) {
                continue;
            }
            self.pr_comments_seen.push(c.id.clone());
            if !c.body.contains(MARKER) {
                fresh.push(c.clone());
            }
        }
        fresh
    }

    /// Take a poll's result in and return what is new.
    pub fn apply_pr(&mut self, s: &Snapshot) -> Vec<Event> {
        let mut out = Vec::new();
        self.pr_state = Some(s.state);
        self.pr_checks = s.checks.clone();
        self.pr_reviewers = s.reviewers.clone();
        self.pr_mergeable = Some(s.mergeable);
        if s.base.is_some() {
            self.pr_base = s.base.clone();
        }
        if s.state != PrState::Open {
            out.push(Event::Finished(s.state));
            return out;
        }
        let ci = s.ci();
        match ci {
            Some(Ci::Failing) if self.pr_ci != Some(Ci::Failing) => {
                self.pr_ci_failed = true;
                let names = s
                    .checks
                    .iter()
                    .filter(|c| c.bucket == "fail")
                    .map(|c| c.name.clone())
                    .collect();
                out.push(Event::CiFailing(names));
            }
            Some(Ci::Passing) if self.pr_ci_failed => {
                self.pr_ci_failed = false;
                out.push(Event::CiPassingAgain);
            }
            _ => {}
        }
        self.pr_ci = ci;
        match s.mergeable {
            Mergeable::No if !self.pr_conflict => {
                self.pr_conflict = true;
                out.push(Event::Conflict);
            }
            Mergeable::Yes => self.pr_conflict = false,
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: PrState, mergeable: Mergeable, checks: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            state,
            mergeable,
            checks: checks
                .iter()
                .map(|(n, b)| CheckRun {
                    name: n.to_string(),
                    bucket: b.to_string(),
                    workflow: None,
                    link: None,
                })
                .collect(),
            reviewers: Vec::new(),
            base: None,
            title: None,
            body: None,
        }
    }

    #[test]
    fn a_poll_keeps_checks_reviewers_and_merge_state() {
        let mut a = Job::new("A", 1);
        let mut s = snap(PrState::Open, Mergeable::Unknown, &[("t", "pass")]);
        s.reviewers = vec![Reviewer {
            name: "bob".into(),
            state: ReviewState::Approved,
        }];
        a.apply_pr(&s);
        assert_eq!(a.pr_checks, s.checks);
        assert_eq!(a.pr_reviewers, s.reviewers);
        assert_eq!(a.pr_mergeable, Some(Mergeable::Unknown));
    }

    #[test]
    fn a_poll_keeps_the_hosts_base() {
        let mut a = Job::new("A", 1);
        let mut s = snap(PrState::Open, Mergeable::Yes, &[]);
        s.base = Some("develop".into());
        a.apply_pr(&s);
        assert_eq!(a.pr_base.as_deref(), Some("develop"));
        s.base = None;
        a.apply_pr(&s);
        assert_eq!(a.pr_base.as_deref(), Some("develop"));
    }

    #[test]
    fn base_ref_is_plain_and_prefixed() {
        assert_eq!(
            base_ref(Some("main"), Some("dev")).as_deref(),
            Some("origin/main")
        );
        assert_eq!(base_ref(None, Some("dev")).as_deref(), Some("origin/dev"));
        assert_eq!(
            base_ref(None, Some("origin/rel/1.2")).as_deref(),
            Some("origin/rel/1.2")
        );
        assert_eq!(base_ref(None, None), None);
        for bad in ["", "-x", "a..b", "a b", "a;rm", "a$(x)", "a\nb", "ü"] {
            assert_eq!(base_ref(Some(bad), None), None, "{bad:?}");
        }
    }

    #[test]
    fn ci_rolls_up_checks() {
        let ci = |c: &[(&str, &str)]| snap(PrState::Open, Mergeable::Yes, c).ci();
        assert_eq!(ci(&[]), None);
        assert_eq!(ci(&[("a", "pass"), ("b", "skipping")]), Some(Ci::Passing));
        assert_eq!(ci(&[("a", "pass"), ("b", "pending")]), Some(Ci::Pending));
        assert_eq!(ci(&[("a", "fail"), ("b", "pending")]), Some(Ci::Failing));
    }

    #[test]
    fn failing_then_passing_notifies_once_each() {
        let mut a = Job::new("A", 1);
        let pass = snap(PrState::Open, Mergeable::Yes, &[("t", "pass")]);
        assert_eq!(a.apply_pr(&pass), [], "passing from the start is not news");
        let fail = snap(
            PrState::Open,
            Mergeable::Yes,
            &[("t", "fail"), ("l", "pass")],
        );
        assert_eq!(a.apply_pr(&fail), [Event::CiFailing(vec!["t".into()])]);
        assert_eq!(a.apply_pr(&fail), [], "still failing: said once");
        let pending = snap(PrState::Open, Mergeable::Yes, &[("t", "pending")]);
        assert_eq!(a.apply_pr(&pending), []);
        assert_eq!(a.apply_pr(&pass), [Event::CiPassingAgain]);
        assert_eq!(a.apply_pr(&pass), []);
        // Fails again after a pending run: news again.
        assert_eq!(a.apply_pr(&fail).len(), 1);
    }

    #[test]
    fn conflict_is_said_once_until_it_clears() {
        let mut a = Job::new("A", 1);
        let bad = snap(PrState::Open, Mergeable::No, &[]);
        assert_eq!(a.apply_pr(&bad), [Event::Conflict]);
        assert_eq!(a.apply_pr(&bad), []);
        let unknown = snap(PrState::Open, Mergeable::Unknown, &[]);
        assert_eq!(a.apply_pr(&unknown), [], "unknown keeps the flag");
        assert_eq!(a.apply_pr(&bad), []);
        a.apply_pr(&snap(PrState::Open, Mergeable::Yes, &[]));
        assert_eq!(a.apply_pr(&bad), [Event::Conflict]);
    }

    fn comment(id: &str, body: &str) -> Comment {
        Comment {
            id: id.into(),
            kind: "issue".into(),
            author: "alice".into(),
            body: body.into(),
            url: String::new(),
            created_at: String::new(),
            path: None,
            line: None,
            in_reply_to: None,
        }
    }

    #[test]
    fn comments_are_new_once_and_never_ours() {
        let mut a = Job::new("A", 1);
        let all = vec![
            comment("1", "please rename"),
            comment("2", &format!("done {MARKER}")),
        ];
        let fresh = a.new_comments(&all);
        assert_eq!(
            fresh.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["1"]
        );
        assert_eq!(a.new_comments(&all), [], "seen already, ours included");
        let more = [all, vec![comment("3", "and this")]].concat();
        assert_eq!(a.new_comments(&more).len(), 1);
    }

    fn verdict(id: &str, d: Decision, reply: Option<&str>) -> Verdict {
        Verdict {
            id: id.into(),
            decision: d,
            note: None,
            reply: reply.map(str::to_string),
        }
    }

    #[test]
    fn user_decisions_win_and_every_comment_needs_one() {
        let comments = [comment("a", "x"), comment("b", "y")];
        let proposals = [
            verdict("a", Decision::Fix, None),
            verdict("b", Decision::Skip, None),
        ];
        let mut over = BTreeMap::new();
        over.insert("b".into(), verdict("b", Decision::Reply, Some("thanks")));
        let v = decide(&comments, &proposals, &over).unwrap();
        assert_eq!(
            (v[0].decision, v[1].decision),
            (Decision::Fix, Decision::Reply)
        );

        let e = decide(&comments, &proposals[..1], &BTreeMap::new()).unwrap_err();
        assert!(e.contains("comment b has no decision"), "{e}");
        let e = decide(&comments, &[verdict("z", Decision::Skip, None)], &over).unwrap_err();
        assert!(e.contains("z is not one of the comments"), "{e}");
        let bad = [
            verdict("a", Decision::Reply, Some("  ")),
            proposals[1].clone(),
        ];
        let e = decide(&comments, &bad, &BTreeMap::new()).unwrap_err();
        assert!(e.contains("no reply text"), "{e}");
    }

    fn seeded(base: &str) -> TextSync {
        let mut t = TextSync::default();
        t.seed(base);
        t
    }

    #[test]
    fn text_sync_host_unknown_does_nothing() {
        let mut t = seeded("a");
        assert_eq!(t.sync(None, Some("b")), Sync::Nothing);
        assert_eq!(t, seeded("a"));
    }

    #[test]
    fn text_sync_without_baseline_takes_the_host_value() {
        let mut t = TextSync::default();
        assert_eq!(t.sync(Some("h"), Some("l")), Sync::Nothing);
        assert_eq!(t, seeded("h"));
    }

    #[test]
    fn text_sync_equal_texts_move_the_baseline() {
        let mut t = seeded("old");
        t.notified = Some("x".into());
        assert_eq!(t.sync(Some("new"), Some("new")), Sync::Baseline);
        assert_eq!(t, seeded("new"));
    }

    #[test]
    fn text_sync_unchanged_is_nothing() {
        let mut t = seeded("a");
        assert_eq!(t.sync(Some("a"), Some("a")), Sync::Baseline);
        assert_eq!(t.sync(Some("a"), None), Sync::Baseline);
    }

    #[test]
    fn text_sync_host_edit_is_written_locally() {
        let mut t = seeded("a");
        assert_eq!(t.sync(Some("h"), Some("a")), Sync::Write("h".into()));
        assert_eq!(t, seeded("h"));
        // A missing local field counts as unchanged.
        let mut t = seeded("a");
        assert_eq!(t.sync(Some("h"), None), Sync::Write("h".into()));
    }

    #[test]
    fn text_sync_empty_host_value_notifies_once_and_is_never_written() {
        let mut t = seeded("a");
        assert_eq!(t.sync(Some(""), Some("a")), Sync::Diverged);
        assert_eq!(t.sync(Some(""), Some("a")), Sync::Nothing);
        assert_eq!(t.baseline, seeded("a").baseline);
    }

    #[test]
    fn text_sync_both_changed_notifies_once_per_host_value() {
        let mut t = seeded("a");
        assert_eq!(t.sync(Some("h"), Some("l")), Sync::Diverged);
        assert_eq!(t.sync(Some("h"), Some("l")), Sync::Nothing);
        assert_eq!(t.sync(Some("h2"), Some("l")), Sync::Diverged);
        // The local edit is untouched: the baseline stays.
        assert_eq!(t.baseline, seeded("a").baseline);
    }

    #[test]
    fn text_sync_local_edit_alone_is_nothing() {
        let mut t = seeded("a");
        assert_eq!(t.sync(Some("a"), Some("l")), Sync::Nothing);
        assert_eq!(t, seeded("a"));
    }

    #[test]
    fn text_sync_host_edit_after_a_divergence_is_written_once_local_is_back() {
        let mut t = seeded("a");
        assert_eq!(t.sync(Some("h"), Some("l")), Sync::Diverged);
        t.seed("l"); // the agent posted its text
        assert_eq!(t.sync(Some("h"), Some("l")), Sync::Write("h".into()));
        assert_eq!(t.notified, None);
    }

    #[test]
    fn normalisation_is_crlf_and_trailing_whitespace_only() {
        assert_eq!(norm_body("a\r\nb  \r\n\r\n"), "a\nb");
        assert_eq!(norm_body("  a\n\n\nb\n"), "  a\n\n\nb");
        assert_eq!(norm_title("  Add x \n"), "Add x");
        let mut t = seeded(&norm_body("a\nb\n"));
        assert_eq!(
            t.sync(Some(&norm_body("a\r\nb\r\n")), Some(&norm_body("a\nb"))),
            Sync::Baseline
        );
    }

    #[test]
    fn text_sync_json_omits_empty_and_reads_old_jobs() {
        assert_eq!(serde_json::to_string(&TextSync::default()).unwrap(), "{}");
        assert!(serde_json::from_str::<TextSync>("{}").unwrap().is_empty());
        let t = seeded("a");
        let back: TextSync = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }

    proptest::proptest! {
        #[test]
        fn text_sync_never_writes_over_a_local_edit(
            base in "[a-z]{0,6}", local in "[a-z]{0,6}", host in "[a-z]{0,6}",
        ) {
            proptest::prop_assume!(base != local);
            let mut t = seeded(&base);
            let r = t.sync(Some(&host), Some(&local));
            proptest::prop_assert!(!matches!(r, Sync::Write(_)));
            // The same call again is never a second divergence.
            let again = t.sync(Some(&host), Some(&local));
            proptest::prop_assert!(again != Sync::Diverged);
        }
    }

    #[test]
    fn merged_or_closed_finishes() {
        let mut a = Job::new("A", 1);
        let s = snap(PrState::Merged, Mergeable::Unknown, &[("t", "fail")]);
        assert_eq!(a.apply_pr(&s), [Event::Finished(PrState::Merged)]);
        assert_eq!(a.pr_state, Some(PrState::Merged));
    }
}
