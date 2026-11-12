//! Tool-permission mode of an agent (R-002a, design §4.1): one vocabulary for
//! every runtime; each runtime plugin maps a mode to its own flags.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Asks before every tool use that needs permission; the strictest.
    Ask,
    /// Accepts file edits; commands that change things still ask.
    #[default]
    Edits,
    /// The runtime's own safety checks decide.
    Auto,
    /// Asks nothing: only in the container (T4).
    All,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 4] = [Self::Ask, Self::Edits, Self::Auto, Self::All];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Edits => "edits",
            Self::Auto => "auto",
            Self::All => "all",
        }
    }

    /// `all` asks nothing, so only the container may use it.
    pub fn allowed(self, sandbox: bool) -> bool {
        sandbox || self != Self::All
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = String;
    fn from_str(s: &str) -> Result<PermissionMode, String> {
        Self::ALL
            .into_iter()
            .find(|m| m.as_str() == s)
            .ok_or_else(|| format!("permission mode {s:?} must be ask, edits, auto or all"))
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
