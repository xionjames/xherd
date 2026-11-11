//! Recognising git/host authentication failures in command output or on a
//! terminal screen (R-688). Pure; the actor decides what to do with a match.

/// Lower-case fragments git, ssh, `gh` and `glab` print when access is refused.
const MARKERS: &[&str] = &[
    "authentication failed",
    "permission denied (publickey",
    "could not read username",
    "could not read password",
    "terminal prompts disabled",
    "invalid username or password",
    "support for password authentication was removed",
    "bad credentials",
    "http 401",
    "host key verification failed",
    "saml enforcement",
    "authorize your token",
    "you are not logged into any github hosts",
    "gh auth login",
    "glab auth login",
];

/// The first line of `text` that looks like an authentication failure,
/// trimmed and capped at 200 characters.
pub fn failing_line(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let low = l.to_lowercase();
        MARKERS
            .iter()
            .any(|m| low.contains(m))
            .then(|| l.trim().chars().take(200).collect())
    })
}

pub fn is_auth_failure(text: &str) -> bool {
    failing_line(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_usual_messages() {
        for s in [
            "remote: Invalid username or password.\nfatal: Authentication failed for 'https://github.com/o/r.git/'",
            "git@github.com: Permission denied (publickey).",
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "HTTP 401: Bad credentials (https://api.github.com/graphql)",
            "To get started with GitHub CLI, please run:  gh auth login",
        ] {
            assert!(is_auth_failure(s), "{s}");
        }
    }

    #[test]
    fn ignores_other_failures() {
        for s in [
            "error: failed to push some refs (non-fast-forward)",
            "no checks reported",
            "",
        ] {
            assert!(!is_auth_failure(s), "{s}");
        }
    }

    #[test]
    fn returns_the_matching_line() {
        let t = "fatal: something\nfatal: Authentication failed for x\nmore";
        assert_eq!(
            failing_line(t).unwrap(),
            "fatal: Authentication failed for x"
        );
    }
}
