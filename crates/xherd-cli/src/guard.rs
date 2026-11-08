//! `xherd guard pre-push` (R-1204): the check behind the worktree's
//! `pre-push` hook. Needs no daemon: it reads git's hook input and decides
//! with `xherd_core::guard`. Exit 1 refuses the push, with the reasons on stderr.

use std::io::Read;
use std::time::Duration;

use xherd_core::exec::{Cmd, run};
use xherd_core::guard::{parse_refs, violations};

/// Does git see `old` as an ancestor of `new`? A commit missing locally is not.
async fn is_ancestor(old: &str, new: &str) -> bool {
    let cmd =
        Cmd::new(["git", "merge-base", "--is-ancestor", old, new]).timeout(Duration::from_secs(30));
    run(cmd).await.is_ok_and(|o| o.success())
}

pub fn pre_push(branch: &str) -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    let refs = parse_refs(&input);
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 0;
    };
    // Ancestry answers are gathered first so the pure rules stay synchronous.
    let known: Vec<(String, String, bool)> = rt.block_on(async {
        let mut v = Vec::new();
        for r in &refs {
            if !r.remote_sha.chars().all(|c| c == '0') {
                v.push((
                    r.remote_sha.clone(),
                    r.local_sha.clone(),
                    is_ancestor(&r.remote_sha, &r.local_sha).await,
                ));
            }
        }
        v
    });
    let why = violations(branch, &refs, |old, new| {
        known.iter().any(|(o, n, ok)| o == old && n == new && *ok)
    });
    if why.is_empty() {
        return 0;
    }
    eprintln!("xherd: push refused");
    for w in why {
        eprintln!("  - {w}");
    }
    eprintln!("(xherd guard, R-1204; agents push only their own branch)");
    1
}
