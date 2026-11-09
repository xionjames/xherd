//! Autonomy (spec §6.3): which gates xherd may approve by itself.
//!
//! Pure policy. The actor decides *when* to ask (a stop at an open gate) and
//! adds the tripwires (§6.4); this module only answers "may this level approve
//! this class of gate" and "has the run limit been reached".

use serde::{Deserialize, Serialize};

use crate::domain::Class;

/// How much xherd may approve without a human (R-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Autonomy {
    /// Every gate waits for the user.
    #[default]
    Ask,
    /// Routine and code gates are approved; critical ones (spec, ship) wait.
    Important,
    /// Unattended: critical gates too (v1.x, R-1209). Not selectable yet.
    Never,
}

impl Autonomy {
    pub fn as_str(self) -> &'static str {
        match self {
            Autonomy::Ask => "ask",
            Autonomy::Important => "important",
            Autonomy::Never => "never",
        }
    }

    /// Levels a user can choose in this version. `never` needs its one-time
    /// confirmation (R-1209) and comes after the guardrails have run in practice.
    pub fn selectable(self) -> bool {
        self != Autonomy::Never
    }

    /// One line for `instructions.md` (R-109): what the level means for the agent.
    pub fn instructions(self) -> &'static str {
        match self {
            Autonomy::Ask => "ask. Every gate waits for the user's approval; stop at each gate.",
            Autonomy::Important => {
                "important. xherd may approve design, tasks, review and commit gates by itself \
                 when nothing needs a human; spec and ship gates always wait. Still stop at \
                 every gate and mark anything doubtful with `--needs-human \"<why>\"`."
            }
            Autonomy::Never => {
                "never. xherd may approve every gate except pull-request rounds. Stop at every \
                 gate; mark anything doubtful with `--needs-human \"<why>\"`."
            }
        }
    }
}

impl std::fmt::Display for Autonomy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Autonomy {
    type Err = String;
    fn from_str(s: &str) -> Result<Autonomy, String> {
        match s {
            "ask" => Ok(Autonomy::Ask),
            "important" => Ok(Autonomy::Important),
            "never" => Ok(Autonomy::Never),
            _ => Err(format!("autonomy {s:?} must be ask, important or never")),
        }
    }
}

/// The table of spec §6.3.
pub fn may_auto_approve(level: Autonomy, class: Class) -> bool {
    match (level, class) {
        (_, Class::HumanOnly) | (Autonomy::Ask, _) => false,
        (Autonomy::Important, Class::Critical) => false,
        (Autonomy::Important, _) | (Autonomy::Never, _) => true,
    }
}

/// Run limit (R-120): `max` consecutive auto-approvals are allowed, the next
/// one waits for a human. 0 = no limit.
pub fn run_limit(in_a_row: u32, max: u32) -> Option<String> {
    (max > 0 && in_a_row >= max).then(|| {
        format!(
            "{in_a_row} gates in a row were approved automatically (maxAutoApprovalsInRow {max})"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Autonomy::*;
    use Class::*;

    #[test]
    fn table_of_the_spec() {
        let all = [Critical, Routine, Code, HumanOnly];
        assert!(all.iter().all(|c| !may_auto_approve(Ask, *c)));
        assert_eq!(
            all.map(|c| may_auto_approve(Important, c)),
            [false, true, true, false]
        );
        assert_eq!(
            all.map(|c| may_auto_approve(Never, c)),
            [true, true, true, false]
        );
    }

    #[test]
    fn human_only_never_approves() {
        for l in [Ask, Important, Never] {
            assert!(!may_auto_approve(l, HumanOnly));
        }
    }

    #[test]
    fn run_limit_counts_consecutive_approvals() {
        assert!(run_limit(9, 10).is_none());
        assert!(run_limit(10, 10).is_some());
        assert!(run_limit(500, 0).is_none(), "0 = no limit");
    }

    #[test]
    fn names_round_trip() {
        for l in [Ask, Important, Never] {
            assert_eq!(l.as_str().parse::<Autonomy>().unwrap(), l);
            assert_eq!(serde_json::to_value(l).unwrap(), l.as_str());
        }
        assert!("sometimes".parse::<Autonomy>().is_err());
        assert!(!Never.selectable() && Important.selectable() && Ask.selectable());
        assert_eq!(Autonomy::default(), Ask);
    }
}
