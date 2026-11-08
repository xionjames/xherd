//! Pre-push guard rules (R-1204), pure. git feeds a `pre-push` hook one line
//! per ref: `<local ref> <local sha> <remote ref> <remote sha>`.
//!
//! Allowed: pushing the agent's own branch to the same branch on the remote,
//! as a fast-forward. Refused: any other ref (base branch, tags, other
//! branches), deleting a remote ref, rewriting history. Hooks can be skipped
//! (`git push --no-verify`), so this stops mistakes, not a determined agent;
//! branch protection on the host is the real boundary.

const ZERO: char = '0';

/// One ref line of the hook's stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRef {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_ref: String,
    pub remote_sha: String,
}

fn is_zero(sha: &str) -> bool {
    !sha.is_empty() && sha.chars().all(|c| c == ZERO)
}

/// Parse the hook's stdin; malformed lines are skipped.
pub fn parse_refs(stdin: &str) -> Vec<PushRef> {
    stdin
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some(PushRef {
                local_ref: p.next()?.into(),
                local_sha: p.next()?.into(),
                remote_ref: p.next()?.into(),
                remote_sha: p.next()?.into(),
            })
        })
        .collect()
}

/// Why each ref would be refused (empty = the push may go ahead).
/// `is_ancestor(old, new)` answers whether `old` is reachable from `new`.
pub fn violations(
    branch: &str,
    refs: &[PushRef],
    is_ancestor: impl Fn(&str, &str) -> bool,
) -> Vec<String> {
    let own = format!("refs/heads/{branch}");
    let mut out = Vec::new();
    for r in refs {
        if is_zero(&r.local_sha) {
            out.push(format!("deleting {} is not allowed", r.remote_ref));
        } else if r.remote_ref != own {
            out.push(format!(
                "this agent may push only {branch}, not {}",
                r.remote_ref
            ));
        } else if !is_zero(&r.remote_sha) && !is_ancestor(&r.remote_sha, &r.local_sha) {
            out.push(format!(
                "{} would not be a fast-forward (force push); rebase or merge instead",
                r.remote_ref
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z: &str = "0000000000000000000000000000000000000000";

    fn r(local: &str, remote_ref: &str, remote_sha: &str) -> PushRef {
        PushRef {
            local_ref: "refs/heads/feat".into(),
            local_sha: local.into(),
            remote_ref: remote_ref.into(),
            remote_sha: remote_sha.into(),
        }
    }

    #[test]
    fn own_branch_new_or_fast_forward_is_allowed() {
        let ok = |_: &str, _: &str| true;
        assert!(violations("feat", &[r("aaa", "refs/heads/feat", Z)], ok).is_empty());
        assert!(violations("feat", &[r("bbb", "refs/heads/feat", "aaa")], ok).is_empty());
    }

    #[test]
    fn other_refs_deletions_and_force_pushes_are_refused() {
        let ok = |_: &str, _: &str| true;
        assert_eq!(
            violations("feat", &[r("aaa", "refs/heads/main", "bbb")], ok).len(),
            1
        );
        assert_eq!(
            violations("feat", &[r("aaa", "refs/tags/v1", Z)], ok).len(),
            1
        );
        let del = violations("feat", &[r(Z, "refs/heads/feat", "aaa")], ok);
        assert!(del[0].contains("deleting"), "{del:?}");
        let no_ff = |_: &str, _: &str| false;
        let f = violations("feat", &[r("bbb", "refs/heads/feat", "aaa")], no_ff);
        assert!(f[0].contains("fast-forward"), "{f:?}");
    }

    #[test]
    fn parses_hook_stdin() {
        let refs = parse_refs("refs/heads/a aaa refs/heads/a bbb\nbroken\n\n");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].remote_sha, "bbb");
    }
}
